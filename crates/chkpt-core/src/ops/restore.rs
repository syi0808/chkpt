use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::FileIndex;
use crate::ops::io_order::sort_scanned_for_locality;
use crate::ops::lock::ProjectLock;
use crate::scanner::ScannedFile;
use crate::store::blob::{hash_file, BlobStore};
use crate::store::pack::{read_object_from_pack_set, PackSet};
use crate::store::snapshot::SnapshotStore;
use crate::store::tree::{EntryType, TreeStore};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ops::progress::{emit, ProgressCallback, ProgressEvent};

#[derive(Default)]
pub struct RestoreOptions {
    pub dry_run: bool,
    pub progress: ProgressCallback,
}

#[derive(Debug)]
pub struct RestoreResult {
    pub snapshot_id: String,
    pub files_added: u64,
    pub files_changed: u64,
    pub files_removed: u64,
    pub files_unchanged: u64,
}

struct CurrentFileState {
    hash_hex: String,
}

struct RestoreDiff {
    files_to_add: Vec<String>,
    files_to_change: Vec<String>,
    files_to_remove: Vec<String>,
    files_unchanged: u64,
}

/// Convert a [u8; 32] to a 64-char hex string.
fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Recursively walk a tree and collect all file entries as (relative_path, blob_hash_hex).
fn collect_tree_files(
    tree_store: &TreeStore,
    tree_hash_hex: &str,
    prefix: &str,
    result: &mut BTreeMap<String, String>,
) -> Result<()> {
    let entries = tree_store.read(tree_hash_hex)?;
    for entry in &entries {
        let path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };
        match entry.entry_type {
            EntryType::File => {
                let blob_hash_hex = bytes_to_hex(&entry.hash);
                result.insert(path, blob_hash_hex);
            }
            EntryType::Dir => {
                let subtree_hash_hex = bytes_to_hex(&entry.hash);
                collect_tree_files(tree_store, &subtree_hash_hex, &path, result)?;
            }
            EntryType::Symlink => {
                // Symlinks are not restored (consistent with scanner skipping them)
            }
        }
    }
    Ok(())
}

/// Scan the current workspace to get a mapping of (relative_path -> content_hash_hex).
///
/// This uses the scanner to discover files, then hashes each file to get the current
/// content hash for comparison with the target snapshot state.
fn scan_current_state(
    workspace_root: &Path,
    cached_entries: &HashMap<String, crate::index::FileEntry>,
) -> Result<BTreeMap<String, CurrentFileState>> {
    let scanned = crate::scanner::scan_workspace(workspace_root, None)?;
    let mut state = BTreeMap::new();
    let mut stale_files = Vec::new();

    for file in scanned {
        if let Some(hash_hex) = cached_hash_hex(&file, cached_entries) {
            state.insert(file.relative_path.clone(), CurrentFileState { hash_hex });
        } else {
            stale_files.push(file);
        }
    }

    for (file, hash_hex) in hash_scanned_files(stale_files)? {
        state.insert(file.relative_path.clone(), CurrentFileState { hash_hex });
    }
    Ok(state)
}

fn restore_files(
    workspace_root: &Path,
    restore_tasks: &[(String, String)],
    blob_store: &BlobStore,
    has_loose_objects: bool,
    pack_set: &PackSet,
    progress: &ProgressCallback,
    progress_counter: &AtomicU64,
    restore_total: u64,
) -> Result<Vec<String>> {
    if restore_tasks.is_empty() {
        return Ok(Vec::new());
    }

    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(restore_tasks.len());
    if worker_count <= 1 {
        let mut restored = Vec::with_capacity(restore_tasks.len());
        for (path, blob_hash_hex) in restore_tasks {
            let content = if has_loose_objects && blob_store.exists(blob_hash_hex) {
                blob_store.read(blob_hash_hex)?
            } else {
                read_object_from_pack_set(pack_set, blob_hash_hex)?
            };
            let file_path = workspace_root.join(path);
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, &content)?;
            let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
            emit(
                progress,
                ProgressEvent::RestoreFile {
                    completed,
                    total: restore_total,
                },
            );
            restored.push(path.clone());
        }
        return Ok(restored);
    }

    let chunk_size = restore_tasks.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let workers: Vec<_> = restore_tasks
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || -> Result<Vec<String>> {
                    let mut restored = Vec::with_capacity(chunk.len());
                    for (path, blob_hash_hex) in chunk {
                        let content = if has_loose_objects && blob_store.exists(blob_hash_hex) {
                            blob_store.read(blob_hash_hex)?
                        } else {
                            read_object_from_pack_set(pack_set, blob_hash_hex)?
                        };
                        let file_path = workspace_root.join(path);
                        if let Some(parent) = file_path.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(&file_path, &content)?;
                        let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
                        emit(
                            progress,
                            ProgressEvent::RestoreFile {
                                completed,
                                total: restore_total,
                            },
                        );
                        restored.push(path.clone());
                    }
                    Ok(restored)
                })
            })
            .collect();

        let mut restored_paths = Vec::with_capacity(restore_tasks.len());
        for worker in workers {
            let chunk = worker
                .join()
                .map_err(|_| ChkpttError::Other("restore worker thread panicked".into()))??;
            restored_paths.extend(chunk);
        }
        Ok(restored_paths)
    })
}

fn diff_restore_states(
    target_state: &BTreeMap<String, String>,
    current_state: &BTreeMap<String, CurrentFileState>,
) -> RestoreDiff {
    let mut files_to_add = Vec::new();
    let mut files_to_change = Vec::new();
    let mut files_to_remove = Vec::new();
    let mut files_unchanged = 0;

    let mut target_iter = target_state.iter().peekable();
    let mut current_iter = current_state.iter().peekable();

    loop {
        match (target_iter.peek(), current_iter.peek()) {
            (Some((target_path, target_hash)), Some((current_path, current_file))) => {
                match target_path.cmp(current_path) {
                    std::cmp::Ordering::Less => {
                        files_to_add.push((*target_path).clone());
                        target_iter.next();
                    }
                    std::cmp::Ordering::Greater => {
                        files_to_remove.push((*current_path).clone());
                        current_iter.next();
                    }
                    std::cmp::Ordering::Equal => {
                        if target_hash.as_str() != current_file.hash_hex {
                            files_to_change.push((*target_path).clone());
                        } else {
                            files_unchanged += 1;
                        }
                        target_iter.next();
                        current_iter.next();
                    }
                }
            }
            (Some((target_path, _)), None) => {
                files_to_add.push((*target_path).clone());
                target_iter.next();
            }
            (None, Some((current_path, _))) => {
                files_to_remove.push((*current_path).clone());
                current_iter.next();
            }
            (None, None) => break,
        }
    }

    RestoreDiff {
        files_to_add,
        files_to_change,
        files_to_remove,
        files_unchanged,
    }
}

/// Restore workspace to a snapshot state.
///
/// This is the main restore function that:
/// 1. Resolves the snapshot ID ("latest" or prefix match)
/// 2. Loads the snapshot and reconstructs the target file state from the tree
/// 3. Compares target state vs current workspace state
/// 4. Either reports what would change (dry_run) or performs the actual restore
pub fn restore(
    workspace_root: &Path,
    snapshot_id: &str,
    options: RestoreOptions,
) -> Result<RestoreResult> {
    // 1. Compute project_id, create StoreLayout
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    // 2. Acquire project lock
    let _lock = ProjectLock::acquire(&layout.locks_dir())?;

    // 3. Resolve snapshot ID
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    let resolved_snapshot = if snapshot_id == "latest" {
        snapshot_store
            .latest()?
            .ok_or_else(|| ChkpttError::SnapshotNotFound("latest (no snapshots exist)".into()))?
    } else {
        // Try exact match first
        match snapshot_store.load(snapshot_id) {
            Ok(snap) => snap,
            Err(ChkpttError::SnapshotNotFound(_)) => {
                // Try prefix match
                let all_ids = snapshot_store.all_ids()?;
                let matches: Vec<_> = all_ids
                    .iter()
                    .filter(|id| id.starts_with(snapshot_id))
                    .collect();
                match matches.len() {
                    0 => {
                        return Err(ChkpttError::SnapshotNotFound(snapshot_id.to_string()));
                    }
                    1 => snapshot_store.load(matches[0])?,
                    _ => {
                        return Err(ChkpttError::Other(format!(
                            "Ambiguous snapshot prefix '{}': matches {} snapshots",
                            snapshot_id,
                            matches.len()
                        )));
                    }
                }
            }
            Err(e) => return Err(e),
        }
    };

    let resolved_id = resolved_snapshot.id.clone();

    // 4. Load snapshot's tree to get target state (path -> blob_hash_hex)
    let tree_store = TreeStore::new(layout.trees_dir());
    let root_tree_hash_hex = bytes_to_hex(&resolved_snapshot.root_tree_hash);
    let mut target_state: BTreeMap<String, String> = BTreeMap::new();
    collect_tree_files(&tree_store, &root_tree_hash_hex, "", &mut target_state)?;

    // 5. Scan current workspace to get current state (path -> content_hash_hex)
    let mut index = FileIndex::open(layout.index_path())?;
    let cached_entries = index.entries_by_path()?;
    let current_state = scan_current_state(workspace_root, &cached_entries)?;
    emit(
        &options.progress,
        ProgressEvent::ScanCurrentComplete {
            file_count: current_state.len() as u64,
        },
    );

    // 6. Compare target state vs current state
    let diff = diff_restore_states(&target_state, &current_state);
    let files_to_add = diff.files_to_add;
    let files_to_change = diff.files_to_change;
    let files_to_remove = diff.files_to_remove;
    let files_unchanged = diff.files_unchanged;

    let result = RestoreResult {
        snapshot_id: resolved_id.clone(),
        files_added: files_to_add.len() as u64,
        files_changed: files_to_change.len() as u64,
        files_removed: files_to_remove.len() as u64,
        files_unchanged,
    };

    // 7. If dry_run, return result without modifying workspace
    if options.dry_run {
        return Ok(result);
    }

    // 8. Perform actual restore
    let blob_store = BlobStore::new(layout.objects_dir());
    let has_loose_objects = blob_store_has_loose_objects(&layout.objects_dir())?;
    let packs_dir = layout.packs_dir();
    let pack_set = PackSet::open_all(&packs_dir)?;

    let restore_total = (files_to_add.len() + files_to_change.len() + files_to_remove.len()) as u64;
    emit(
        &options.progress,
        ProgressEvent::RestoreStart {
            add: files_to_add.len() as u64,
            change: files_to_change.len() as u64,
            remove: files_to_remove.len() as u64,
        },
    );

    // 8a. Restore files that need to be added or changed (parallel)
    let restore_tasks: Vec<(String, String)> = files_to_add
        .iter()
        .chain(files_to_change.iter())
        .map(|path| {
            (
                path.clone(),
                target_state
                    .get(path)
                    .expect("target hash missing for restore task")
                    .clone(),
            )
        })
        .collect();
    let restore_progress = AtomicU64::new(0);
    let restored_paths = restore_files(
        workspace_root,
        &restore_tasks,
        &blob_store,
        has_loose_objects,
        &pack_set,
        &options.progress,
        &restore_progress,
        restore_total,
    )?;

    // 8b. Remove files that are not in the target snapshot
    for path in &files_to_remove {
        let file_path = workspace_root.join(path);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
        let completed = restore_progress.fetch_add(1, Ordering::Relaxed) + 1;
        emit(
            &options.progress,
            ProgressEvent::RestoreFile {
                completed,
                total: restore_total,
            },
        );
    }

    // 8c. Clean up empty directories
    cleanup_empty_dirs(workspace_root)?;

    let file_entries = restored_index_entries(workspace_root, &restored_paths, &target_state)?;
    index.apply_changes(&files_to_remove, &file_entries)?;

    Ok(result)
}

/// Convert a 64-char hex string to a [u8; 32] array.
fn hex_to_bytes(hex: &str) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        if let Ok(b) = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
            bytes[i] = b;
        }
    }
    bytes
}

fn restored_index_entries(
    workspace_root: &Path,
    restored_paths: &[String],
    target_state: &BTreeMap<String, String>,
) -> Result<Vec<crate::index::FileEntry>> {
    let mut file_entries = Vec::with_capacity(restored_paths.len());
    for path in restored_paths {
        let absolute_path = workspace_root.join(path);
        let metadata = std::fs::metadata(&absolute_path)?;
        let hash_hex = target_state.get(path).ok_or_else(|| {
            ChkpttError::RestoreFailed(format!("Missing target hash for {}", path))
        })?;
        let scanned = scanned_file_from_metadata(path.clone(), absolute_path, &metadata);

        file_entries.push(crate::index::FileEntry {
            path: scanned.relative_path,
            blob_hash: hex_to_bytes(hash_hex),
            size: scanned.size,
            mtime_secs: scanned.mtime_secs,
            mtime_nanos: scanned.mtime_nanos,
            inode: scanned.inode,
            mode: scanned.mode,
        });
    }
    Ok(file_entries)
}

fn cached_hash_hex(
    file: &ScannedFile,
    cached_entries: &HashMap<String, crate::index::FileEntry>,
) -> Option<String> {
    let cached = cached_entries.get(&file.relative_path)?;
    if cached.mtime_secs == file.mtime_secs
        && cached.mtime_nanos == file.mtime_nanos
        && cached.size == file.size
        && cached.inode == file.inode
    {
        Some(bytes_to_hex(&cached.blob_hash))
    } else {
        None
    }
}

fn hash_scanned_files(scanned_files: Vec<ScannedFile>) -> Result<Vec<(ScannedFile, String)>> {
    if scanned_files.is_empty() {
        return Ok(Vec::new());
    }
    let mut scanned_files = scanned_files;
    sort_scanned_for_locality(&mut scanned_files);

    let worker_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(scanned_files.len());
    if worker_count <= 1 {
        return scanned_files
            .into_iter()
            .map(|file| Ok((file.clone(), hash_file(&file.absolute_path)?)))
            .collect();
    }

    let chunk_size = scanned_files.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for chunk in scanned_files.chunks(chunk_size) {
            workers.push(scope.spawn(move || -> Result<Vec<(ScannedFile, String)>> {
                chunk
                    .iter()
                    .map(|file| Ok((file.clone(), hash_file(&file.absolute_path)?)))
                    .collect()
            }));
        }

        let mut hashed = Vec::with_capacity(scanned_files.len());
        for worker in workers {
            let chunk = worker
                .join()
                .map_err(|_| ChkpttError::Other("restore worker thread panicked".into()))??;
            hashed.extend(chunk);
        }
        Ok(hashed)
    })
}

#[cfg(unix)]
fn scanned_file_from_metadata(
    relative_path: String,
    absolute_path: std::path::PathBuf,
    metadata: &std::fs::Metadata,
) -> ScannedFile {
    use std::os::unix::fs::MetadataExt;

    ScannedFile {
        relative_path,
        absolute_path,
        size: metadata.len(),
        mtime_secs: metadata.mtime(),
        mtime_nanos: metadata.mtime_nsec(),
        inode: Some(metadata.ino()),
        mode: metadata.mode(),
    }
}

#[cfg(not(unix))]
fn scanned_file_from_metadata(
    relative_path: String,
    absolute_path: std::path::PathBuf,
    metadata: &std::fs::Metadata,
) -> ScannedFile {
    use std::time::UNIX_EPOCH;

    let (mtime_secs, mtime_nanos) = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| (duration.as_secs() as i64, duration.subsec_nanos() as i64))
        .unwrap_or((0, 0));

    ScannedFile {
        relative_path,
        absolute_path,
        size: metadata.len(),
        mtime_secs,
        mtime_nanos,
        inode: None,
        mode: 0o644,
    }
}

/// Recursively remove empty directories under root (but not root itself).
fn cleanup_empty_dirs(root: &Path) -> Result<()> {
    cleanup_empty_dirs_recursive(root, root)
}

fn blob_store_has_loose_objects(objects_dir: &Path) -> Result<bool> {
    if !objects_dir.exists() {
        return Ok(false);
    }
    for prefix_entry in std::fs::read_dir(objects_dir)? {
        let prefix_entry = prefix_entry?;
        if !prefix_entry.file_type()?.is_dir() {
            continue;
        }
        for object_entry in std::fs::read_dir(prefix_entry.path())? {
            let object_entry = object_entry?;
            if object_entry.file_type()?.is_file()
                && !object_entry.file_name().to_string_lossy().ends_with(".tmp")
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn cleanup_empty_dirs_recursive(root: &Path, dir: &Path) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    // First recurse into subdirectories
    let entries: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();

    for entry in &entries {
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            cleanup_empty_dirs_recursive(root, &entry.path())?;
        }
    }

    // After recursing, check if directory is now empty (and it's not the root)
    if dir != root {
        let remaining: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
        if remaining.is_empty() {
            std::fs::remove_dir(dir)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_blob_store_has_loose_objects_false_for_empty_store() {
        let dir = TempDir::new().unwrap();
        let objects_dir = dir.path().join("objects");
        std::fs::create_dir_all(&objects_dir).unwrap();
        assert!(!blob_store_has_loose_objects(&objects_dir).unwrap());
    }

    #[test]
    fn test_blob_store_has_loose_objects_true_when_file_exists() {
        let dir = TempDir::new().unwrap();
        let objects_dir = dir.path().join("objects");
        std::fs::create_dir_all(objects_dir.join("aa")).unwrap();
        std::fs::write(objects_dir.join("aa").join("bb"), b"data").unwrap();
        assert!(blob_store_has_loose_objects(&objects_dir).unwrap());
    }

    #[test]
    fn test_diff_restore_states_classifies_paths() {
        let target_state = BTreeMap::from([
            ("a.txt".to_string(), "hash-a".to_string()),
            ("b.txt".to_string(), "hash-b-target".to_string()),
            ("c.txt".to_string(), "hash-c".to_string()),
        ]);
        let current_state = BTreeMap::from([
            (
                "b.txt".to_string(),
                CurrentFileState {
                    hash_hex: "hash-b-current".to_string(),
                },
            ),
            (
                "c.txt".to_string(),
                CurrentFileState {
                    hash_hex: "hash-c".to_string(),
                },
            ),
            (
                "d.txt".to_string(),
                CurrentFileState {
                    hash_hex: "hash-d".to_string(),
                },
            ),
        ]);

        let diff = diff_restore_states(&target_state, &current_state);
        assert_eq!(diff.files_to_add, vec!["a.txt".to_string()]);
        assert_eq!(diff.files_to_change, vec!["b.txt".to_string()]);
        assert_eq!(diff.files_to_remove, vec!["d.txt".to_string()]);
        assert_eq!(diff.files_unchanged, 1);
    }

    #[test]
    fn test_diff_restore_states_handles_empty_inputs() {
        let target_state: BTreeMap<String, String> = BTreeMap::new();
        let current_state: BTreeMap<String, CurrentFileState> = BTreeMap::new();
        let diff = diff_restore_states(&target_state, &current_state);

        assert!(diff.files_to_add.is_empty());
        assert!(diff.files_to_change.is_empty());
        assert!(diff.files_to_remove.is_empty());
        assert_eq!(diff.files_unchanged, 0);
    }
}
