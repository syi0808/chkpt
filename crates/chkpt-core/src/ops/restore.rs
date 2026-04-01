use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::FileIndex;
use crate::ops::io_order::sort_scanned_for_locality;
use crate::ops::lock::ProjectLock;
use crate::scanner::ScannedFile;
use crate::store::blob::{hash_path, BlobStore};
use crate::store::catalog::{ManifestEntry, MetadataCatalog};
use crate::store::pack::{PackLocation, PackSet};
use crate::store::snapshot::SnapshotStore;
use crate::store::tree::{EntryType, TreeStore};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufWriter, Write};
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
    is_symlink: bool,
}

struct TargetFileState {
    hash_hex: String,
    is_symlink: bool,
}

struct RestoreDiff {
    files_to_add: Vec<String>,
    files_to_change: Vec<String>,
    files_to_remove: Vec<String>,
    files_unchanged: u64,
}

#[derive(Debug, Clone, Copy)]
enum RestoreSource {
    Loose,
    Packed(PackLocation),
}

#[derive(Debug, Clone)]
struct RestoreTask {
    path: String,
    hash_hex: String,
    is_symlink: bool,
    source: RestoreSource,
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
    result: &mut BTreeMap<String, TargetFileState>,
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
                result.insert(
                    path,
                    TargetFileState {
                        hash_hex: blob_hash_hex,
                        is_symlink: false,
                    },
                );
            }
            EntryType::Dir => {
                let subtree_hash_hex = bytes_to_hex(&entry.hash);
                collect_tree_files(tree_store, &subtree_hash_hex, &path, result)?;
            }
            EntryType::Symlink => {
                let blob_hash_hex = bytes_to_hex(&entry.hash);
                result.insert(
                    path,
                    TargetFileState {
                        hash_hex: blob_hash_hex,
                        is_symlink: true,
                    },
                );
            }
        }
    }
    Ok(())
}

fn target_state_from_manifest(manifest: &[ManifestEntry]) -> BTreeMap<String, TargetFileState> {
    manifest
        .iter()
        .map(|entry| {
            (
                entry.path.clone(),
                TargetFileState {
                    hash_hex: bytes_to_hex(&entry.blob_hash),
                    is_symlink: mode_is_symlink(entry.mode),
                },
            )
        })
        .collect()
}

/// Scan the current workspace to get a mapping of (relative_path -> content_hash_hex).
///
/// This uses the scanner to discover files, then hashes each file to get the current
/// content hash for comparison with the target snapshot state.
fn scan_current_state(
    workspace_root: &Path,
    cached_entries: &HashMap<String, crate::index::FileEntry>,
    include_deps: bool,
) -> Result<BTreeMap<String, CurrentFileState>> {
    let scanned = crate::scanner::scan_workspace_with_options(workspace_root, None, include_deps)?;
    let mut state = BTreeMap::new();
    let mut stale_files = Vec::new();

    for file in scanned {
        if let Some(hash_hex) = cached_hash_hex(&file, cached_entries) {
            state.insert(
                file.relative_path.clone(),
                CurrentFileState {
                    hash_hex,
                    is_symlink: file.is_symlink,
                },
            );
        } else {
            stale_files.push(file);
        }
    }

    for (file, hash_hex) in hash_scanned_files(stale_files)? {
        state.insert(
            file.relative_path.clone(),
            CurrentFileState {
                hash_hex,
                is_symlink: file.is_symlink,
            },
        );
    }
    Ok(state)
}

fn restore_files(
    workspace_root: &Path,
    restore_tasks: &[RestoreTask],
    blob_store: &BlobStore,
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
        for task in restore_tasks {
            restore_file(workspace_root, task, blob_store, pack_set)?;
            let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
            emit(
                progress,
                ProgressEvent::RestoreFile {
                    completed,
                    total: restore_total,
                },
            );
            restored.push(task.path.clone());
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
                    for task in chunk {
                        restore_file(workspace_root, task, blob_store, pack_set)?;
                        let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
                        emit(
                            progress,
                            ProgressEvent::RestoreFile {
                                completed,
                                total: restore_total,
                            },
                        );
                        restored.push(task.path.clone());
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

fn restore_file(
    workspace_root: &Path,
    task: &RestoreTask,
    blob_store: &BlobStore,
    pack_set: &PackSet,
) -> Result<()> {
    let file_path = workspace_root.join(&task.path);
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if let Ok(metadata) = std::fs::symlink_metadata(&file_path) {
        if metadata.file_type().is_symlink() || task.is_symlink {
            std::fs::remove_file(&file_path)?;
        }
    }

    match task.source {
        RestoreSource::Loose => {
            let content = blob_store.read(&task.hash_hex)?;
            if task.is_symlink {
                restore_symlink(&file_path, &content)?;
            } else {
                std::fs::write(&file_path, &content)?;
            }
        }
        RestoreSource::Packed(location) => {
            if task.is_symlink {
                let content = pack_set.read(&task.hash_hex)?;
                restore_symlink(&file_path, &content)?;
            } else {
                let file = std::fs::File::create(&file_path)?;
                let mut writer = BufWriter::with_capacity(256 * 1024, file);
                pack_set.copy_to_writer(&location, &mut writer)?;
                writer.flush()?;
            }
        }
    }

    Ok(())
}

#[cfg(unix)]
fn restore_symlink(path: &Path, target_bytes: &[u8]) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let target = std::ffi::OsStr::from_bytes(target_bytes);
    std::os::unix::fs::symlink(target, path)?;
    Ok(())
}

#[cfg(not(unix))]
fn restore_symlink(_path: &Path, _target_bytes: &[u8]) -> Result<()> {
    Err(ChkpttError::RestoreFailed(
        "symlink restore is only supported on unix platforms".into(),
    ))
}

fn build_restore_tasks(
    files_to_add: &[String],
    files_to_change: &[String],
    target_state: &BTreeMap<String, TargetFileState>,
    blob_store: &BlobStore,
    has_loose_objects: bool,
    pack_set: &PackSet,
) -> Result<Vec<RestoreTask>> {
    let mut tasks = Vec::with_capacity(files_to_add.len() + files_to_change.len());

    for path in files_to_add.iter().chain(files_to_change.iter()) {
        let target = target_state
            .get(path)
            .expect("target hash missing for restore task");
        let hash_hex = target.hash_hex.clone();

        let source = if has_loose_objects && blob_store.exists(&hash_hex) {
            RestoreSource::Loose
        } else {
            RestoreSource::Packed(
                pack_set
                    .locate(&hash_hex)
                    .ok_or_else(|| ChkpttError::ObjectNotFound(hash_hex.clone()))?,
            )
        };

        tasks.push(RestoreTask {
            path: path.clone(),
            hash_hex,
            is_symlink: target.is_symlink,
            source,
        });
    }

    tasks.sort_unstable_by(|left, right| match (&left.source, &right.source) {
        (RestoreSource::Packed(left_location), RestoreSource::Packed(right_location)) => (
            left_location.reader_index,
            left_location.offset,
            left.path.as_str(),
        )
            .cmp(&(
                right_location.reader_index,
                right_location.offset,
                right.path.as_str(),
            )),
        (RestoreSource::Packed(_), RestoreSource::Loose) => std::cmp::Ordering::Less,
        (RestoreSource::Loose, RestoreSource::Packed(_)) => std::cmp::Ordering::Greater,
        (RestoreSource::Loose, RestoreSource::Loose) => left.path.cmp(&right.path),
    });
    Ok(tasks)
}

fn diff_restore_states(
    target_state: &BTreeMap<String, TargetFileState>,
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
            (Some((target_path, target_file)), Some((current_path, current_file))) => {
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
                        if target_file.hash_hex != current_file.hash_hex
                            || target_file.is_symlink != current_file.is_symlink
                        {
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
    let catalog = MetadataCatalog::open(layout.catalog_path())?;

    // 3. Resolve snapshot ID
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    let resolved_id = resolve_snapshot_id(&catalog, &snapshot_store, snapshot_id)?;

    // 4. Load snapshot's tree to get target state (path -> blob_hash_hex)
    let manifest = catalog.snapshot_manifest(&resolved_id)?;
    let target_state = if manifest.is_empty() {
        let resolved_snapshot = snapshot_store.load(&resolved_id)?;
        let tree_store = TreeStore::new(layout.trees_dir());
        let root_tree_hash_hex = bytes_to_hex(&resolved_snapshot.root_tree_hash);
        let mut state = BTreeMap::new();
        collect_tree_files(&tree_store, &root_tree_hash_hex, "", &mut state)?;
        state
    } else {
        target_state_from_manifest(&manifest)
    };
    let target_includes_deps = target_state.keys().any(|path| path_contains_dependency_dir(path));

    // 5. Scan current workspace to get current state (path -> content_hash_hex)
    let mut index = FileIndex::open(layout.index_path())?;
    let cached_entries = index.entries();
    let current_state = scan_current_state(workspace_root, &cached_entries, target_includes_deps)?;
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
    let restore_tasks = build_restore_tasks(
        &files_to_add,
        &files_to_change,
        &target_state,
        &blob_store,
        has_loose_objects,
        &pack_set,
    )?;
    let restore_progress = AtomicU64::new(0);
    let restored_paths = restore_files(
        workspace_root,
        &restore_tasks,
        &blob_store,
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

    // 8c. Clean up empty directories affected by removed files only.
    cleanup_removed_file_parents(workspace_root, &files_to_remove)?;

    let file_entries = restored_index_entries(workspace_root, &restored_paths, &target_state)?;
    index.apply_changes(&files_to_remove, &file_entries)?;

    Ok(result)
}

fn resolve_snapshot_id(
    catalog: &MetadataCatalog,
    snapshot_store: &SnapshotStore,
    snapshot_ref: &str,
) -> Result<String> {
    match catalog.resolve_snapshot_ref(snapshot_ref) {
        Ok(snapshot) => Ok(snapshot.id),
        Err(ChkpttError::SnapshotNotFound(_)) => {
            let resolved_snapshot = if snapshot_ref == "latest" {
                snapshot_store.latest()?.ok_or_else(|| {
                    ChkpttError::SnapshotNotFound("latest (no snapshots exist)".into())
                })?
            } else {
                match snapshot_store.load(snapshot_ref) {
                    Ok(snapshot) => snapshot,
                    Err(ChkpttError::SnapshotNotFound(_)) => {
                        let all_ids = snapshot_store.all_ids()?;
                        let matches: Vec<_> = all_ids
                            .iter()
                            .filter(|id| id.starts_with(snapshot_ref))
                            .collect();
                        match matches.len() {
                            0 => {
                                return Err(ChkpttError::SnapshotNotFound(snapshot_ref.to_string()));
                            }
                            1 => snapshot_store.load(matches[0])?,
                            _ => {
                                return Err(ChkpttError::Other(format!(
                                    "Ambiguous snapshot prefix '{}': matches {} snapshots",
                                    snapshot_ref,
                                    matches.len()
                                )));
                            }
                        }
                    }
                    Err(error) => return Err(error),
                }
            };
            Ok(resolved_snapshot.id)
        }
        Err(error) => Err(error),
    }
}

fn path_contains_dependency_dir(relative_path: &str) -> bool {
    relative_path.split('/').any(|component| {
        matches!(
            component,
            "node_modules"
                | ".venv"
                | "venv"
                | "__pypackages__"
                | ".tox"
                | ".nox"
                | ".gradle"
                | ".m2"
        )
    })
}

fn mode_is_symlink(mode: u32) -> bool {
    #[cfg(unix)]
    {
        (mode & 0o170000) == 0o120000
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        false
    }
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
    target_state: &BTreeMap<String, TargetFileState>,
) -> Result<Vec<crate::index::FileEntry>> {
    let mut file_entries = Vec::with_capacity(restored_paths.len());
    for path in restored_paths {
        let absolute_path = workspace_root.join(path);
        let metadata = std::fs::symlink_metadata(&absolute_path)?;
        let target = target_state.get(path).ok_or_else(|| {
            ChkpttError::RestoreFailed(format!("Missing target hash for {}", path))
        })?;
        let scanned = scanned_file_from_metadata(path.clone(), absolute_path, &metadata);

        file_entries.push(crate::index::FileEntry {
            path: scanned.relative_path,
            blob_hash: hex_to_bytes(&target.hash_hex),
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
        && cached.mode == file.mode
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
            .map(|file| {
                Ok((
                    file.clone(),
                    hash_path(&file.absolute_path, file.is_symlink)?,
                ))
            })
            .collect();
    }

    let chunk_size = scanned_files.len().div_ceil(worker_count);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for chunk in scanned_files.chunks(chunk_size) {
            workers.push(scope.spawn(move || -> Result<Vec<(ScannedFile, String)>> {
                chunk
                    .iter()
                    .map(|file| {
                        Ok((
                            file.clone(),
                            hash_path(&file.absolute_path, file.is_symlink)?,
                        ))
                    })
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
        device: Some(metadata.dev()),
        inode: Some(metadata.ino()),
        mode: metadata.mode(),
        is_symlink: metadata.file_type().is_symlink(),
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
        device: None,
        inode: None,
        mode: 0o644,
        is_symlink: metadata.file_type().is_symlink(),
    }
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

fn cleanup_removed_file_parents(root: &Path, removed_paths: &[String]) -> Result<()> {
    if removed_paths.is_empty() {
        return Ok(());
    }

    let mut candidates = HashSet::new();
    for removed_path in removed_paths {
        let mut current = root.join(removed_path);
        while let Some(parent) = current.parent() {
            if parent == root {
                candidates.insert(parent.to_path_buf());
                break;
            }
            if !parent.starts_with(root) {
                break;
            }
            candidates.insert(parent.to_path_buf());
            current = parent.to_path_buf();
        }
    }

    let mut candidates: Vec<_> = candidates.into_iter().filter(|dir| dir != root).collect();
    candidates.sort_unstable_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.cmp(right))
    });

    for dir in candidates {
        match std::fs::remove_dir(&dir) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => return Err(error.into()),
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
    fn test_cleanup_removed_file_parents_removes_only_empty_ancestor_chain() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let empty_leaf = root.join("a/b/c");
        std::fs::create_dir_all(&empty_leaf).unwrap();
        std::fs::write(empty_leaf.join("gone.txt"), b"gone").unwrap();
        std::fs::remove_file(empty_leaf.join("gone.txt")).unwrap();

        let non_empty_leaf = root.join("a/keep");
        std::fs::create_dir_all(&non_empty_leaf).unwrap();
        std::fs::write(non_empty_leaf.join("keep.txt"), b"keep").unwrap();

        cleanup_removed_file_parents(root, &[String::from("a/b/c/gone.txt")]).unwrap();

        assert!(!root.join("a/b/c").exists());
        assert!(!root.join("a/b").exists());
        assert!(root.join("a").exists());
        assert!(root.join("a/keep/keep.txt").exists());
    }

    #[test]
    fn test_cleanup_removed_file_parents_skips_non_empty_directories() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let shared = root.join("shared");
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(shared.join("still-here.txt"), b"keep").unwrap();

        cleanup_removed_file_parents(root, &[String::from("shared/gone.txt")]).unwrap();

        assert!(root.join("shared").exists());
        assert!(root.join("shared/still-here.txt").exists());
    }

    #[test]
    fn test_diff_restore_states_classifies_paths() {
        let target_state = BTreeMap::from([
            (
                "a.txt".to_string(),
                TargetFileState {
                    hash_hex: "hash-a".to_string(),
                    is_symlink: false,
                },
            ),
            (
                "b.txt".to_string(),
                TargetFileState {
                    hash_hex: "hash-b-target".to_string(),
                    is_symlink: true,
                },
            ),
            (
                "c.txt".to_string(),
                TargetFileState {
                    hash_hex: "hash-c".to_string(),
                    is_symlink: false,
                },
            ),
        ]);
        let current_state = BTreeMap::from([
            (
                "b.txt".to_string(),
                CurrentFileState {
                    hash_hex: "hash-b-current".to_string(),
                    is_symlink: false,
                },
            ),
            (
                "c.txt".to_string(),
                CurrentFileState {
                    hash_hex: "hash-c".to_string(),
                    is_symlink: false,
                },
            ),
            (
                "d.txt".to_string(),
                CurrentFileState {
                    hash_hex: "hash-d".to_string(),
                    is_symlink: false,
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
        let target_state: BTreeMap<String, TargetFileState> = BTreeMap::new();
        let current_state: BTreeMap<String, CurrentFileState> = BTreeMap::new();
        let diff = diff_restore_states(&target_state, &current_state);

        assert!(diff.files_to_add.is_empty());
        assert!(diff.files_to_change.is_empty());
        assert!(diff.files_to_remove.is_empty());
        assert_eq!(diff.files_unchanged, 0);
    }

    #[test]
    fn test_diff_restore_states_detects_type_changes() {
        let target_state = BTreeMap::from([(
            "link".to_string(),
            TargetFileState {
                hash_hex: "same-hash".to_string(),
                is_symlink: true,
            },
        )]);
        let current_state = BTreeMap::from([(
            "link".to_string(),
            CurrentFileState {
                hash_hex: "same-hash".to_string(),
                is_symlink: false,
            },
        )]);

        let diff = diff_restore_states(&target_state, &current_state);
        assert_eq!(diff.files_to_change, vec!["link".to_string()]);
    }
}
