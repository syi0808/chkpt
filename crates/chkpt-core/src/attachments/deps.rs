use crate::error::Result;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

/// Compute a deterministic key from a lockfile's contents.
///
/// Uses BLAKE3 to hash the file contents and returns the first 16 hex characters.
pub fn compute_deps_key(lockfile_path: &Path) -> Result<String> {
    let contents = fs::read(lockfile_path)?;
    let hash = blake3::hash(&contents);
    let hex = hash.to_hex();
    Ok(hex[..16].to_string())
}

/// Archive a deps directory (e.g. node_modules) as a tar.zst archive.
///
/// The archive is stored at `archive_dir/<deps_key>.tar.zst`.
/// If an archive with the same key already exists, it is reused (no work done).
/// Returns the deps_key.
pub fn archive_deps(deps_dir: &Path, archive_dir: &Path, deps_key: &str) -> Result<String> {
    let archive_path = archive_dir.join(format!("{}.tar.zst", deps_key));

    // If archive already exists, skip creation (reuse)
    if archive_path.exists() {
        return Ok(deps_key.to_string());
    }

    // Create tar archive in memory
    let mut tar_builder = tar::Builder::new(Vec::new());
    tar_builder.append_dir_all(".", deps_dir)?;
    let tar_data = tar_builder.into_inner()?;

    // Compress with zstd
    let compressed = zstd::encode_all(Cursor::new(&tar_data), 3)?;

    // Write to file
    fs::write(&archive_path, &compressed)?;

    Ok(deps_key.to_string())
}

/// Restore a deps directory from a tar.zst archive.
///
/// Reads the archive at `archive_dir/<deps_key>.tar.zst` and extracts it to `deps_dir`.
pub fn restore_deps(deps_dir: &Path, archive_dir: &Path, deps_key: &str) -> Result<()> {
    let archive_path = archive_dir.join(format!("{}.tar.zst", deps_key));

    // Read compressed archive
    let compressed = fs::read(&archive_path)?;

    // Decompress with zstd
    let mut decoder = zstd::Decoder::new(Cursor::new(&compressed))?;
    let mut tar_data = Vec::new();
    decoder.read_to_end(&mut tar_data)?;

    // Ensure target directory exists
    fs::create_dir_all(deps_dir)?;

    // Extract tar archive
    let mut archive = tar::Archive::new(Cursor::new(&tar_data));
    archive.unpack(deps_dir)?;

    Ok(())
}
