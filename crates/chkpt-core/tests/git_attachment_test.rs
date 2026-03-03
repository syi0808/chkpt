use chkpt_core::attachments::git::{compute_git_key, create_git_bundle, restore_git_bundle};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn init_git_repo(dir: &std::path::Path) {
    Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir)
        .output()
        .unwrap();
    fs::write(dir.join("file.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(dir)
        .output()
        .unwrap();
}

#[test]
fn test_create_git_bundle() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);

    let archive_dir = dir.path().join("bundles");
    fs::create_dir_all(&archive_dir).unwrap();

    let key = create_git_bundle(&repo, &archive_dir).unwrap();
    assert!(!key.is_empty());
    assert!(archive_dir.join(format!("{}.bundle", key)).exists());
}

#[test]
fn test_restore_git_bundle() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);

    let archive_dir = dir.path().join("bundles");
    fs::create_dir_all(&archive_dir).unwrap();

    let key = create_git_bundle(&repo, &archive_dir).unwrap();

    // Create a new empty repo and restore into it
    let new_repo = dir.path().join("new_repo");
    fs::create_dir_all(&new_repo).unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(&new_repo)
        .output()
        .unwrap();

    restore_git_bundle(&new_repo, &archive_dir, &key).unwrap();

    // Should have the refs from the bundle
    let output = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(&new_repo)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(log.contains("initial"));
}

#[test]
fn test_git_key_based_on_content() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);

    let archive_dir = dir.path().join("bundles");
    fs::create_dir_all(&archive_dir).unwrap();

    let key1 = create_git_bundle(&repo, &archive_dir).unwrap();
    // Creating again should return same key (content hasn't changed)
    let key2 = create_git_bundle(&repo, &archive_dir).unwrap();
    assert_eq!(key1, key2);
}

#[test]
fn test_compute_git_key() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_git_repo(&repo);

    let archive_dir = dir.path().join("bundles");
    fs::create_dir_all(&archive_dir).unwrap();

    let key = create_git_bundle(&repo, &archive_dir).unwrap();
    let bundle_path = archive_dir.join(format!("{}.bundle", key));

    // compute_git_key on the bundle file should return the same key
    let computed_key = compute_git_key(&bundle_path).unwrap();
    assert_eq!(key, computed_key);
    assert_eq!(computed_key.len(), 16); // 16 hex chars
}
