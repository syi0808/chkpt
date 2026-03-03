use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// Built-in directories that are always excluded from scanning.
const BUILTIN_EXCLUSIONS: &[&str] = &[
    ".git/",
    "node_modules/",
    ".chkpt/",
    "target/",
];

/// Matcher that combines built-in exclusions with .chkptignore patterns.
pub struct IgnoreMatcher {
    gitignore: Option<Gitignore>,
}

impl IgnoreMatcher {
    /// Create a new matcher, optionally loading patterns from a .chkptignore file.
    ///
    /// If `chkptignore_path` is provided, patterns are loaded from that file.
    /// If the file does not exist, no user patterns are loaded (only built-in exclusions apply).
    pub fn new(chkptignore_path: Option<&Path>) -> Self {
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

        Self { gitignore }
    }

    /// Check if the given relative path should be ignored.
    ///
    /// `relative_path` should use forward slashes.
    /// `is_dir` indicates whether the path is a directory.
    pub fn is_ignored(&self, relative_path: &str, is_dir: bool) -> bool {
        // Check built-in exclusions first
        for exclusion in BUILTIN_EXCLUSIONS {
            let dir_name = exclusion.trim_end_matches('/');
            // Match the directory itself or any path starting with it
            if relative_path == dir_name
                || relative_path.starts_with(&format!("{}/", dir_name))
            {
                return true;
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;

    #[test]
    fn test_builtin_exclusions() {
        let matcher = IgnoreMatcher::new(None);
        assert!(matcher.is_ignored(".git", true));
        assert!(matcher.is_ignored(".git/HEAD", false));
        assert!(matcher.is_ignored("node_modules", true));
        assert!(matcher.is_ignored("node_modules/pkg/index.js", false));
        assert!(matcher.is_ignored(".chkpt", true));
        assert!(matcher.is_ignored(".chkpt/config", false));
        assert!(matcher.is_ignored("target", true));
        assert!(matcher.is_ignored("target/debug/main", false));
    }

    #[test]
    fn test_non_excluded_paths() {
        let matcher = IgnoreMatcher::new(None);
        assert!(!matcher.is_ignored("src/main.rs", false));
        assert!(!matcher.is_ignored("README.md", false));
        assert!(!matcher.is_ignored(".gitignore", false));
    }

    #[test]
    fn test_chkptignore_patterns() {
        let dir = TempDir::new().unwrap();
        let ignore_path = dir.path().join(".chkptignore");
        fs::write(&ignore_path, "*.log\nbuild/\n").unwrap();

        let matcher = IgnoreMatcher::new(Some(&ignore_path));
        assert!(matcher.is_ignored("debug.log", false));
        assert!(matcher.is_ignored("build/out.o", false));
        assert!(!matcher.is_ignored("src/main.rs", false));
    }
}
