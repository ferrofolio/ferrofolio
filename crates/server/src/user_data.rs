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

use crate::sessions::Credentials;

/// The name of the data directory in the platform's user data location.
const APP_NAME: &str = "Ferrofolio";

/// Its name before the rename; an existing directory is moved once (`adopt_legacy_dir`).
const LEGACY_APP_NAME: &str = "Finanze";

/// The data directory Python uses without `--data-dir`, resolved as
/// `finanze/data_dir.py` resolves it: `appdirs.user_data_dir("Ferrofolio", False)`,
/// after moving an existing `Finanze` directory next to it there once.
pub fn default_data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::home_dir().map(OsString::from));
    let base = data_base_for(
        std::env::var_os("XDG_DATA_HOME"),
        home,
        cfg!(target_os = "macos"),
    );
    let legacy = base.join(LEGACY_APP_NAME);
    let target = base.join(APP_NAME);
    match adopt_legacy_dir(&legacy, &target) {
        Adoption::NoLegacy => target,
        Adoption::Moved => {
            tracing::info!(from = %legacy.display(), to = %target.display(), "moved the data directory");
            target
        }
        Adoption::BothExist => {
            tracing::warn!(
                used = %target.display(),
                unchanged = %legacy.display(),
                "both data directories exist; start with --data-dir and the unchanged one to use it instead"
            );
            target
        }
        Adoption::Failed(error) => {
            tracing::warn!(
                %error,
                from = %legacy.display(),
                to = %target.display(),
                "could not move the data directory, keeping the old one"
            );
            legacy
        }
    }
}

/// What `adopt_legacy_dir` did.
#[derive(Debug)]
enum Adoption {
    /// There is no legacy directory.
    NoLegacy,
    /// The legacy directory was renamed to the new one.
    Moved,
    /// The new directory already holds something; both stay as they are and the
    /// new one is used.
    BothExist,
    /// The legacy directory could not be moved and stays in use, so that its data
    /// is not hidden behind an empty directory.
    Failed(std::io::Error),
}

/// Renames `legacy` to `target` unless `target` already holds something, as
/// `adopt_legacy_dir` in `finanze/data_dir.py` does.
fn adopt_legacy_dir(legacy: &Path, target: &Path) -> Adoption {
    adopt_legacy_dir_with(legacy, target, |from, to| std::fs::rename(from, to))
}

/// `adopt_legacy_dir` with the rename passed in, so that tests can interleave
/// another start.
fn adopt_legacy_dir_with(
    legacy: &Path,
    target: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> Adoption {
    if !legacy.is_dir() {
        return Adoption::NoLegacy;
    }
    match std::fs::read_dir(target).map(|mut entries| entries.next()) {
        Ok(Some(Ok(_))) => return Adoption::BothExist,
        Ok(Some(Err(error))) => return Adoption::Failed(error),
        Ok(None) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Adoption::Failed(error),
    }
    // Replaces an empty target directory atomically (POSIX rename).
    match rename(legacy, target) {
        Ok(()) => Adoption::Moved,
        // Another start moved it in the meantime; the old path no longer exists.
        Err(_) if !legacy.is_dir() && target.is_dir() => Adoption::NoLegacy,
        Err(error) => Adoption::Failed(error),
    }
}

/// The base of `appdirs.user_data_dir` for the given environment: on macOS
/// `~/Library/Application Support`, elsewhere `XDG_DATA_HOME`, which counts as set
/// even when empty, or `~/.local/share`.
fn data_base_for(xdg_data_home: Option<OsString>, home: Option<OsString>, macos: bool) -> PathBuf {
    match (macos, xdg_data_home) {
        (true, _) => expand_home(home, "Library/Application Support"),
        (false, Some(xdg_data_home)) => PathBuf::from(xdg_data_home),
        (false, None) => expand_home(home, ".local/share"),
    }
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
    profiles: Vec<serde_json::Value>,
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

    /// The database of the user with this name: that of the first profile with
    /// the name, as Python's `get_user` finds it. Python keeps `profiles.json` in
    /// memory and writes it on every change, so the file is current once Python
    /// has answered a login or signup.
    pub fn database_path(&self, username: &str) -> Option<PathBuf> {
        let text = std::fs::read(self.0.join("profiles.json")).ok()?;
        let profiles: Profiles = serde_json::from_slice(&text).ok()?;
        let profile = profiles
            .profiles
            .iter()
            .find(|profile| profile.get("name").and_then(|name| name.as_str()) == Some(username))?;
        let id = canonical_uuid(profile.get("id")?.as_str()?)?;
        Some(self.0.join("profiles").join(id).join("data.db"))
    }
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
        let connection = connect(&path, &credentials.password)?;
        Ok(UserDatabase {
            connection: Some(connection),
            path,
            credentials,
        })
    }

    /// The user whose database this is.
    pub fn username(&self) -> &str {
        &self.credentials.username
    }

    /// Whether these are the credentials the database was opened with.
    pub fn opened_with(&self, credentials: &Credentials) -> bool {
        self.credentials.username == credentials.username
            && self.credentials.password == credentials.password
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
        let dir = |xdg, home, macos| data_base_for(xdg, home, macos).join(APP_NAME);
        assert_eq!(
            dir(None, some("/home/anna"), false),
            PathBuf::from("/home/anna/.local/share/Ferrofolio")
        );
        assert_eq!(
            dir(None, some("/home/anna/"), false),
            PathBuf::from("/home/anna/.local/share/Ferrofolio")
        );
        assert_eq!(
            dir(some("/data/xdg"), some("/home/anna"), false),
            PathBuf::from("/data/xdg/Ferrofolio")
        );
        // An empty XDG_DATA_HOME makes the directory relative, as in appdirs.
        assert_eq!(
            dir(some(""), some("/home/anna"), false),
            PathBuf::from("Ferrofolio")
        );
        assert_eq!(
            dir(None, some("/"), false),
            PathBuf::from("/.local/share/Ferrofolio")
        );
        assert_eq!(
            dir(None, None, false),
            PathBuf::from("~/.local/share/Ferrofolio")
        );
        assert_eq!(
            dir(some("/data/xdg"), some("/Users/anna"), true),
            PathBuf::from("/Users/anna/Library/Application Support/Ferrofolio")
        );
    }

    /// A new directory under the system's temporary directory, removed on drop,
    /// also when an assertion fails.
    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new(name: &str) -> Self {
            // A directory left by an earlier run is skipped rather than reused.
            (0..1000)
                .map(|n| {
                    std::env::temp_dir()
                        .join(format!("ferrofolio-{name}-{}-{n}", std::process::id()))
                })
                .find_map(|directory| {
                    std::fs::create_dir(&directory)
                        .ok()
                        .map(|()| TempDirectory(directory))
                })
                .unwrap()
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The legacy and the new directory inside `base`, with data in the legacy one.
    fn legacy_with_data(base: &Path) -> (PathBuf, PathBuf) {
        let legacy = base.join(LEGACY_APP_NAME);
        std::fs::create_dir_all(legacy.join("profiles")).unwrap();
        std::fs::write(legacy.join("profiles.json"), "legacy").unwrap();
        (legacy, base.join(APP_NAME))
    }

    #[test]
    fn uses_the_new_directory_without_legacy_data() {
        let base = TempDirectory::new("adopt-none");
        let target = base.0.join(APP_NAME);
        assert!(matches!(
            adopt_legacy_dir(&base.0.join(LEGACY_APP_NAME), &target),
            Adoption::NoLegacy
        ));
        assert!(!target.exists());
    }

    #[test]
    fn moves_the_legacy_directory() {
        let base = TempDirectory::new("adopt-move");
        let (legacy, target) = legacy_with_data(&base.0);
        assert!(matches!(
            adopt_legacy_dir(&legacy, &target),
            Adoption::Moved
        ));
        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(target.join("profiles.json")).unwrap(),
            "legacy"
        );
        assert!(target.join("profiles").is_dir());
    }

    #[test]
    fn moves_into_an_empty_new_directory() {
        let base = TempDirectory::new("adopt-empty");
        let (legacy, target) = legacy_with_data(&base.0);
        std::fs::create_dir(&target).unwrap();
        assert!(matches!(
            adopt_legacy_dir(&legacy, &target),
            Adoption::Moved
        ));
        assert!(!legacy.exists());
        assert!(target.join("profiles.json").exists());
    }

    #[test]
    fn never_mixes_two_directories_with_data() {
        let base = TempDirectory::new("adopt-both");
        let (legacy, target) = legacy_with_data(&base.0);
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("profiles.json"), "new").unwrap();
        assert!(matches!(
            adopt_legacy_dir(&legacy, &target),
            Adoption::BothExist
        ));
        assert_eq!(
            std::fs::read_to_string(legacy.join("profiles.json")).unwrap(),
            "legacy"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("profiles.json")).unwrap(),
            "new"
        );
        assert!(!target.join("profiles").exists());
    }

    #[test]
    fn uses_the_new_directory_when_another_start_moved_it_first() {
        let base = TempDirectory::new("adopt-race");
        let (legacy, target) = legacy_with_data(&base.0);
        let adoption = adopt_legacy_dir_with(&legacy, &target, |from, to| {
            std::fs::rename(from, to)?;
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        });
        assert!(matches!(adoption, Adoption::NoLegacy));
        assert!(!legacy.exists());
        assert!(target.join("profiles.json").exists());
    }

    #[test]
    fn keeps_the_legacy_directory_when_the_rename_fails() {
        let base = TempDirectory::new("adopt-fail");
        let (legacy, target) = legacy_with_data(&base.0);
        let adoption = adopt_legacy_dir_with(&legacy, &target, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        });
        assert!(matches!(adoption, Adoption::Failed(_)));
        assert!(legacy.join("profiles.json").exists());
        assert!(!target.exists());
    }

    #[test]
    fn keeps_the_legacy_directory_when_the_new_path_is_a_file() {
        let base = TempDirectory::new("adopt-file");
        let (legacy, target) = legacy_with_data(&base.0);
        std::fs::write(&target, "not a directory").unwrap();
        assert!(matches!(
            adopt_legacy_dir(&legacy, &target),
            Adoption::Failed(_)
        ));
        assert!(legacy.join("profiles.json").exists());
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
        let cleanup = TempDirectory::new("user-data");
        let directory = cleanup.0.clone();
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
        drop(cleanup);
    }
}
