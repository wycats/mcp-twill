//! Lexical path normalization and boundary checks.
//!
//! Normalization resolves `.`/`..` segments and separators without touching
//! the filesystem. Boundary comparison is platform-appropriate: case-sensitive
//! for POSIX-style paths, case-insensitive (ASCII) for Windows drive-letter
//! paths. Only `file:` URIs and plain paths participate; any other scheme is
//! an [`UnsupportedRootScheme`] error.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A URI used a scheme other than `file:`, so it cannot participate in
/// path boundary checks.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("unsupported root scheme `{scheme}` in `{uri}`; only file: URIs participate in boundary checks")]
pub struct UnsupportedRootScheme {
    pub scheme: String,
    pub uri: String,
}

/// A lexically normalized path: separators unified, `.`/`..` resolved, with
/// an optional Windows drive prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedPath {
    drive: Option<char>,
    absolute: bool,
    components: Vec<String>,
}

impl NormalizedPath {
    /// The Windows drive letter, if the input was a drive-letter path.
    pub fn drive(&self) -> Option<char> {
        self.drive
    }

    pub fn is_absolute(&self) -> bool {
        self.absolute
    }

    pub fn components(&self) -> &[String] {
        &self.components
    }

    fn case_insensitive(&self) -> bool {
        self.drive.is_some()
    }
}

/// Lexically normalizes a plain path: unifies separators to `/`, detects a
/// Windows drive-letter prefix, and resolves `.` and `..` segments without
/// touching the filesystem.
pub fn normalize_path(value: &str) -> NormalizedPath {
    let unified = value.replace('\\', "/");
    let bytes = unified.as_bytes();

    let (drive, rest) = if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        (Some(bytes[0] as char), &unified[2..])
    } else {
        (None, unified.as_str())
    };

    let absolute = rest.starts_with('/') || drive.is_some();

    let mut components: Vec<String> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if components.last().is_some_and(|last| last != "..") {
                    components.pop();
                } else if !absolute {
                    // A relative path may retain leading parent segments.
                    components.push("..".to_string());
                }
            }
            other => components.push(other.to_string()),
        }
    }

    NormalizedPath {
        drive,
        absolute,
        components,
    }
}

/// Normalizes a `file:` URI or plain path into a [`NormalizedPath`].
///
/// Any scheme other than `file:` is rejected with [`UnsupportedRootScheme`].
/// Plain paths (no scheme) are accepted and normalized directly.
pub fn normalize_file_uri(value: &str) -> Result<NormalizedPath, UnsupportedRootScheme> {
    match parse_scheme(value) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file") => {
            let after_scheme = &value[scheme.len() + 1..];
            let path = after_scheme
                .strip_prefix("//")
                .unwrap_or(after_scheme);
            // `file:///C:/repo` yields `/C:/repo`; drop the slash before the drive.
            let bytes = path.as_bytes();
            let path = if bytes.len() >= 3
                && bytes[0] == b'/'
                && bytes[1].is_ascii_alphabetic()
                && bytes[2] == b':'
            {
                &path[1..]
            } else {
                path
            };
            Ok(normalize_path(path))
        }
        Some(scheme) => Err(UnsupportedRootScheme {
            scheme: scheme.to_string(),
            uri: value.to_string(),
        }),
        None => Ok(normalize_path(value)),
    }
}

/// Extracts a URI scheme from `value`, distinguishing schemes from Windows
/// drive letters: a single alphabetic character before `:` is a scheme only
/// when followed by `//`.
fn parse_scheme(value: &str) -> Option<&str> {
    let colon = value.find(':')?;
    let candidate = &value[..colon];
    if candidate.is_empty() {
        return None;
    }
    let mut chars = candidate.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    if candidate.len() == 1 && !value[colon + 1..].starts_with("//") {
        // Single letter followed by something other than `//`: a drive path.
        return None;
    }
    Some(candidate)
}

/// Reports whether `candidate` is `root` or lies inside `root`.
///
/// Drive-letter paths compare case-insensitively (drive and components,
/// ASCII); POSIX-style paths compare case-sensitively. A drive-letter path
/// and a drive-less path never contain each other.
pub fn path_has_prefix(candidate: &NormalizedPath, root: &NormalizedPath) -> bool {
    match (candidate.drive, root.drive) {
        (Some(a), Some(b)) => {
            if !a.eq_ignore_ascii_case(&b) {
                return false;
            }
        }
        (None, None) => {}
        _ => return false,
    }

    if candidate.absolute != root.absolute {
        return false;
    }

    if candidate.components.len() < root.components.len() {
        return false;
    }

    let case_insensitive = candidate.case_insensitive() && root.case_insensitive();
    root.components
        .iter()
        .zip(candidate.components.iter())
        .all(|(r, c)| {
            if case_insensitive {
                r.eq_ignore_ascii_case(c)
            } else {
                r == c
            }
        })
}

/// Reports whether two normalized paths identify the same location under the
/// platform-appropriate case rules.
pub fn paths_equal(a: &NormalizedPath, b: &NormalizedPath) -> bool {
    path_has_prefix(a, b) && path_has_prefix(b, a)
}
