use crate::error::{ChkpttError, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Create a git bundle from a repository, storing it in `archive_dir`.
///
/// 1. Runs `git bundle create <temp_file> --all` in the repo directory
/// 2. Reads the bundle file contents
/// 3. Computes git_key = BLAKE3 hash of bundle content, first 16 hex chars
/// 4. Moves the bundle to `archive_dir/<git_key>.bundle` (skips if already exists)
/// 5. Returns git_key
pub fn create_git_bundle(repo_path: &Path, archive_dir: &Path) -> Result<String> {
    // Create a temporary bundle file inside archive_dir to avoid cross-device moves
    let temp_bundle = archive_dir.join("_tmp_bundle.bundle");

    let output = Command::new("git")
        .args(["bundle", "create"])
        .arg(&temp_bundle)
        .arg("--all")
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Clean up temp file on failure
        let _ = fs::remove_file(&temp_bundle);
        return Err(ChkpttError::Other(format!(
            "git bundle create failed: {}",
            stderr
        )));
    }

    // Compute the key from the bundle content
    let git_key = compute_git_key(&temp_bundle)?;
    let dest = archive_dir.join(format!("{}.bundle", git_key));

    if dest.exists() {
        // Bundle with same content already archived; remove the temp file
        fs::remove_file(&temp_bundle)?;
    } else {
        // Move temp bundle to its final content-addressed name
        fs::rename(&temp_bundle, &dest)?;
    }

    Ok(git_key)
}

/// Restore a git bundle into a repository.
///
/// 1. Finds bundle at `archive_dir/<git_key>.bundle`
/// 2. Fetches refs from the bundle into remote-tracking refs
/// 3. Checks out the default branch
pub fn restore_git_bundle(repo_path: &Path, archive_dir: &Path, git_key: &str) -> Result<()> {
    let bundle_path = archive_dir.join(format!("{}.bundle", git_key));

    if !bundle_path.exists() {
        return Err(ChkpttError::ObjectNotFound(format!(
            "git bundle not found: {}",
            bundle_path.display()
        )));
    }

    // Determine what branches are in the bundle
    let list_output = Command::new("git")
        .args(["bundle", "list-heads"])
        .arg(&bundle_path)
        .current_dir(repo_path)
        .output()?;

    if !list_output.status.success() {
        let stderr = String::from_utf8_lossy(&list_output.stderr);
        return Err(ChkpttError::Other(format!(
            "git bundle list-heads failed: {}",
            stderr
        )));
    }

    let heads = String::from_utf8_lossy(&list_output.stdout);

    // Fetch from the bundle into remote-tracking refs under "bundle/"
    let output = Command::new("git")
        .arg("fetch")
        .arg(&bundle_path)
        .arg("refs/heads/*:refs/remotes/bundle/*")
        .current_dir(repo_path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ChkpttError::Other(format!(
            "git fetch from bundle failed: {}",
            stderr
        )));
    }

    // Determine the best branch to checkout
    let branch_name = if heads.contains("refs/heads/main") {
        "main"
    } else if heads.contains("refs/heads/master") {
        "master"
    } else {
        // Fall back to the first branch ref found
        heads
            .lines()
            .find_map(|line| {
                line.split_whitespace()
                    .nth(1)
                    .and_then(|r| r.strip_prefix("refs/heads/"))
            })
            .unwrap_or("main")
    };

    // Create local branch from the remote-tracking ref and checkout
    let checkout_output = Command::new("git")
        .args(["checkout", "-B", branch_name])
        .arg(format!("bundle/{}", branch_name))
        .current_dir(repo_path)
        .output()?;

    if !checkout_output.status.success() {
        let stderr = String::from_utf8_lossy(&checkout_output.stderr);
        return Err(ChkpttError::Other(format!(
            "git checkout failed: {}",
            stderr
        )));
    }

    Ok(())
}

/// Compute a content-based key for a git bundle file.
///
/// Reads the bundle file, computes a BLAKE3 hash, and returns the first 16 hex characters.
pub fn compute_git_key(bundle_path: &Path) -> Result<String> {
    let contents = fs::read(bundle_path)?;
    let hash = blake3::hash(&contents);
    let hex = hash.to_hex();
    Ok(hex[..16].to_string())
}
