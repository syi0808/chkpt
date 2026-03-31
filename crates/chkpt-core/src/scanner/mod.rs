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
    pub inode: Option<u64>,
    pub mode: u32,
}

/// Scan workspace, respecting .chkptignore and built-in exclusions.
pub fn scan_workspace(root: &Path, chkptignore: Option<&Path>) -> Result<Vec<ScannedFile>> {
    scan_workspace_parallel(root, chkptignore)
}

/// Scan workspace using the parallel walker.
pub fn scan_workspace_parallel(
    root: &Path,
    chkptignore: Option<&Path>,
) -> Result<Vec<ScannedFile>> {
    walker::walk_parallel(root, chkptignore)
}
