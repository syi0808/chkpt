use crate::error::Result;
use crate::scanner::matcher::IgnoreMatcher;
use crate::scanner::ScannedFile;
use std::path::Path;

/// Walk a workspace directory, collecting file metadata and respecting ignore rules.
///
/// If `chkptignore_override` is provided, it is used as the .chkptignore path.
/// Otherwise, `root/.chkptignore` is checked automatically.
pub fn walk(root: &Path, chkptignore_override: Option<&Path>) -> Result<Vec<ScannedFile>> {
    let chkptignore_path = match chkptignore_override {
        Some(p) => Some(p.to_path_buf()),
        None => {
            let default_path = root.join(".chkptignore");
            if default_path.exists() {
                Some(default_path)
            } else {
                None
            }
        }
    };

    let matcher = IgnoreMatcher::new(chkptignore_path.as_deref());
    let mut files = Vec::new();

    walk_dir(root, root, &matcher, &mut files)?;

    // Sort by relative path for deterministic output
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    Ok(files)
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    matcher: &IgnoreMatcher,
    files: &mut Vec<ScannedFile>,
) -> Result<()> {
    let entries = std::fs::read_dir(dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        // Compute relative path with forward slashes
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if file_type.is_dir() {
            if !matcher.is_ignored(&relative, true) {
                walk_dir(root, &path, matcher, files)?;
            }
        } else if file_type.is_file() {
            if !matcher.is_ignored(&relative, false) {
                let metadata = std::fs::metadata(&path)?;
                let scanned = build_scanned_file(&path, &relative, &metadata);
                files.push(scanned);
            }
        }
        // Symlinks are skipped
    }

    Ok(())
}

#[cfg(unix)]
fn build_scanned_file(
    path: &Path,
    relative_path: &str,
    metadata: &std::fs::Metadata,
) -> ScannedFile {
    use std::os::unix::fs::MetadataExt;

    ScannedFile {
        relative_path: relative_path.to_string(),
        absolute_path: path.to_path_buf(),
        size: metadata.len(),
        mtime_secs: metadata.mtime(),
        mtime_nanos: metadata.mtime_nsec(),
        inode: Some(metadata.ino()),
        mode: metadata.mode(),
    }
}

#[cfg(not(unix))]
fn build_scanned_file(
    path: &Path,
    relative_path: &str,
    metadata: &std::fs::Metadata,
) -> ScannedFile {
    use std::time::UNIX_EPOCH;

    let (mtime_secs, mtime_nanos) = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs() as i64, d.subsec_nanos() as i64))
        .unwrap_or((0, 0));

    ScannedFile {
        relative_path: relative_path.to_string(),
        absolute_path: path.to_path_buf(),
        size: metadata.len(),
        mtime_secs,
        mtime_nanos,
        inode: None,
        mode: 0o644,
    }
}
