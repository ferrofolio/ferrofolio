// SPDX-License-Identifier: AGPL-3.0-only

//! Paths as Python's `pathlib` and `os.path` write them on POSIX, for the paths the
//! status reports. They work on bytes, as Python's paths are the bytes of the file
//! system read with `surrogateescape`.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

/// `str(PurePosixPath(raw))`: repeated slashes and `.` components are dropped, `..`
/// is kept, and exactly two leading slashes stay as they are, as POSIX allows.
pub fn normalize(raw: &[u8]) -> Vec<u8> {
    let (root, tail) = parse(raw);
    format(root, &tail)
}

/// `os.path.join(base, name)` for a `name` without a leading slash.
pub fn join(base: &[u8], name: &[u8]) -> Vec<u8> {
    let mut joined = base.to_vec();
    if !joined.is_empty() && !joined.ends_with(b"/") {
        joined.push(b'/');
    }
    joined.extend_from_slice(name);
    joined
}

/// `str(Path(raw).absolute())`: the normalised path, after the working directory
/// if it is relative. Nothing is resolved; `..` stays. A relative path needs the
/// working directory.
pub fn absolute(raw: &[u8], cwd: Option<&[u8]>) -> Option<Vec<u8>> {
    let (root, tail) = parse(raw);
    if !root.is_empty() {
        return Some(format(root, &tail));
    }
    let cwd = cwd?;
    if tail.is_empty() {
        return Some(cwd.to_vec());
    }
    let (cwd_root, mut cwd_tail) = parse(cwd);
    cwd_tail.extend(tail);
    Some(format(cwd_root, &cwd_tail))
}

/// `str(Path(raw).resolve())`: `os.path.realpath` of the normalised path, which
/// follows symbolic links and `..` and keeps the components that do not exist. A
/// relative path needs the working directory.
pub fn resolve(raw: &[u8], cwd: Option<&[u8]>) -> Option<Vec<u8>> {
    let filename = normalize(raw);
    let start = if filename.starts_with(b"/") {
        b"/".to_vec()
    } else {
        cwd?.to_vec()
    };
    Some(normalize(&realpath(&filename, start)))
}

/// `posixpath.realpath(filename, strict=False)` of Python 3.14, from `start`, which
/// is `/` for an absolute `filename` and the working directory otherwise.
fn realpath(filename: &[u8], start: Vec<u8>) -> Vec<u8> {
    // The parts still to resolve, the next one last. `None` marks a symbolic link
    // whose target has been resolved; the link's own path follows it.
    let mut rest: Vec<Option<Vec<u8>>> = filename
        .split(|byte| *byte == b'/')
        .rev()
        .map(|part| Some(part.to_vec()))
        .collect();
    let mut part_count = rest.len();
    let mut path = start;
    // Links seen, with their resolved target once it is known.
    let mut seen: HashMap<Vec<u8>, Option<Vec<u8>>> = HashMap::new();

    while part_count > 0 {
        let Some(entry) = rest.pop() else {
            break;
        };
        let Some(name) = entry else {
            if let Some(Some(link)) = rest.pop() {
                seen.insert(link, Some(path.clone()));
            }
            continue;
        };
        part_count -= 1;
        if name.is_empty() || name == b"." {
            continue;
        }
        if name == b".." {
            let parent = path.iter().rposition(|byte| *byte == b'/').unwrap_or(0);
            path.truncate(parent);
            if path.is_empty() {
                path.push(b'/');
            }
            continue;
        }
        let new_path = if path == b"/" {
            [path.as_slice(), &name].concat()
        } else {
            [path.as_slice(), b"/", &name].concat()
        };
        let is_link = std::fs::symlink_metadata(OsStr::from_bytes(&new_path))
            .is_ok_and(|metadata| metadata.file_type().is_symlink());
        if !is_link {
            // Not a link, or an error that a non-strict realpath ignores.
            path = new_path;
            continue;
        }
        if let Some(resolved) = seen.get(&new_path) {
            // A resolved target is reused; an unresolved one is a loop.
            path = resolved.clone().unwrap_or(new_path);
            continue;
        }
        let Ok(target) = std::fs::read_link(OsStr::from_bytes(&new_path)) else {
            path = new_path;
            continue;
        };
        let target = target.into_os_string().into_vec();
        if target.starts_with(b"/") {
            path = b"/".to_vec();
        }
        seen.insert(new_path.clone(), None);
        rest.push(Some(new_path));
        rest.push(None);
        let parts: Vec<Option<Vec<u8>>> = target
            .split(|byte| *byte == b'/')
            .rev()
            .map(|part| Some(part.to_vec()))
            .collect();
        part_count += parts.len();
        rest.extend(parts);
    }
    path
}

/// The root (empty, `/` or `//`) and the components of a path, as `pathlib` parses
/// it.
fn parse(raw: &[u8]) -> (&'static [u8], Vec<Vec<u8>>) {
    // `posixpath.splitroot`: two slashes stay, one or three and more become one.
    let root: &'static [u8] = if !raw.starts_with(b"/") {
        b""
    } else if raw.starts_with(b"//") && !raw.starts_with(b"///") {
        b"//"
    } else {
        b"/"
    };
    let tail = raw
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty() && *part != b".")
        .map(<[u8]>::to_vec)
        .collect();
    (root, tail)
}

fn format(root: &[u8], tail: &[Vec<u8>]) -> Vec<u8> {
    if root.is_empty() && tail.is_empty() {
        return b".".to_vec();
    }
    let mut formatted = root.to_vec();
    formatted.extend_from_slice(&tail.join(&b'/'));
    formatted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: Option<Vec<u8>>) -> Option<String> {
        bytes.map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn normalizes_like_pure_posix_path() {
        let normalized = |raw: &str| String::from_utf8(normalize(raw.as_bytes())).unwrap();
        assert_eq!(normalized(""), ".");
        assert_eq!(normalized("."), ".");
        assert_eq!(normalized("./a//b/./c/"), "a/b/c");
        assert_eq!(normalized("a/../b"), "a/../b");
        assert_eq!(normalized("/"), "/");
        assert_eq!(normalized("//"), "//");
        assert_eq!(normalized("///"), "/");
        assert_eq!(normalized("//a//b"), "//a/b");
        assert_eq!(normalized("///a"), "/a");
        assert_eq!(normalized("/a/."), "/a");
    }

    #[test]
    fn joins_like_os_path() {
        let joined = |base: &str, name: &str| {
            String::from_utf8(join(base.as_bytes(), name.as_bytes())).unwrap()
        };
        assert_eq!(joined("a", "b"), "a/b");
        assert_eq!(joined("a/", "b"), "a/b");
        assert_eq!(joined("", "b"), "b");
        assert_eq!(joined("/", "b"), "/b");
    }

    #[test]
    fn makes_paths_absolute_without_resolving_them() {
        let cwd = Some(b"/work/dir".as_slice());
        let absolute = |raw: &str| text(absolute(raw.as_bytes(), cwd));
        assert_eq!(absolute("/a/./b/../c"), Some("/a/b/../c".to_owned()));
        assert_eq!(absolute("a/b"), Some("/work/dir/a/b".to_owned()));
        assert_eq!(absolute("../a"), Some("/work/dir/../a".to_owned()));
        assert_eq!(absolute(""), Some("/work/dir".to_owned()));
        assert_eq!(absolute("//a"), Some("//a".to_owned()));
        assert_eq!(
            text(super::absolute(b"a", Some(b"/"))),
            Some("/a".to_owned())
        );
        assert_eq!(super::absolute(b"a", None), None);
        assert_eq!(text(super::absolute(b"/a", None)), Some("/a".to_owned()));
    }
}
