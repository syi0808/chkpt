pub mod matcher;
pub mod walker;

use crate::error::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub relative_path: String,
    pub absolute_path: std::path::PathBuf,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub device: Option<u64>,
    pub inode: Option<u64>,
    pub mode: u32,
    pub is_symlink: bool,
}

/// Scan workspace, respecting .chkptignore and built-in exclusions.
pub fn scan_workspace(root: &Path, chkptignore: Option<&Path>) -> Result<Vec<ScannedFile>> {
    scan_workspace_with_options(root, chkptignore, false)
}

/// Scan workspace with configurable options.
pub fn scan_workspace_with_options(
    root: &Path,
    chkptignore: Option<&Path>,
    include_deps: bool,
) -> Result<Vec<ScannedFile>> {
    walker::walk_parallel(root, chkptignore, include_deps)
}

/// Scan workspace using the parallel walker.
pub fn scan_workspace_parallel(
    root: &Path,
    chkptignore: Option<&Path>,
) -> Result<Vec<ScannedFile>> {
    walker::walk_parallel(root, chkptignore, false)
}
