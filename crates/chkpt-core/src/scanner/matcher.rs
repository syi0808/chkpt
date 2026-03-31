use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// Directories always excluded (never overridable).
const ALWAYS_EXCLUDED: &[&str] = &[".git", ".chkpt", "target"];

/// Dependency directories excluded by default, includable via --include-deps.
const DEPENDENCY_DIRS: &[&str] = &[
    "node_modules",
    ".venv",
    "venv",
    "__pypackages__",
    ".tox",
    ".nox",
    ".gradle",
    ".m2",
];

/// Matcher that combines built-in exclusions with .chkptignore patterns.
pub struct IgnoreMatcher {
    gitignore: Option<Gitignore>,
    include_deps: bool,
}

impl IgnoreMatcher {
    /// Create a new matcher, optionally loading patterns from a .chkptignore file.
    ///
    /// If `chkptignore_path` is provided, patterns are loaded from that file.
    /// If the file does not exist, no user patterns are loaded (only built-in exclusions apply).
    pub fn new(chkptignore_path: Option<&Path>, include_deps: bool) -> Self {
        let gitignore = chkptignore_path.and_then(|path| {
            if path.exists() {
                let mut builder = GitignoreBuilder::new(path.parent().unwrap_or(Path::new(".")));
                if let Some(err) = builder.add(path) {
                    tracing::warn!("Error parsing .chkptignore: {}", err);
                    return None;
                }
                match builder.build() {
                    Ok(gi) => Some(gi),
                    Err(err) => {
                        tracing::warn!("Error building .chkptignore matcher: {}", err);
                        None
                    }
                }
            } else {
                None
            }
        });

        Self {
            gitignore,
            include_deps,
        }
    }

    /// Check if the given relative path should be ignored.
    ///
    /// `relative_path` should use forward slashes.
    /// `is_dir` indicates whether the path is a directory.
    pub fn is_ignored(&self, relative_path: &str, is_dir: bool) -> bool {
        // Check built-in exclusions first
        if has_excluded_directory_component(relative_path, is_dir, self.include_deps) {
            return true;
        }

        // Check .chkptignore patterns
        if let Some(ref gi) = self.gitignore {
            let matched = gi.matched_path_or_any_parents(relative_path, is_dir);
            if matched.is_ignore() {
                return true;
            }
        }

        false
    }
}

fn has_excluded_directory_component(relative_path: &str, is_dir: bool, include_deps: bool) -> bool {
    let mut components = relative_path.split('/').peekable();

    while let Some(component) = components.next() {
        let is_last = components.peek().is_none();
        let is_directory_component = is_dir || !is_last;

        if is_directory_component {
            if ALWAYS_EXCLUDED.contains(&component) {
                return true;
            }
            if !include_deps && DEPENDENCY_DIRS.contains(&component) {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_builtin_exclusions() {
        let matcher = IgnoreMatcher::new(None, false);
        assert!(matcher.is_ignored(".git", true));
        assert!(matcher.is_ignored(".git/HEAD", false));
        assert!(matcher.is_ignored("node_modules", true));
        assert!(matcher.is_ignored("node_modules/pkg/index.js", false));
        assert!(matcher.is_ignored(".chkpt", true));
        assert!(matcher.is_ignored(".chkpt/config", false));
        assert!(matcher.is_ignored("target", true));
        assert!(matcher.is_ignored("target/debug/main", false));
        assert!(matcher.is_ignored("packages/app/node_modules", true));
        assert!(matcher.is_ignored("packages/app/node_modules/pkg/index.js", false));
        assert!(matcher.is_ignored("services/api/.venv", true));
        assert!(matcher.is_ignored("services/api/.venv/lib/site.py", false));
        assert!(matcher.is_ignored("crates/core/target", true));
        assert!(matcher.is_ignored("crates/core/target/debug/app", false));
    }

    #[test]
    fn test_non_excluded_paths() {
        let matcher = IgnoreMatcher::new(None, false);
        assert!(!matcher.is_ignored("src/main.rs", false));
        assert!(!matcher.is_ignored("README.md", false));
        assert!(!matcher.is_ignored(".gitignore", false));
        assert!(!matcher.is_ignored("src/targeting.rs", false));
        assert!(!matcher.is_ignored("src/venv_config.rs", false));
    }

    #[test]
    fn test_chkptignore_patterns() {
        let dir = TempDir::new().unwrap();
        let ignore_path = dir.path().join(".chkptignore");
        fs::write(&ignore_path, "*.log\nbuild/\n").unwrap();

        let matcher = IgnoreMatcher::new(Some(&ignore_path), false);
        assert!(matcher.is_ignored("debug.log", false));
        assert!(matcher.is_ignored("build/out.o", false));
        assert!(!matcher.is_ignored("src/main.rs", false));
    }
}
