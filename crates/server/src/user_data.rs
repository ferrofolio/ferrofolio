// SPDX-License-Identifier: AGPL-3.0-only

//! The data directory and the users in it, laid out as `finanze/args.py` and
//! `finanze/infrastructure/user_files/user_data_manager.py` lay them out: a
//! `profiles.json` with every user's id and name, and each user's encrypted
//! database at `profiles/<id>/data.db`.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use infra_db::{Database, OpenError};
use serde::Deserialize;
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::sessions::base64_url;
use crate::{pycompat, pypath};

use crate::sessions::Credentials;

/// The name of the application directory Python uses.
const APP_NAME: &str = "Finanze";

/// The data directory Python uses without `--data-dir`:
/// `appdirs.user_data_dir("Finanze", False)`.
pub fn default_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::home_dir().map(OsString::from));
    data_dir_for(
        std::env::var_os("XDG_DATA_HOME"),
        home,
        cfg!(target_os = "macos"),
    )
}

/// `appdirs.user_data_dir` for the given environment: on macOS in
/// `~/Library/Application Support`, elsewhere in `XDG_DATA_HOME`, which counts as
/// set even when empty, or `~/.local/share`.
fn data_dir_for(xdg_data_home: Option<OsString>, home: Option<OsString>, macos: bool) -> PathBuf {
    let base = match (macos, xdg_data_home) {
        (true, _) => expand_home(home, "Library/Application Support"),
        (false, Some(xdg_data_home)) => PathBuf::from(xdg_data_home),
        (false, None) => expand_home(home, ".local/share"),
    };
    base.join(APP_NAME)
}

/// `os.path.expanduser("~/<rest>")`: the home directory without trailing slashes,
/// then `/<rest>`; without a home directory the path stays as it is.
fn expand_home(home: Option<OsString>, rest: &str) -> PathBuf {
    let Some(home) = home else {
        return PathBuf::from(format!("~/{rest}"));
    };
    let mut bytes = home.into_vec();
    while bytes.last() == Some(&b'/') {
        bytes.pop();
    }
    bytes.push(b'/');
    bytes.extend_from_slice(OsStr::new(rest).as_bytes());
    PathBuf::from(OsString::from_vec(bytes))
}

#[derive(Deserialize)]
struct Profiles {
    profiles: Vec<Value>,
}

/// A user as the status reports them (`finanze/domain/user.py`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// `User.hashed_id()`.
    pub id: String,
    pub username: String,
    /// `str(user.path.absolute())`: the user's directory.
    pub path: String,
}

/// The user who logged in or signed up last, as `profiles.json` records them.
#[derive(Debug, Clone, PartialEq)]
pub struct LastLogged {
    /// The profile's `name`.
    pub username: Value,
    /// The profile's `guest`, `false` without one: the user still has to register.
    pub guest: Value,
}

/// The profile of a user, found by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The id in the form `str(uuid.UUID(...))` writes.
    id: String,
    username: String,
}

/// The data directory of the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir(PathBuf);

impl DataDir {
    pub fn new(path: PathBuf) -> Self {
        DataDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The profile of the user with this name: the first profile with the name, as
    /// Python's `get_user` finds it. Python keeps `profiles.json` in memory and
    /// writes it on every change, so the file is current once Python has answered
    /// a login or signup.
    pub fn profile(&self, username: &str) -> Option<Profile> {
        let text = std::fs::read(self.0.join("profiles.json")).ok()?;
        let profiles: Profiles = serde_json::from_slice(&text).ok()?;
        let profile = profiles
            .profiles
            .iter()
            .find(|profile| profile.get("name").and_then(Value::as_str) == Some(username))?;
        let id = canonical_uuid(profile.get("id")?.as_str()?)?;
        Some(Profile {
            id,
            username: username.to_owned(),
        })
    }

    /// The database of the user with this name (`profile`).
    pub fn database_path(&self, username: &str) -> Option<PathBuf> {
        self.profile(username)
            .map(|profile| self.database_of(&profile))
    }

    /// The database of a profile, `profiles/<id>/data.db`.
    pub fn database_of(&self, profile: &Profile) -> PathBuf {
        self.0.join("profiles").join(&profile.id).join("data.db")
    }

    /// A profile's user as the status reports them. Python builds the user's
    /// directory as `os.path.join(<data dir>, "profiles", <id>)` and makes it
    /// absolute with the working directory, `cwd`, which a relative data directory
    /// needs.
    pub fn user_of(&self, profile: &Profile, cwd: Option<&[u8]>) -> Option<User> {
        let base = pypath::normalize(self.0.as_os_str().as_bytes());
        let directory = pypath::join(&pypath::join(&base, b"profiles"), profile.id.as_bytes());
        let path = pypath::absolute(&directory, cwd)?;
        Some(User {
            id: hashed_id(&profile.id)?,
            username: profile.username.clone(),
            path: String::from_utf8_lossy(&path).into_owned(),
        })
    }

    /// The user who logged in or signed up last, as Python's `get_last_user` finds
    /// them: the profile whose `id` is the `last_logged` of `profiles.json`. Python
    /// reads the file at startup and fails on a profile it cannot read; Rust reads
    /// the file, which Python writes on every change, and takes what it finds.
    pub fn last_logged(&self) -> Option<LastLogged> {
        let text = std::fs::read(self.0.join("profiles.json")).ok()?;
        let data: Value = serde_json::from_slice(&text).ok()?;
        let last = data.get("last_logged")?;
        if !pycompat::truthy(last) {
            return None;
        }
        let profile = data
            .get("profiles")?
            .as_array()?
            .iter()
            .find(|profile| profile.get("id") == Some(last))?;
        Some(LastLogged {
            username: profile.get("name").cloned().unwrap_or(Value::Null),
            guest: profile.get("guest").cloned().unwrap_or(Value::Bool(false)),
        })
    }
}

/// `User.hashed_id()`: the first 10 bytes of the SHA-1 digest of the id's 16 bytes,
/// in URL-safe Base64 without padding, with `-` and `_` replaced by `a` and `z`.
fn hashed_id(canonical: &str) -> Option<String> {
    let digits: Vec<u8> = canonical.bytes().filter(|byte| *byte != b'-').collect();
    let bytes = digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let digest = Sha1::digest(&bytes);
    let encoded = base64_url(digest.get(..10)?);
    Some(encoded.replace('-', "a").replace('_', "z"))
}

/// Why a user's database could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum UnlockError {
    #[error("the data directory has no profile with the login's username")]
    NoProfile,
    #[error("the database file cannot be read: {0}")]
    File(std::io::Error),
    #[error("a request is replacing or rekeying the database")]
    Replacing,
    #[error("no login has unlocked the database")]
    Locked,
    #[error(transparent)]
    Open(#[from] OpenError),
}

/// The file a database was opened from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    device: u64,
    inode: u64,
}

fn file_id(path: &Path) -> Result<FileId, UnlockError> {
    let metadata = std::fs::metadata(path).map_err(UnlockError::File)?;
    Ok(FileId {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

/// A user's database, opened read-only with the password of a login.
///
/// It keeps the credentials, as Python's `DBManager` keeps the password while the
/// data is unlocked, so that it can open the database again: `current` does so
/// after `close` and when the file has been replaced. Opening derives the key, so
/// it runs on the blocking pool. `Sessions` decides when a connection may be open
/// (`Sessions::with_database`).
pub struct UserDatabase {
    connection: Option<(Database, FileId)>,
    path: PathBuf,
    credentials: Credentials,
}

impl fmt::Debug for UserDatabase {
    // The credentials stay out of logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UserDatabase")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl UserDatabase {
    /// Opens the database of the user the credentials name.
    pub fn open(data_dir: &DataDir, credentials: Credentials) -> Result<Self, UnlockError> {
        let path = data_dir
            .database_path(&credentials.username)
            .ok_or(UnlockError::NoProfile)?;
        Self::open_at(path, credentials)
    }

    /// Opens the database at `path` with the credentials.
    pub fn open_at(path: PathBuf, credentials: Credentials) -> Result<Self, UnlockError> {
        let connection = connect(&path, &credentials.password)?;
        Ok(UserDatabase {
            connection: Some(connection),
            path,
            credentials,
        })
    }

    /// Closes the connection; `current` opens it again.
    pub fn close(&mut self) {
        self.connection = None;
    }

    /// The database, opened again first if it was closed or its file has been
    /// replaced since.
    pub fn current(&mut self) -> Result<&Database, UnlockError> {
        let file = file_id(&self.path)?;
        if self
            .connection
            .as_ref()
            .is_none_or(|(_, opened)| *opened != file)
        {
            // Dropped first: the old connection must not outlive the new file's.
            self.connection = None;
            self.connection = Some(connect(&self.path, &self.credentials.password)?);
        }
        match &self.connection {
            Some((database, _)) => Ok(database),
            None => Err(UnlockError::Locked),
        }
    }
}

/// Opens the database at `path` and returns it with the file it was opened from.
fn connect(path: &Path, password: &str) -> Result<(Database, FileId), UnlockError> {
    // Taken before opening: a file replaced in between is opened again by the next
    // `current`.
    let file = file_id(path)?;
    let database = Database::open_read_only(path, password)?;
    Ok((database, file))
}

/// `str(uuid.UUID(text))` for the spellings Python writes and reads: 32 hex digits,
/// with or without hyphens, braces or a `urn:uuid:` prefix, in either case; the
/// result has lowercase digits in groups of 8, 4, 4, 4 and 12.
fn canonical_uuid(text: &str) -> Option<String> {
    let digits: String = text
        .replace("urn:", "")
        .replace("uuid:", "")
        .trim_matches(|character| character == '{' || character == '}')
        .replace('-', "")
        .to_ascii_lowercase();
    if digits.len() != 32 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let group = |range: std::ops::Range<usize>| digits.get(range);
    Some(format!(
        "{}-{}-{}-{}-{}",
        group(0..8)?,
        group(8..12)?,
        group(12..16)?,
        group(16..20)?,
        group(20..32)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_default_data_directory_like_appdirs() {
        let some = |text: &str| Some(OsString::from(text));
        let dir = |xdg, home, macos| data_dir_for(xdg, home, macos);
        assert_eq!(
            dir(None, some("/home/anna"), false),
            PathBuf::from("/home/anna/.local/share/Finanze")
        );
        assert_eq!(
            dir(None, some("/home/anna/"), false),
            PathBuf::from("/home/anna/.local/share/Finanze")
        );
        assert_eq!(
            dir(some("/data/xdg"), some("/home/anna"), false),
            PathBuf::from("/data/xdg/Finanze")
        );
        // An empty XDG_DATA_HOME makes the directory relative, as in appdirs.
        assert_eq!(
            dir(some(""), some("/home/anna"), false),
            PathBuf::from("Finanze")
        );
        assert_eq!(
            dir(None, some("/"), false),
            PathBuf::from("/.local/share/Finanze")
        );
        assert_eq!(
            dir(None, None, false),
            PathBuf::from("~/.local/share/Finanze")
        );
        assert_eq!(
            dir(some("/data/xdg"), some("/Users/anna"), true),
            PathBuf::from("/Users/anna/Library/Application Support/Finanze")
        );
    }

    #[test]
    fn writes_ids_like_python() {
        let canonical = Some("0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b".to_owned());
        for spelling in [
            "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b",
            "0D3E7C1A-5B2F-4C8E-9A1B-2C3D4E5F6A7B",
            "0d3e7c1a5b2f4c8e9a1b2c3d4e5f6a7b",
            "{0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b}",
            "urn:uuid:0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b",
        ] {
            assert_eq!(canonical_uuid(spelling), canonical, "{spelling}");
        }
        for invalid in [
            "",
            "0d3e7c1a",
            "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7",
            "../../etc/passwd",
        ] {
            assert_eq!(canonical_uuid(invalid), None, "{invalid}");
        }
        assert_eq!(canonical_uuid("0g3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b"), None);
    }

    #[test]
    fn finds_the_database_of_the_first_profile_with_the_name() {
        let directory =
            std::env::temp_dir().join(format!("ferrofolio-user-data-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("profiles.json"),
            r#"{"last_logged": null, "profiles": [
                {"id": "11111111-1111-1111-1111-111111111111", "name": "other"},
                {"id": "0D3E7C1A5B2F4C8E9A1B2C3D4E5F6A7B", "name": "anna"},
                {"id": "22222222-2222-2222-2222-222222222222", "name": "anna"},
                {"id": "../escape", "name": "bad"},
                {"name": "no-id"}
            ]}"#,
        )
        .unwrap();
        let data_dir = DataDir::new(directory.clone());
        assert_eq!(
            data_dir.database_path("anna"),
            Some(directory.join("profiles/0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b/data.db"))
        );
        for missing in ["nobody", "bad", "no-id", "Anna"] {
            assert_eq!(data_dir.database_path(missing), None, "{missing}");
        }
        std::fs::write(directory.join("profiles.json"), "not json").unwrap();
        assert_eq!(data_dir.database_path("anna"), None);
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(data_dir.database_path("anna"), None);
    }

    #[test]
    fn hashes_ids_like_python() {
        assert_eq!(
            hashed_id("0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b").as_deref(),
            Some("pzYaT1SJsVifVQ")
        );
        assert_eq!(
            hashed_id("00000000-0000-0000-0000-00000000000f").as_deref(),
            Some("LDWeSTzmV8uBag")
        );
        assert_eq!(
            hashed_id("00000000-0000-0000-0000-000000000000").as_deref(),
            Some("4SnyfFEDvFzESw")
        );
    }

    #[test]
    fn reports_users_with_the_directory_python_builds() {
        let profile = Profile {
            id: "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b".to_owned(),
            username: "anna".to_owned(),
        };
        let user = |data_dir: &str, cwd: Option<&[u8]>| {
            DataDir::new(PathBuf::from(data_dir))
                .user_of(&profile, cwd)
                .map(|user| user.path)
        };
        let id = &profile.id;
        let cwd = Some(b"/srv".as_slice());
        assert_eq!(user("/data", None), Some(format!("/data/profiles/{id}")));
        assert_eq!(
            user("/data/./x/../y/", None),
            Some(format!("/data/x/../y/profiles/{id}"))
        );
        assert_eq!(user("//data", None), Some(format!("//data/profiles/{id}")));
        assert_eq!(user("data", cwd), Some(format!("/srv/data/profiles/{id}")));
        assert_eq!(user("", cwd), Some(format!("/srv/profiles/{id}")));
        assert_eq!(user("data", None), None);
        let anna = DataDir::new(PathBuf::from("/data"))
            .user_of(&profile, None)
            .unwrap();
        assert_eq!(anna.id, "pzYaT1SJsVifVQ");
        assert_eq!(anna.username, "anna");
    }

    #[test]
    fn finds_the_last_logged_user_by_the_id_python_wrote() {
        let directory =
            std::env::temp_dir().join(format!("ferrofolio-last-logged-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let data_dir = DataDir::new(directory.clone());
        let last = |profiles: Value| {
            std::fs::write(directory.join("profiles.json"), profiles.to_string()).unwrap();
            data_dir.last_logged()
        };
        let id = "0d3e7c1a-5b2f-4c8e-9a1b-2c3d4e5f6a7b";
        assert_eq!(
            last(serde_json::json!({"last_logged": id, "profiles": [
                {"id": "11111111-1111-1111-1111-111111111111", "name": "other"},
                {"id": id, "name": "anna", "guest": true},
            ]})),
            Some(LastLogged {
                username: Value::from("anna"),
                guest: Value::Bool(true),
            })
        );
        assert_eq!(
            last(serde_json::json!({"last_logged": id, "profiles": [{"id": id, "name": "anna"}]})),
            Some(LastLogged {
                username: Value::from("anna"),
                guest: Value::Bool(false),
            })
        );
        // The ids are compared as Python wrote them, not as UUIDs.
        assert_eq!(
            last(serde_json::json!({"last_logged": id, "profiles": [
                {"id": id.to_uppercase(), "name": "anna"},
            ]})),
            None
        );
        assert_eq!(
            last(
                serde_json::json!({"last_logged": null, "profiles": [{"id": id, "name": "anna"}]})
            ),
            None
        );
        assert_eq!(
            last(serde_json::json!({"last_logged": "", "profiles": [{"id": "", "name": "anna"}]})),
            None
        );
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(data_dir.last_logged(), None);
    }
}
