// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/v1/status`, ported from
//! `finanze/infrastructure/controller/routes/get_status.py` and
//! `finanze/application/use_cases/get_status.py`: whether the data is unlocked and
//! for whom, the server's details and options, and the feature flags.

use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header::{CONTENT_TYPE, HOST, ORIGIN};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value, json};

use crate::app::AppState;
use crate::config::{Cli, LogLevel};
use crate::error::ApiError;
use crate::gateway::forward;
use crate::proxy::remove_hop_by_hop;
use crate::routes::apply_cors;
use crate::user_data::{DataDir, LastLogged, default_data_dir};
use crate::{pycompat, pypath};

pub const PATH: &str = "/api/v1/status";

/// The version Python reads from `finanze/version.py`; the release bump changes
/// both together (`.bumpversion.toml`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");
/// The address the gateway connects to the backend from.
const GATEWAY_ADDRESS: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// The options the backend runs with, as `ServerDetailsAdapter` reports them.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub data_dir: PathBuf,
    /// The public port; the backend's own is internal.
    pub port: u16,
    pub log_level: LogLevel,
    pub log_dir: PathBuf,
    pub log_file_level: Option<LogLevel>,
    pub third_party_log_level: LogLevel,
}

impl ServerOptions {
    /// The options the gateway passes on to the backend, with Python's defaults for
    /// those it leaves out.
    pub fn from_cli(cli: &Cli, data_dir: PathBuf) -> Self {
        ServerOptions {
            data_dir,
            port: cli.port,
            log_level: cli.log_level,
            // `Path(appdirs.user_data_dir("Finanze", False)) / "logs"`, whatever the
            // data directory.
            log_dir: cli
                .log_dir
                .clone()
                .unwrap_or_else(|| default_data_dir().join("logs")),
            log_file_level: cli.log_file_level,
            third_party_log_level: cli.third_party_log_level,
        }
    }
}

/// What the status reports besides the lock state, fixed at startup as in Python.
#[derive(Debug, Clone)]
pub struct StatusConfig {
    options: ServerOptions,
    platform_type: Option<&'static str>,
    platform_version: Option<String>,
    features: Map<String, Value>,
    /// `FINANZE_PUBLIC_URL`, without surrounding whitespace and trailing slashes.
    public_url: Option<String>,
    /// Whether uvicorn takes the request scheme from the gateway's proxy headers.
    trusts_gateway: bool,
}

impl StatusConfig {
    /// The status the backend would report, from the process environment. `None`
    /// when `FEATURE_FLAG_URL` is set: Python then loads the feature flags from
    /// that URL at startup and answers the status itself.
    pub fn from_env(options: ServerOptions) -> Option<Self> {
        Self::new(
            options,
            std::env::vars_os(),
            platform_type(),
            platform_version(),
        )
    }

    /// The status for these options, environment and platform.
    pub fn new(
        options: ServerOptions,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
        platform_type: Option<&'static str>,
        platform_version: Option<String>,
    ) -> Option<Self> {
        let mut features = Map::new();
        // `DEFAULT_FEATURE_FLAGS` of `env_feature_flag_adapter.py`.
        features.insert("TR_V2_AUTH_API".to_owned(), Value::from("ON"));
        let mut public_url = None;
        let mut forwarded_allow_ips = None;
        for (key, value) in environment {
            let text = || value.to_string_lossy().into_owned();
            match key.as_bytes() {
                // Python reads a remote source only for a non-empty URL.
                b"FEATURE_FLAG_URL" if !value.is_empty() => return None,
                b"FINANZE_PUBLIC_URL" => {
                    let url = pycompat::strip(&text()).trim_end_matches('/').to_owned();
                    public_url = Some(url).filter(|url| !url.is_empty());
                }
                b"FORWARDED_ALLOW_IPS" => forwarded_allow_ips = Some(text()),
                _ => {
                    if let Some(name) = key.as_bytes().strip_prefix(b"FF_") {
                        let name = String::from_utf8_lossy(name).into_owned();
                        features.insert(name, Value::String(text()));
                    }
                }
            }
        }
        Some(StatusConfig {
            options,
            platform_type,
            platform_version,
            features,
            public_url,
            trusts_gateway: trusts_gateway(forwarded_allow_ips.as_deref()),
        })
    }

    /// The `options` object. Python resolves the directories on every request; a
    /// relative one needs the working directory.
    fn options(&self, cwd: Option<&[u8]>) -> Option<Value> {
        let resolve = |path: &PathBuf| {
            pypath::resolve(path.as_os_str().as_bytes(), cwd)
                .map(|path| String::from_utf8_lossy(&path).into_owned())
        };
        Some(json!({
            "dataDir": resolve(&self.options.data_dir)?,
            "port": self.options.port,
            "logLevel": self.options.log_level.as_arg(),
            "logDir": resolve(&self.options.log_dir)?,
            "logFileLevel": self.options.log_file_level.map(LogLevel::as_arg),
            "thirdPartyLogLevel": self.options.third_party_log_level.as_arg(),
        }))
    }

    /// `public_base_url()` of `public_url.py` for a request the gateway would
    /// forward: `FINANZE_PUBLIC_URL`, or the URL the request's headers give.
    fn public_url(&self, headers: &HeaderMap, uri: &Uri) -> Option<String> {
        if let Some(url) = &self.public_url {
            return Some(url.clone());
        }
        // The backend gets the headers without the hop-by-hop ones, and, from a
        // request in absolute form without a `Host`, the URI's authority as `Host`.
        let mut headers = headers.clone();
        remove_hop_by_hop(&mut headers);
        let host = match headers.get(X_FORWARDED_HOST).and_then(first_item) {
            Some(host) => host,
            None => match headers.get(HOST) {
                Some(host) => latin1(host),
                None => uri.authority()?.as_str().to_owned(),
            },
        };
        if host.is_empty() {
            return None;
        }
        let forwarded = headers
            .get(X_FORWARDED_PROTO)
            .and_then(first_item)
            .map(|scheme| scheme.to_lowercase())
            .filter(|scheme| scheme == "http" || scheme == "https");
        let scheme = forwarded.unwrap_or_else(|| self.backend_scheme(&headers).to_owned());
        Some(format!("{scheme}://{host}"))
    }

    /// The request scheme the backend sees behind the gateway: uvicorn takes the
    /// last `X-Forwarded-Proto` from an address it trusts. Only `https` makes a
    /// difference; `ws`, `wss` and plain HTTP all give `http`.
    fn backend_scheme(&self, headers: &HeaderMap) -> &'static str {
        let last = headers
            .get_all(X_FORWARDED_PROTO)
            .iter()
            .next_back()
            .map(latin1);
        if self.trusts_gateway && last.as_deref().map(pycompat::strip) == Some("https") {
            "https"
        } else {
            "http"
        }
    }
}

/// `GET /api/v1/status`. The backend answers it without a status configuration,
/// and without API sessions, as only the backend knows then whether the data is
/// unlocked (`Sessions::status`).
pub async fn status(State(state): State<AppState>, request: Request) -> Response {
    let Some(config) = state.status.clone() else {
        return forward(State(state), request).await;
    };
    let Some((unlocked, user)) = state.sessions.status(request.headers()) else {
        return forward(State(state), request).await;
    };
    let has_origin = request.headers().contains_key(ORIGIN);
    let public_url = config.public_url(request.headers(), request.uri());
    let data_dir = state.data_dir.clone();
    // Resolving the directories and reading `profiles.json` touch the file system.
    let read = tokio::task::spawn_blocking(move || read_files(&config, data_dir.as_ref())).await;
    let mut response = match read {
        Ok(Some((config, options, last_logged))) => {
            let user = user
                .map(|user| json!({"id": user.id, "username": user.username, "path": user.path}));
            let body = json!({
                "status": if unlocked { "UNLOCKED" } else { "LOCKED" },
                "server": {
                    "version": VERSION,
                    "platform_type": config.platform_type,
                    "platform_version": config.platform_version.clone(),
                    "publicUrl": public_url,
                    "options": options,
                },
                "features": config.features.clone(),
                "user": user,
                "lastLogged": last_logged.as_ref().map(|last| last.username.clone()),
                "pendingRegister": last_logged.map_or(Value::Bool(false), |last| last.guest),
                "apiSession": state.sessions.enabled(),
            });
            // `jsonify` in Quart: compact JSON and a newline.
            let headers = [(CONTENT_TYPE, HeaderValue::from_static("application/json"))];
            (StatusCode::OK, headers, format!("{body}\n")).into_response()
        }
        Ok(None) => {
            ApiError::Unexpected("The working directory is unavailable".to_owned()).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "reading the status did not complete");
            ApiError::Unexpected("Status failed".to_owned()).into_response()
        }
    };
    apply_cors(response.headers_mut(), has_origin);
    response
}

type Files = (Arc<StatusConfig>, Value, Option<LastLogged>);

/// The options with resolved directories and the last user, read from the file
/// system; `None` if a relative directory needs a working directory that is gone.
fn read_files(config: &Arc<StatusConfig>, data_dir: Option<&DataDir>) -> Option<Files> {
    let cwd = std::env::current_dir()
        .ok()
        .map(|directory| directory.into_os_string().into_vec());
    let options = config.options(cwd.as_deref())?;
    let last_logged = data_dir.and_then(DataDir::last_logged);
    Some((Arc::clone(config), options, last_logged))
}

/// A header value as Quart reads it, in Latin-1.
fn latin1(value: &HeaderValue) -> String {
    value.as_bytes().iter().copied().map(char::from).collect()
}

/// `_first()` of `public_url.py`: the first comma-separated item, stripped, if it
/// is not empty.
fn first_item(value: &HeaderValue) -> Option<String> {
    let text = latin1(value);
    let item = pycompat::strip(text.split(',').next().unwrap_or_default());
    (!item.is_empty()).then(|| item.to_owned())
}

/// Whether uvicorn trusts the proxy headers of the gateway, which connects from
/// 127.0.0.1: `FORWARDED_ALLOW_IPS`, by default `127.0.0.1,::1`, is `*` or names
/// that address or an IPv4 network that contains it, as `ipaddress` reads them.
pub fn trusts_gateway(forwarded_allow_ips: Option<&str>) -> bool {
    let Some(value) = forwarded_allow_ips else {
        return true;
    };
    if value == "*" {
        return true;
    }
    value.split(',').map(pycompat::strip).any(|host| {
        if host.contains('/') {
            ipv4_network(host)
                .is_some_and(|(network, mask)| u32::from(GATEWAY_ADDRESS) & mask == network)
        } else {
            host.parse::<Ipv4Addr>()
                .is_ok_and(|address| address == GATEWAY_ADDRESS)
        }
    })
}

/// `ipaddress.IPv4Network(text)`, strict: the network address and the mask, with a
/// prefix length or a netmask or hostmask in dotted form, and no host bits set.
fn ipv4_network(text: &str) -> Option<(u32, u32)> {
    let (address, mask) = text.split_once('/')?;
    if mask.contains('/') {
        return None;
    }
    let address = u32::from(address.parse::<Ipv4Addr>().ok()?);
    let prefix = if !mask.is_empty() && mask.bytes().all(|byte| byte.is_ascii_digit()) {
        mask.parse::<u32>().ok().filter(|prefix| *prefix <= 32)?
    } else {
        let mask = u32::from(mask.parse::<Ipv4Addr>().ok()?);
        prefix_of(mask).or_else(|| prefix_of(!mask))?
    };
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    (address & mask == address).then_some((address, mask))
}

/// The prefix length of a netmask: ones, then only zeros.
fn prefix_of(mask: u32) -> Option<u32> {
    let prefix = mask.leading_ones();
    (mask.checked_shl(prefix).unwrap_or(0) == 0).then_some(prefix)
}

/// `detect_os()` of `server_details_adapter.py`.
pub fn platform_type() -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        Some("LINUX")
    } else if cfg!(target_os = "macos") {
        Some("MACOS")
    } else {
        None
    }
}

/// `detect_os_version()` of `server_details_adapter.py`: the kernel release, as
/// `platform.release()` reports it, and on macOS the product version.
pub fn platform_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    let version = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
        .ok()
        .and_then(|plist| product_version(&plist));
    #[cfg(not(target_os = "macos"))]
    let version = nix::sys::utsname::uname()
        .ok()
        .map(|name| name.release().to_string_lossy().into_owned());
    version.filter(|version| !version.is_empty())
}

/// The `ProductVersion` of macOS's `SystemVersion.plist`, which
/// `platform.mac_ver()` reads.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
fn product_version(plist: &str) -> Option<String> {
    let (_, after_key) = plist.split_once("<key>ProductVersion</key>")?;
    let value = after_key.trim_start().strip_prefix("<string>")?;
    let (version, _) = value.split_once("</string>")?;
    Some(version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_settings_from_the_environment() {
        let options = ServerOptions {
            data_dir: PathBuf::from("/data"),
            port: 7592,
            log_level: LogLevel::Info,
            log_dir: PathBuf::from("/data/logs"),
            log_file_level: None,
            third_party_log_level: LogLevel::Error,
        };
        let config = |pairs: &[(&str, &str)]| {
            let environment = pairs
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value)));
            StatusConfig::new(options.clone(), environment, Some("LINUX"), None)
        };
        let defaults = config(&[]).unwrap();
        assert_eq!(
            defaults.features,
            json!({"TR_V2_AUTH_API": "ON"})
                .as_object()
                .cloned()
                .unwrap()
        );
        assert_eq!(defaults.public_url, None);
        assert!(defaults.trusts_gateway);

        let configured = config(&[
            ("FF_TR_V2_AUTH_API", "OFF"),
            ("FF_NEW", "x"),
            ("FINANZE_PUBLIC_URL", " https://example.org// \n"),
            ("FORWARDED_ALLOW_IPS", "10.0.0.1"),
            ("FEATURE_FLAG_URL", ""),
        ])
        .unwrap();
        assert_eq!(
            Value::Object(configured.features),
            json!({"TR_V2_AUTH_API": "OFF", "NEW": "x"})
        );
        assert_eq!(
            configured.public_url.as_deref(),
            Some("https://example.org")
        );
        assert!(!configured.trusts_gateway);
        assert_eq!(
            config(&[("FINANZE_PUBLIC_URL", " / ")]).unwrap().public_url,
            None
        );

        assert!(config(&[("FEATURE_FLAG_URL", "https://flags.example.org")]).is_none());
    }

    #[test]
    fn reads_the_product_version_of_macos() {
        let plist = "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t\
                     <key>ProductVersion</key>\n\t<string>15.1</string>\n</dict>";
        assert_eq!(product_version(plist).as_deref(), Some("15.1"));
        assert_eq!(product_version("<dict></dict>"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reports_the_kernel_release_as_python_does() {
        // `platform.release()` is `os.uname().release`, which `uname -r` prints.
        let output = std::process::Command::new("uname")
            .arg("-r")
            .output()
            .unwrap();
        let release = String::from_utf8(output.stdout).unwrap();
        assert_eq!(platform_version().as_deref(), Some(release.trim_end()));
        assert_eq!(platform_type(), Some("LINUX"));
    }

    #[test]
    fn reads_netmasks_like_ipaddress() {
        let network = |text: &str| ipv4_network(text);
        assert_eq!(network("127.0.0.0/8"), Some((0x7F00_0000, 0xFF00_0000)));
        assert_eq!(network("127.0.0.0/08"), Some((0x7F00_0000, 0xFF00_0000)));
        assert_eq!(
            network("127.0.0.0/255.0.0.0"),
            Some((0x7F00_0000, 0xFF00_0000))
        );
        assert_eq!(
            network("127.0.0.0/0.255.255.255"),
            Some((0x7F00_0000, 0xFF00_0000))
        );
        assert_eq!(network("0.0.0.0/0"), Some((0, 0)));
        assert_eq!(network("0.0.0.0/0.0.0.0"), Some((0, 0)));
        assert_eq!(network("127.0.0.1/32"), Some((0x7F00_0001, u32::MAX)));
        for invalid in [
            "127.0.0.1/8",
            "127.0.0.0/33",
            "127.0.0.0/",
            "127.0.0.0/8/8",
            "127.0.0.0/255.0.255.0",
            "127.0.0/8",
            "::1/128",
        ] {
            assert_eq!(network(invalid), None, "{invalid}");
        }
    }
}
