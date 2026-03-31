#[allow(unused_imports)]
use chkpt_core::scanner::{scan_workspace, ScannedFile};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_scan_basic_files() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::write(dir.path().join("b.txt"), "world").unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main(){}").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert_eq!(files.len(), 3);
}

#[test]
fn test_scan_respects_chkptignore() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "keep").unwrap();
    fs::write(dir.path().join("b.log"), "ignore").unwrap();
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::write(dir.path().join("build/out.o"), "ignore").unwrap();
    fs::write(dir.path().join(".chkptignore"), "*.log\nbuild/\n").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(!paths.contains(&"b.log"));
    assert!(!paths.contains(&"build/out.o"));
    // .chkptignore itself should be included
    assert!(paths.contains(&".chkptignore"));
}

#[test]
fn test_scan_excludes_chkpt_dir() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "data").unwrap();
    fs::create_dir_all(dir.path().join(".chkpt")).unwrap();
    fs::write(dir.path().join(".chkpt/config"), "x").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(!paths.iter().any(|p| p.starts_with(".chkpt")));
}

#[test]
fn test_scan_excludes_git_dir_by_default() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "data").unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/HEAD"), "ref").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert!(!files.iter().any(|f| f.relative_path.starts_with(".git")));
}

#[test]
fn test_scan_excludes_node_modules_by_default() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("index.js"), "code").unwrap();
    fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
    fs::write(dir.path().join("node_modules/pkg/index.js"), "dep").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert!(!files
        .iter()
        .any(|f| f.relative_path.starts_with("node_modules")));
}

#[test]
fn test_scanned_file_has_metadata() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("test.txt"), "content").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "test.txt");
    assert_eq!(files[0].size, 7);
    assert!(files[0].mtime_secs > 0);
}

#[test]
fn test_parallel_walk_matches_scan_workspace() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::write(dir.path().join("src/nested/main.rs"), "fn main(){}").unwrap();
    fs::write(dir.path().join(".chkptignore"), "*.tmp\n").unwrap();
    fs::write(dir.path().join("skip.tmp"), "ignore me").unwrap();

    let standard = scan_workspace(dir.path(), None).unwrap();
    let parallel = chkpt_core::scanner::walker::walk_parallel(dir.path(), None).unwrap();

    let standard_paths: Vec<_> = standard.iter().map(|f| f.relative_path.clone()).collect();
    let parallel_paths: Vec<_> = parallel.iter().map(|f| f.relative_path.clone()).collect();

    assert_eq!(parallel_paths, standard_paths);
}

#[test]
fn test_parallel_scan_entrypoint_matches_sequential_walk() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::write(dir.path().join("src/nested/main.rs"), "fn main(){}").unwrap();
    fs::write(dir.path().join(".chkptignore"), "*.tmp\n").unwrap();
    fs::write(dir.path().join("skip.tmp"), "ignore me").unwrap();

    let sequential = chkpt_core::scanner::walker::walk(dir.path(), None).unwrap();
    let parallel = chkpt_core::scanner::scan_workspace_parallel(dir.path(), None).unwrap();

    let sequential_paths: Vec<_> = sequential.iter().map(|f| f.relative_path.clone()).collect();
    let parallel_paths: Vec<_> = parallel.iter().map(|f| f.relative_path.clone()).collect();

    assert_eq!(parallel_paths, sequential_paths);
}
