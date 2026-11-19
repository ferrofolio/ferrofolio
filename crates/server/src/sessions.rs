// SPDX-License-Identifier: AGPL-3.0-only

//! API sessions, held by Rust (docs/rust-backend.md, "Sessions").
//!
//! Rust issues and checks the session tokens clients use, as Python's
//! `InMemoryApiSessionStore` did: 32 random bytes in URL-safe Base64, kept only as
//! their SHA-256 digest, valid until the next logout, or for a remembered session
//! until its remember period has passed.
//!
//! The Python backend keeps its own session guard. Rust passes a login on to it,
//! keeps the token Python answers with as the session's backend token and gives
//! the client a token of its own. A request with a valid session reaches Python
//! with that backend token in place of the client's; every other request reaches
//! Python unchanged, so Python's guard decides about it as it did before. Clients
//! therefore never hold a token Python knows.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, HeaderValue};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::pycompat;

/// Whether clients need a session (`finanze/args.py`).
pub const ENABLED_VARIABLE: &str = "FINANZE_API_SESSION";
/// How many days a remembered session lasts (`finanze/args.py`).
pub const REMEMBER_DAYS_VARIABLE: &str = "FINANZE_API_SESSION_REMEMBER_DAYS";

/// The wall-clock time remembered sessions expire by.
pub trait SessionClock: Send + Sync + fmt::Debug {
    fn now(&self) -> SystemTime;
}

/// The system's wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemSessionClock;

impl SessionClock for SystemSessionClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// The session settings, read from the environment the Python backend reads them
/// from, so that both always agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSettings {
    /// Whether clients need a session at all.
    pub enabled: bool,
    /// How long a remembered session lasts; without a period, as long as any other.
    pub remember_ttl: Option<Duration>,
}

impl Default for SessionSettings {
    fn default() -> Self {
        SessionSettings {
            enabled: true,
            remember_ttl: None,
        }
    }
}

impl SessionSettings {
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var_os(ENABLED_VARIABLE).as_deref(),
            std::env::var_os(REMEMBER_DAYS_VARIABLE).as_deref(),
        )
    }

    /// The settings for the raw values of both variables.
    pub fn from_values(enabled: Option<&OsStr>, remember_days: Option<&OsStr>) -> Self {
        SessionSettings {
            enabled: sessions_enabled(enabled),
            remember_ttl: remember_ttl(remember_days),
        }
    }
}

/// `FINANZE_API_SESSION`: on unless it is `0`, `false`, `no` or `off`, in any case
/// and with whitespace around. Python reads a value that is not valid Unicode with
/// escapes, which match none of these words.
fn sessions_enabled(raw: Option<&OsStr>) -> bool {
    let Some(text) = raw.and_then(OsStr::to_str) else {
        return true;
    };
    !matches!(
        pycompat::strip(text).to_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// `FINANZE_API_SESSION_REMEMBER_DAYS`: a positive number of days as `int()` reads
/// it; anything else means that remembered sessions do not expire. Python logs the
/// warning about an invalid value.
fn remember_ttl(raw: Option<&OsStr>) -> Option<Duration> {
    let text = pycompat::strip(raw?.to_str()?);
    if text.is_empty() {
        return None;
    }
    let days = pycompat::int(&Value::String(text.to_owned())).ok()?;
    let days = u64::try_from(days).ok().filter(|days| *days > 0)?;
    // Python fails to start with a period beyond `timedelta`'s range, and the
    // server stops with it.
    days.checked_mul(24 * 60 * 60).map(Duration::from_secs)
}

/// A session token could not be issued.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("the system has no random numbers: {0}")]
    Random(getrandom::Error),
    #[error("the backend answered with a token that is not a header value")]
    BackendToken,
    #[error("a logout ended the sessions while the login was running")]
    LoggedOut,
}

/// A login on its way to the backend.
#[derive(Debug, Clone, Copy)]
pub struct LoginAttempt {
    /// When it reached Rust; a remembered session lasts from then.
    started: SystemTime,
    /// The logouts that had ended the sessions by then.
    logouts: u64,
}

type TokenDigest = [u8; 32];

/// A session, under the digest of its token.
struct Session {
    /// When it ends, if it does.
    expires: Option<SystemTime>,
    /// The `Authorization` value with the backend token of its own login, which
    /// lasts until Python's logout (`LoginBody`). A session never uses the token
    /// of a later login: one that Python has ended stays ended there even while
    /// Rust has not yet learnt of the logout.
    backend: HeaderValue,
}

#[derive(Default)]
struct State {
    sessions: HashMap<TokenDigest, Session>,
    /// How many times a logout has ended the sessions.
    logouts: u64,
}

/// The sessions of the API. Clones share them.
#[derive(Clone)]
pub struct Sessions {
    settings: SessionSettings,
    clock: Arc<dyn SessionClock>,
    state: Arc<Mutex<State>>,
}

impl fmt::Debug for Sessions {
    // Tokens, and digests of them, stay out of logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sessions")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl Sessions {
    pub fn new(settings: SessionSettings, clock: Arc<dyn SessionClock>) -> Self {
        Sessions {
            settings,
            clock,
            state: Arc::default(),
        }
    }

    /// Whether clients need a session. Without sessions, Rust keeps no state and
    /// passes logins on unchanged.
    pub fn enabled(&self) -> bool {
        self.settings.enabled
    }

    /// Notes a login before it goes to the backend.
    pub fn login_attempt(&self) -> LoginAttempt {
        LoginAttempt {
            started: self.clock.now(),
            logouts: self.lock().logouts,
        }
    }

    /// Starts a session for a login the backend accepted with `backend_token` and
    /// returns the client's token. A remembered session expires one remember period
    /// after the login reached Rust.
    ///
    /// A logout that ended the sessions while the login was running may have ended
    /// the backend token too, so the login then starts no session.
    pub fn start(
        &self,
        attempt: LoginAttempt,
        remember: bool,
        backend_token: &str,
    ) -> Result<String, SessionError> {
        let mut authorization = HeaderValue::try_from(format!("Bearer {backend_token}"))
            .map_err(|_| SessionError::BackendToken)?;
        authorization.set_sensitive(true);
        let token = new_token()?;
        let expires = if remember {
            // A period beyond the clock's range never ends.
            self.settings
                .remember_ttl
                .and_then(|ttl| attempt.started.checked_add(ttl))
        } else {
            None
        };

        let mut state = self.lock();
        if state.logouts != attempt.logouts {
            return Err(SessionError::LoggedOut);
        }
        state.sessions.insert(
            digest(&token),
            Session {
                expires,
                backend: authorization,
            },
        );
        Ok(token)
    }

    /// Whether the request carries the token of a valid session.
    pub fn is_valid(&self, headers: &HeaderMap) -> bool {
        self.backend_authorization(headers).is_some()
    }

    /// The `Authorization` value the backend gets in place of the client's: the
    /// session's backend token, if the request carries the token of a valid
    /// session.
    pub fn backend_authorization(&self, headers: &HeaderMap) -> Option<HeaderValue> {
        if !self.settings.enabled {
            return None;
        }
        let token = bearer_token(headers)?;
        let token_digest = digest(&token);
        let now = self.clock.now();
        let mut state = self.lock();
        let session = state.sessions.get(&token_digest)?;
        if session.expires.is_some_and(|expires| now >= expires) {
            state.sessions.remove(&token_digest);
            return None;
        }
        Some(session.backend.clone())
    }

    /// Ends every session and forgets their backend tokens, as a logout does.
    pub fn revoke_all(&self) {
        let mut state = self.lock();
        state.sessions.clear();
        state.logouts = state.logouts.wrapping_add(1);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // The state stays consistent if a holder panicked: every change is a
        // single insert, removal or assignment.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The session token of a request as Python's session guard reads it: the
/// `Authorization` header, decoded as Latin-1, split at its first space, with a
/// scheme of `bearer` in any case and the rest without surrounding whitespace.
pub fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?;
    let text: String = value.as_bytes().iter().copied().map(char::from).collect();
    let (scheme, token) = text.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = pycompat::strip(token);
    (!token.is_empty()).then(|| token.to_owned())
}

/// The body of a login or signup as Rust forwards it, and what Rust reads from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginBody {
    /// The body the backend gets.
    pub forwarded: Bytes,
    /// Whether the client asked to be remembered, if Rust could read the body and
    /// so ask the backend for a token that lasts until the logout. A login whose
    /// body Rust cannot read starts no session.
    pub remember: Option<bool>,
}

impl LoginBody {
    /// Rust keeps the client's `remember` for its own session and asks the backend
    /// for a token that lasts until the logout, so that the backend token outlives
    /// every session: it appends `"remember": false` to a JSON object, and Python's
    /// `json` module keeps the last of repeated keys. The rest of the body reaches
    /// Python unchanged.
    ///
    /// A body that is not a JSON object Rust can read goes on unchanged, so that
    /// Python answers it as before: it refuses it, or reads JSON that serde_json
    /// does not, such as `NaN`, and may then issue a token that expires.
    ///
    /// With `arbitrary_precision`, serde_json reads an object whose first key is its
    /// private number token as a number, so such a `remember` can count as false
    /// where Python counts it as true: the session then lasts until the logout, as
    /// the client could ask for with `false` (docs/rust-backend.md, known deviations).
    pub fn new(body: Bytes, is_json: bool) -> Self {
        let unchanged = |body| LoginBody {
            forwarded: body,
            remember: None,
        };
        if !is_json {
            return unchanged(body);
        }
        let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(&body) else {
            return unchanged(body);
        };
        // serde_json allows only whitespace after the object.
        let Some(end) = body.iter().rposition(|byte| *byte == b'}') else {
            return unchanged(body);
        };
        let member: &[u8] = if fields.is_empty() {
            br#""remember":false"#
        } else {
            br#","remember":false"#
        };
        let (head, tail) = body.split_at(end);
        LoginBody {
            forwarded: Bytes::from([head, member, tail].concat()),
            remember: Some(fields.get("remember").is_some_and(pycompat::truthy)),
        }
    }
}

/// A new token: `secrets.token_urlsafe(32)` in Python.
fn new_token() -> Result<String, SessionError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(SessionError::Random)?;
    Ok(base64_url(&bytes))
}

fn digest(token: &str) -> TokenDigest {
    Sha256::digest(token.as_bytes()).into()
}

/// URL-safe Base64 without padding (RFC 4648, section 5).
fn base64_url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk
            .iter()
            .chain(std::iter::repeat(&0))
            .take(3)
            .fold(0u32, |group, byte| (group << 8) | u32::from(*byte));
        // n bytes give n + 1 characters.
        for position in 0..=chunk.len() {
            let shift = 18 - 6 * position;
            let index = usize::try_from((group >> shift) & 0x3F).unwrap_or_default();
            if let Some(character) = ALPHABET.get(index) {
                encoded.push(char::from(*character));
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    use super::*;

    #[derive(Debug)]
    struct TestClock(Mutex<SystemTime>);

    impl SessionClock for TestClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().unwrap()
        }
    }

    fn sessions(remember_ttl: Option<Duration>) -> (Sessions, Arc<TestClock>) {
        let clock = Arc::new(TestClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let settings = SessionSettings {
            enabled: true,
            remember_ttl,
        };
        (Sessions::new(settings, Arc::clone(&clock) as _), clock)
    }

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_bytes(format!("Bearer {token}").as_bytes()).unwrap(),
        );
        headers
    }

    #[test]
    fn encodes_url_safe_base64_without_padding() {
        assert_eq!(base64_url(b""), "");
        assert_eq!(base64_url(b"f"), "Zg");
        assert_eq!(base64_url(b"fo"), "Zm8");
        assert_eq!(base64_url(b"foo"), "Zm9v");
        assert_eq!(base64_url(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_url(&[0xFB, 0xFF, 0xBF]), "-_-_");
    }

    #[test]
    fn tokens_look_like_python_tokens() {
        let token = new_token().unwrap();
        assert_eq!(token.len(), 43);
        assert!(
            token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
        assert_ne!(token, new_token().unwrap());
    }

    #[test]
    fn reads_the_settings_like_python() {
        let enabled = |raw: &str| sessions_enabled(Some(OsStr::new(raw)));
        assert!(sessions_enabled(None));
        for off in ["0", "false", "No", " OFF\n", "\u{1C}false"] {
            assert!(!enabled(off), "{off:?}");
        }
        for on in ["", "1", "true", "yes", "disabled", "of f"] {
            assert!(enabled(on), "{on:?}");
        }
        assert!(sessions_enabled(Some(&OsString::from_vec(vec![0xFF]))));

        let ttl = |raw: &str| remember_ttl(Some(OsStr::new(raw)));
        let days = |days: u64| Some(Duration::from_secs(days * 86_400));
        assert_eq!(remember_ttl(None), None);
        assert_eq!(ttl("30"), days(30));
        assert_eq!(ttl(" 7 "), days(7));
        assert_eq!(ttl("1_0"), days(10));
        assert_eq!(ttl("+3"), days(3));
        assert_eq!(ttl("\u{663}"), days(3));
        for none in [
            "",
            "  ",
            "0",
            "-5",
            "1.5",
            "abc",
            "_1",
            "99999999999999999999",
        ] {
            assert_eq!(ttl(none), None, "{none:?}");
        }
    }

    #[test]
    fn reads_the_token_like_the_session_guard() {
        let header = |value: &[u8]| {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, HeaderValue::from_bytes(value).unwrap());
            bearer_token(&headers)
        };
        assert_eq!(header(b"Bearer abc"), Some("abc".to_owned()));
        assert_eq!(header(b"bEaReR abc"), Some("abc".to_owned()));
        assert_eq!(header(b"Bearer   abc \t"), Some("abc".to_owned()));
        // Latin-1 whitespace counts as whitespace in Python.
        assert_eq!(header(b"Bearer \xA0abc\x85"), Some("abc".to_owned()));
        assert_eq!(header(b"Bearer a b"), Some("a b".to_owned()));
        assert_eq!(header(b"Bearer"), None);
        assert_eq!(header(b"Bearer   "), None);
        assert_eq!(header(b"Bearer\tabc"), None);
        assert_eq!(header(b"Basic abc"), None);
        assert_eq!(bearer_token(&HeaderMap::new()), None);
    }

    #[test]
    fn a_session_lasts_until_the_logout() {
        let (sessions, clock) = sessions(Some(Duration::from_secs(60)));
        let token = sessions
            .start(sessions.login_attempt(), false, "backend")
            .unwrap();
        *clock.0.lock().unwrap() += Duration::from_secs(365 * 86_400);
        assert_eq!(
            sessions.backend_authorization(&bearer(&token)).unwrap(),
            "Bearer backend"
        );
        assert!(!sessions.is_valid(&bearer("unknown")));
        assert!(!sessions.is_valid(&HeaderMap::new()));

        sessions.revoke_all();
        assert!(!sessions.is_valid(&bearer(&token)));
    }

    #[test]
    fn a_remembered_session_expires_after_its_period() {
        let (sessions, clock) = sessions(Some(Duration::from_secs(60)));
        let started = clock.now();
        let attempt = sessions.login_attempt();
        *clock.0.lock().unwrap() += Duration::from_secs(1);
        let token = sessions.start(attempt, true, "backend").unwrap();
        *clock.0.lock().unwrap() = started + Duration::from_secs(59);
        assert!(sessions.is_valid(&bearer(&token)));
        *clock.0.lock().unwrap() = started + Duration::from_secs(60);
        assert!(!sessions.is_valid(&bearer(&token)));
        // An expired session is gone, also when the clock goes back.
        *clock.0.lock().unwrap() = started;
        assert!(!sessions.is_valid(&bearer(&token)));
    }

    #[test]
    fn without_a_period_a_remembered_session_does_not_expire() {
        let (sessions, clock) = sessions(None);
        let token = sessions
            .start(sessions.login_attempt(), true, "backend")
            .unwrap();
        *clock.0.lock().unwrap() += Duration::from_secs(3650 * 86_400);
        assert!(sessions.is_valid(&bearer(&token)));
    }

    #[test]
    fn every_session_reaches_the_backend_with_its_own_backend_token() {
        let (sessions, _) = sessions(None);
        let first = sessions
            .start(sessions.login_attempt(), false, "first")
            .unwrap();
        let second = sessions
            .start(sessions.login_attempt(), false, "second")
            .unwrap();
        assert_eq!(
            sessions.backend_authorization(&bearer(&first)).unwrap(),
            "Bearer first"
        );
        assert_eq!(
            sessions.backend_authorization(&bearer(&second)).unwrap(),
            "Bearer second"
        );
    }

    #[test]
    fn refuses_a_backend_token_that_is_not_a_header_value() {
        let (sessions, _) = sessions(None);
        assert!(matches!(
            sessions.start(sessions.login_attempt(), false, "a\nb"),
            Err(SessionError::BackendToken)
        ));
    }

    #[test]
    fn disabled_sessions_translate_nothing() {
        let clock = Arc::new(TestClock(Mutex::new(SystemTime::UNIX_EPOCH)));
        let settings = SessionSettings {
            enabled: false,
            remember_ttl: None,
        };
        let sessions = Sessions::new(settings, clock);
        let token = sessions
            .start(sessions.login_attempt(), false, "backend")
            .unwrap();
        assert!(!sessions.is_valid(&bearer(&token)));
    }

    #[test]
    fn a_login_that_a_logout_overtook_starts_no_session() {
        let (sessions, _) = sessions(None);
        let earlier = sessions
            .start(sessions.login_attempt(), false, "first")
            .unwrap();
        let attempt = sessions.login_attempt();
        sessions.revoke_all();
        assert!(matches!(
            sessions.start(attempt, false, "second"),
            Err(SessionError::LoggedOut)
        ));
        assert!(!sessions.is_valid(&bearer(&earlier)));
        let later = sessions
            .start(sessions.login_attempt(), false, "third")
            .unwrap();
        assert_eq!(
            sessions.backend_authorization(&bearer(&later)).unwrap(),
            "Bearer third"
        );
    }

    #[test]
    fn asks_the_backend_for_a_lasting_token() {
        let body = |raw: &str| LoginBody::new(Bytes::from(raw.to_owned()), true);
        let login = body(r#"{"username":"u","password":"p","remember":true}"#);
        assert_eq!(
            login.forwarded,
            r#"{"username":"u","password":"p","remember":true,"remember":false}"#
        );
        assert_eq!(login.remember, Some(true));

        let login = body("{ \"username\": \"u\", \"remember\": 0 } \n");
        assert_eq!(
            login.forwarded,
            "{ \"username\": \"u\", \"remember\": 0 ,\"remember\":false} \n"
        );
        assert_eq!(login.remember, Some(false));

        let login = body(r#"{"remember":"no","remember":null}"#);
        assert_eq!(login.remember, Some(false));
        let login = body(r#"{"remember":"no"}"#);
        assert_eq!(login.remember, Some(true));

        assert_eq!(body("{}").forwarded, r#"{"remember":false}"#);
        assert_eq!(body("{ }").forwarded, r#"{ "remember":false}"#);
    }

    #[test]
    fn passes_bodies_it_cannot_read_on_unchanged() {
        for raw in [
            "",
            "[]",
            "\"text\"",
            r#"{"username":"u","remember":NaN}"#,
            r#"{"a":1"#,
            r#"{"a":1}}"#,
        ] {
            let login = LoginBody::new(Bytes::from(raw), true);
            assert_eq!(login.forwarded, raw);
            assert_eq!(login.remember, None, "{raw}");
        }
        let raw = r#"{"username":"u"}"#;
        let login = LoginBody::new(Bytes::from(raw), false);
        assert_eq!(login.forwarded, raw);
        assert_eq!(login.remember, None);
    }
}
