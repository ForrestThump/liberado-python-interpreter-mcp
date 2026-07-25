//! Path containment for the file tools.
//!
//! `read_file` / `write_file` / `edit_file` run in the *server* process, not inside a session's
//! sandbox. Without a check they reach every path the container can see, which would make this
//! MCP a way around Liberado's zone and write-class model: an agent denied a vault write could
//! simply ask the interpreter to do it. Every path therefore resolves against an allowlist of
//! roots before any I/O happens.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum WorkspaceError {
    #[error("path {requested:?} is outside the permitted workspace roots ({roots})")]
    Outside { requested: String, roots: String },
    #[error("path {0:?} is empty")]
    Empty(String),
}

/// Resolve a caller-supplied path to an absolute path inside one of `roots`.
///
/// Relative paths are taken against the first root, which is what makes `write_file("out.csv")`
/// mean "in my workspace" rather than "wherever the daemon happens to be running".
pub fn resolve(requested: &str, roots: &[PathBuf]) -> Result<PathBuf, WorkspaceError> {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return Err(WorkspaceError::Empty(requested.to_string()));
    }

    let first = roots
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR.to_string()));
    let joined = {
        let candidate = Path::new(trimmed);
        if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            first.join(candidate)
        }
    };

    // Collapse `..` lexically *first*. Doing it after canonicalisation would be too late for a
    // path whose parent does not exist yet, which is exactly the `write_file` case.
    let lexical = lexically_normalize(&joined);
    let real = canonicalize_existing_prefix(&lexical);

    for root in roots {
        let root_real = canonicalize_existing_prefix(&lexically_normalize(root));
        if real.starts_with(&root_real) {
            return Ok(real);
        }
    }

    Err(WorkspaceError::Outside {
        requested: requested.to_string(),
        roots: roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

/// Remove `.` and `..` components without consulting the filesystem.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // Never pop past the prefix/root: `/..` is `/`.
                if matches!(
                    out.components().next_back(),
                    Some(Component::Normal(_)) | None
                ) && out.parent().is_some()
                {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalise as much of `path` as exists, then re-append the rest.
///
/// A pure `canonicalize` fails on paths that do not exist yet; a pure lexical normalisation
/// misses symlinks. Resolving the existing prefix catches a symlinked directory (or an existing
/// symlinked file) while still allowing creation of new files.
fn canonicalize_existing_prefix(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    let mut remainder: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cursor = path;
    while let Some(parent) = cursor.parent() {
        let Some(name) = cursor.file_name() else {
            break;
        };
        remainder.push(name);
        if let Ok(real) = std::fs::canonicalize(parent) {
            let mut out = real;
            for part in remainder.iter().rev() {
                out.push(part);
            }
            return out;
        }
        cursor = parent;
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_roots() -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        (dir, vec![root])
    }

    #[test]
    fn accepts_a_path_inside_the_root() {
        let (_guard, roots) = temp_roots();
        let target = roots[0].join("notes.txt");
        let resolved = resolve(target.to_str().unwrap(), &roots).unwrap();
        assert_eq!(resolved, target);
    }

    #[test]
    fn relative_paths_resolve_against_the_first_root() {
        let (_guard, roots) = temp_roots();
        let resolved = resolve("sub/out.csv", &roots).unwrap();
        assert_eq!(resolved, roots[0].join("sub").join("out.csv"));
    }

    #[test]
    fn accepts_a_not_yet_existing_nested_path() {
        let (_guard, roots) = temp_roots();
        let resolved = resolve("a/b/c/new.txt", &roots).unwrap();
        assert!(resolved.starts_with(&roots[0]));
        assert!(resolved.ends_with("new.txt"));
    }

    #[test]
    fn rejects_a_path_outside_the_root() {
        let (_guard, roots) = temp_roots();
        assert!(matches!(
            resolve("/etc/passwd", &roots),
            Err(WorkspaceError::Outside { .. })
        ));
    }

    /// The obvious escape. `..` is collapsed before any filesystem lookup, so it is caught even
    /// when the intermediate directories do not exist.
    #[test]
    fn rejects_dot_dot_traversal() {
        let (_guard, roots) = temp_roots();
        for attempt in [
            "../outside.txt",
            "a/../../outside.txt",
            "./../../etc/passwd",
        ] {
            assert!(
                matches!(
                    resolve(attempt, &roots),
                    Err(WorkspaceError::Outside { .. })
                ),
                "{attempt:?} should have been refused"
            );
        }
    }

    /// A `..` that stays inside the root is fine — the check is on the destination, not on the
    /// spelling of the path.
    #[test]
    fn allows_dot_dot_that_lands_back_inside() {
        let (_guard, roots) = temp_roots();
        let resolved = resolve("a/../b.txt", &roots).unwrap();
        assert_eq!(resolved, roots[0].join("b.txt"));
    }

    /// Lexical normalisation alone would accept this: the path never spells `..`, but the
    /// directory it walks through points outside the root.
    #[cfg(unix)]
    #[test]
    fn rejects_traversal_through_a_symlinked_directory() {
        let (_guard, roots) = temp_roots();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), b"secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), roots[0].join("escape")).unwrap();

        assert!(matches!(
            resolve("escape/secret.txt", &roots),
            Err(WorkspaceError::Outside { .. })
        ));
    }

    #[test]
    fn rejects_an_empty_path() {
        let (_guard, roots) = temp_roots();
        assert!(matches!(
            resolve("   ", &roots),
            Err(WorkspaceError::Empty(_))
        ));
    }

    #[test]
    fn honours_multiple_roots() {
        let (_a, mut roots) = temp_roots();
        let second = tempfile::tempdir().unwrap();
        roots.push(std::fs::canonicalize(second.path()).unwrap());

        let in_second = roots[1].join("x.txt");
        assert!(resolve(in_second.to_str().unwrap(), &roots).is_ok());
    }

    #[test]
    fn normalize_never_escapes_the_filesystem_root() {
        let normalized = lexically_normalize(Path::new("/../../.."));
        assert_eq!(normalized, PathBuf::from(std::path::MAIN_SEPARATOR_STR));
    }
}
