use crate::config::{project_id_from_path, StoreLayout};
use crate::error::{ChkpttError, Result};
use crate::index::{FileEntry, FileIndex};
use crate::ops::io_order::sort_scanned_refs_for_locality;
use crate::ops::lock::ProjectLock;
use crate::scanner::ScannedFile;
use crate::store::blob::{hex_to_bytes, read_path_bytes};
use crate::store::catalog::{BlobLocation, CatalogSnapshot, ManifestEntry, MetadataCatalog};
use crate::store::pack::{PackSet, PackWriter};
use crate::store::snapshot::{Snapshot, SnapshotStats};
use crate::store::tree::{EntryType, TreeEntry, TreeStore};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use crate::ops::progress::{emit, ProgressCallback, ProgressEvent};

#[derive(Default)]
pub struct SaveOptions {
    pub message: Option<String>,
    pub include_deps: bool,
    pub progress: ProgressCallback,
}

#[derive(Debug)]
pub struct SaveResult {
    pub snapshot_id: String,
    pub stats: SnapshotStats,
}

/// Represents a file with its blob hash after processing.
struct ProcessedFile {
    relative_path: String,
    blob_hash_bytes: [u8; 32],
    size: u64,
    mode: u32,
    entry_type: EntryType,
}

struct PreparedFile {
    relative_path: String,
    blob_hash_hex: String,
    blob_hash_bytes: [u8; 32],
    /// None if this hash was already seen (duplicate content — compression skipped)
    compressed: Option<Vec<u8>>,
    size: u64,
    mode: u32,
    mtime_secs: i64,
    mtime_nanos: i64,
    inode: Option<u64>,
    entry_type: EntryType,
}

struct NewBlobRecord {
    blob_hash: [u8; 32],
    size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct HardlinkKey {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone)]
struct HardlinkPrepared {
    blob_hash_hex: String,
    blob_hash_bytes: [u8; 32],
}

const SEEN_HASH_SHARDS: usize = 64;
const PREPARED_FILE_PIPELINE_SLOTS: usize = 64;
struct ShardedSeenHashes {
    shards: Vec<Mutex<HashSet<[u8; 32]>>>,
}

impl ShardedSeenHashes {
    fn new(expected_entries: usize) -> Self {
        let shard_count = SEEN_HASH_SHARDS.max(1);
        let per_shard_capacity = expected_entries.div_ceil(shard_count).max(1);
        let mut shards = Vec::with_capacity(shard_count);
        for _ in 0..shard_count {
            shards.push(Mutex::new(HashSet::with_capacity(per_shard_capacity)));
        }
        Self { shards }
    }

    #[inline]
    fn insert(&self, hash: [u8; 32]) -> bool {
        let shard_index = ((hash[0] as usize) << 8 | hash[1] as usize) % self.shards.len();
        let mut shard = self.shards[shard_index].lock().unwrap();
        shard.insert(hash)
    }
}

/// Save a checkpoint of the workspace.
///
/// This is the main orchestrating function that ties together scanning,
/// hashing, blob storage, tree building, and snapshot creation.
pub fn save(workspace_root: &Path, options: SaveOptions) -> Result<SaveResult> {
    // 1. Compute project_id from workspace path
    let project_id = project_id_from_path(workspace_root);

    // 2. Create StoreLayout, ensure directories exist
    let layout = StoreLayout::new(&project_id);
    layout.ensure_dirs()?;

    // 3. Acquire project lock
    let _lock = ProjectLock::acquire(&layout.locks_dir())?;
    let catalog = MetadataCatalog::open(layout.catalog_path())?;

    // 4. Scan workspace (respect .chkptignore)
    let scanned_files =
        crate::scanner::scan_workspace_with_options(workspace_root, None, options.include_deps)?;
    emit(
        &options.progress,
        ProgressEvent::ScanComplete {
            file_count: scanned_files.len() as u64,
        },
    );

    // 5. Open/create FileIndex
    let mut index = FileIndex::open(layout.index_path())?;
    let cached_entries = index.entries();

    // 6. Create blob store
    let packs_dir = layout.packs_dir();
    let has_pack_objects = store_has_pack_objects(&packs_dir)?;
    let pack_set = has_pack_objects
        .then(|| PackSet::open_all(&packs_dir))
        .transpose()?;
    let mut pack_writer = PackWriter::new(&packs_dir)?;

    // 7. Process each scanned file: check index, hash, store blob
    let mut processed_files = Vec::with_capacity(scanned_files.len());
    let mut files_to_prepare = Vec::new();
    let mut updated_entries = Vec::new();
    let mut blob_locations_to_record = Vec::new();
    let mut new_blob_records = Vec::new();
    let mut total_bytes: u64 = 0;
    let mut current_paths =
        (!cached_entries.is_empty()).then(|| HashSet::with_capacity(scanned_files.len()));

    for scanned in &scanned_files {
        if let Some(paths) = current_paths.as_mut() {
            paths.insert(scanned.relative_path.as_str());
        }

        if let Some(processed) =
            cached_processed_file(scanned, cached_entries.get(&scanned.relative_path))
        {
            total_bytes += processed.size;
            processed_files.push(processed);
        } else {
            files_to_prepare.push(scanned);
        }
    }
    let removed_paths = current_paths
        .map(|paths| {
            cached_entries
                .keys()
                .filter(|path| !paths.contains(path.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut new_objects: u64 = 0;

    // Sharded dedup set reduces lock contention during parallel hash+compress.
    let seen_hashes = Arc::new(ShardedSeenHashes::new(files_to_prepare.len()));

    // Prioritize on-disk locality to reduce random I/O during read+hash+compress.
    sort_scanned_refs_for_locality(&mut files_to_prepare);

    // Batch parallel: each thread processes a chunk independently, no channel overhead
    let total_to_process = files_to_prepare.len() as u64;
    emit(
        &options.progress,
        ProgressEvent::ProcessStart {
            total: total_to_process,
        },
    );
    let progress_counter = AtomicU64::new(0);
    process_prepared_files_streaming(
        &files_to_prepare,
        &seen_hashes,
        &options.progress,
        &progress_counter,
        total_to_process,
        |prepared| {
            let PreparedFile {
                relative_path,
                blob_hash_hex: _,
                blob_hash_bytes,
                compressed,
                size,
                mode,
                mtime_secs,
                mtime_nanos,
                inode,
                entry_type,
            } = prepared;

            total_bytes += size;
            if let Some(compressed) = compressed {
                let exists_in_pack = pack_set
                    .as_ref()
                    .is_some_and(|ps| ps.contains_bytes(&blob_hash_bytes));
                if !exists_in_pack {
                    pack_writer.add_pre_compressed_bytes(blob_hash_bytes, compressed)?;
                    new_blob_records.push(NewBlobRecord {
                        blob_hash: blob_hash_bytes,
                        size,
                    });
                    new_objects += 1;
                }
            }

            updated_entries.push(FileEntry {
                path: relative_path.clone(),
                blob_hash: blob_hash_bytes,
                size,
                mtime_secs,
                mtime_nanos,
                inode,
                mode,
            });
            processed_files.push(ProcessedFile {
                relative_path,
                blob_hash_bytes,
                size,
                mode,
                entry_type,
            });
            Ok(())
        },
    )?;
    let new_pack_hash = if !pack_writer.is_empty() {
        Some(pack_writer.finish()?)
    } else {
        None
    };
    if let Some(pack_hash) = new_pack_hash {
        for blob in &new_blob_records {
            blob_locations_to_record.push((
                blob.blob_hash,
                BlobLocation {
                    pack_hash: Some(pack_hash.clone()),
                    size: blob.size,
                },
            ));
        }
    }
    catalog.bulk_upsert_blob_locations(&blob_locations_to_record)?;
    emit(&options.progress, ProgressEvent::PackComplete);

    // 8. Build tree bottom-up (skip if nothing changed)
    let latest_catalog_snapshot = catalog.latest_snapshot()?;

    let root_tree_hash = if new_objects == 0
        && removed_paths.is_empty()
        && updated_entries.is_empty()
    {
        if let Some(ref snapshot) = latest_catalog_snapshot {
            // Nothing changed — reuse previous tree hash (skip build entirely)
            root_tree_hash_for_snapshot(&catalog, snapshot, &TreeStore::new(layout.trees_dir()))?
        } else {
            let tree_store = TreeStore::new(layout.trees_dir());
            let hex = build_tree(&processed_files, &tree_store)?;
            hex_to_bytes(&hex)?
        }
    } else {
        let tree_store = TreeStore::new(layout.trees_dir());
        let hex = build_tree(&processed_files, &tree_store)?;
        hex_to_bytes(&hex)?
    };

    // 9. Find latest snapshot for parent_snapshot_id (already fetched above)
    let parent_snapshot_id = latest_catalog_snapshot
        .as_ref()
        .map(|snapshot| snapshot.id.clone());

    // 10. Create Snapshot
    let stats = SnapshotStats {
        total_files: scanned_files.len() as u64,
        total_bytes,
        new_objects,
    };

    let snapshot = Snapshot::new(
        options.message,
        root_tree_hash,
        parent_snapshot_id,
        stats.clone(),
    );

    let snapshot_id = snapshot.id.clone();
    let catalog_snapshot = CatalogSnapshot {
        id: snapshot.id.clone(),
        created_at: snapshot.created_at,
        message: snapshot.message.clone(),
        parent_snapshot_id: snapshot.parent_snapshot_id.clone(),
        manifest_snapshot_id: None,
        root_tree_hash: Some(root_tree_hash),
        stats: stats.clone(),
    };
    let no_manifest_changes =
        new_objects == 0 && removed_paths.is_empty() && updated_entries.is_empty();

    // 11. Save snapshot
    if no_manifest_changes {
        let manifest_snapshot_id = latest_catalog_snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .manifest_snapshot_id
                    .as_deref()
                    .unwrap_or(snapshot.id.as_str())
            })
            .unwrap_or(snapshot.id.as_str());
        catalog.insert_snapshot_metadata_only(&catalog_snapshot, manifest_snapshot_id)?;
    } else {
        let mut manifest: Vec<ManifestEntry> = processed_files
            .iter()
            .map(|processed| ManifestEntry {
                path: processed.relative_path.clone(),
                blob_hash: processed.blob_hash_bytes,
                size: processed.size,
                mode: processed.mode,
            })
            .collect();
        manifest.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        catalog.insert_snapshot(&catalog_snapshot, &manifest)?;
    }

    // 12. Update only changed index entries and remove stale paths.
    index.apply_changes(&removed_paths, &updated_entries)?;

    // 13. Lock released automatically via drop

    // 14. Return SaveResult
    Ok(SaveResult { snapshot_id, stats })
}

#[cfg_attr(not(test), allow(dead_code))]
fn store_has_external_objects(objects_dir: &Path, packs_dir: &Path) -> Result<bool> {
    let _ = objects_dir;
    Ok(store_has_pack_objects(packs_dir)?)
}

fn store_has_pack_objects(packs_dir: &Path) -> Result<bool> {
    let entries = match std::fs::read_dir(packs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }

        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("pack-") && name.ends_with(".dat") {
            return Ok(true);
        }
    }

    Ok(false)
}

fn root_tree_hash_for_snapshot(
    catalog: &MetadataCatalog,
    snapshot: &CatalogSnapshot,
    tree_store: &TreeStore,
) -> Result<[u8; 32]> {
    if let Some(root_tree_hash) = snapshot.root_tree_hash {
        return Ok(root_tree_hash);
    }

    let manifest = catalog.snapshot_manifest(&snapshot.id)?;
    if manifest.is_empty() && snapshot.stats.total_files > 0 {
        return Err(ChkpttError::StoreCorrupted(format!(
            "snapshot '{}' is missing both manifest entries and root_tree_hash",
            snapshot.id
        )));
    }
    let root_tree_hex = build_tree(
        &manifest
            .into_iter()
            .map(|entry| ProcessedFile {
                relative_path: entry.path,
                blob_hash_bytes: entry.blob_hash,
                size: entry.size,
                mode: entry.mode,
                entry_type: if entry.mode & 0o170000 == 0o120000 {
                    EntryType::Symlink
                } else {
                    EntryType::File
                },
            })
            .collect::<Vec<_>>(),
        tree_store,
    )?;
    hex_to_bytes(&root_tree_hex)
}

fn cached_processed_file(
    scanned: &ScannedFile,
    cached: Option<&FileEntry>,
) -> Option<ProcessedFile> {
    if let Some(cached) = cached {
        if cached.mtime_secs == scanned.mtime_secs
            && cached.mtime_nanos == scanned.mtime_nanos
            && cached.size == scanned.size
            && cached.inode == scanned.inode
            && cached.mode == scanned.mode
        {
            return Some(ProcessedFile {
                relative_path: scanned.relative_path.clone(),
                blob_hash_bytes: cached.blob_hash,
                size: scanned.size,
                mode: scanned.mode,
                entry_type: if scanned.is_symlink {
                    EntryType::Symlink
                } else {
                    EntryType::File
                },
            });
        }
    }
    None
}

fn prepare_file(
    scanned: &ScannedFile,
    seen_hashes: &ShardedSeenHashes,
    compressor: &mut zstd::bulk::Compressor<'_>,
) -> Result<PreparedFile> {
    let content = read_path_bytes(&scanned.absolute_path, scanned.is_symlink)?;

    // Hash (BLAKE3 is ~6 GB/s, negligible cost)
    let hash = blake3::hash(&content);
    let blob_hash_bytes: [u8; 32] = *hash.as_bytes();

    // Only compress if this hash hasn't been seen yet (use raw bytes, no hex conversion)
    let is_new = seen_hashes.insert(blob_hash_bytes);

    let compressed = if is_new {
        Some(compress_with_worker_context(&content, compressor)?)
    } else {
        None
    };

    // Defer hex conversion to caller (only needed for pack writer)
    let blob_hash_hex = hash.to_hex().to_string();

    Ok(PreparedFile {
        relative_path: scanned.relative_path.clone(),
        blob_hash_hex,
        blob_hash_bytes,
        compressed,
        size: scanned.size,
        mode: scanned.mode,
        mtime_secs: scanned.mtime_secs,
        mtime_nanos: scanned.mtime_nanos,
        inode: scanned.inode,
        entry_type: if scanned.is_symlink {
            EntryType::Symlink
        } else {
            EntryType::File
        },
    })
}

fn hardlink_key(scanned: &ScannedFile) -> Option<HardlinkKey> {
    if scanned.is_symlink {
        return None;
    }

    Some(HardlinkKey {
        device: scanned.device?,
        inode: scanned.inode?,
    })
}

fn prepare_file_with_hardlink_cache(
    scanned: &ScannedFile,
    seen_hashes: &ShardedSeenHashes,
    compressor: &mut zstd::bulk::Compressor<'_>,
    hardlinks: &mut HashMap<HardlinkKey, HardlinkPrepared>,
) -> Result<PreparedFile> {
    if let Some(key) = hardlink_key(scanned) {
        if let Some(cached) = hardlinks.get(&key) {
            return Ok(PreparedFile {
                relative_path: scanned.relative_path.clone(),
                blob_hash_hex: cached.blob_hash_hex.clone(),
                blob_hash_bytes: cached.blob_hash_bytes,
                compressed: None,
                size: scanned.size,
                mode: scanned.mode,
                mtime_secs: scanned.mtime_secs,
                mtime_nanos: scanned.mtime_nanos,
                inode: scanned.inode,
                entry_type: if scanned.is_symlink {
                    EntryType::Symlink
                } else {
                    EntryType::File
                },
            });
        }

        let prepared = prepare_file(scanned, seen_hashes, compressor)?;
        hardlinks.insert(
            key,
            HardlinkPrepared {
                blob_hash_hex: prepared.blob_hash_hex.clone(),
                blob_hash_bytes: prepared.blob_hash_bytes,
            },
        );
        return Ok(prepared);
    }

    prepare_file(scanned, seen_hashes, compressor)
}

fn split_scanned_refs_preserving_hardlinks<'a>(
    scanned_files: &'a [&'a ScannedFile],
    worker_count: usize,
) -> Vec<&'a [&'a ScannedFile]> {
    if scanned_files.is_empty() {
        return Vec::new();
    }

    let target_chunk_size = scanned_files.len().div_ceil(worker_count.max(1));
    let mut chunks = Vec::with_capacity(worker_count.max(1));
    let mut start = 0usize;

    while start < scanned_files.len() {
        let mut end = (start + target_chunk_size).min(scanned_files.len());
        while end < scanned_files.len()
            && hardlink_key(scanned_files[end - 1]) == hardlink_key(scanned_files[end])
            && hardlink_key(scanned_files[end]).is_some()
        {
            end += 1;
        }
        chunks.push(&scanned_files[start..end]);
        start = end;
    }

    chunks
}

fn compress_with_worker_context(
    content: &[u8],
    compressor: &mut zstd::bulk::Compressor<'_>,
) -> Result<Vec<u8>> {
    Ok(compressor.compress(content)?)
}

/// Prepare files with a bounded producer/consumer pipeline so compressed blobs
/// can be consumed immediately instead of accumulating for the full save.
fn process_prepared_files_streaming<F>(
    scanned_files: &[&ScannedFile],
    seen_hashes: &Arc<ShardedSeenHashes>,
    progress: &ProgressCallback,
    progress_counter: &AtomicU64,
    total_to_process: u64,
    mut on_prepared: F,
) -> Result<()>
where
    F: FnMut(PreparedFile) -> Result<()>,
{
    if scanned_files.is_empty() {
        return Ok(());
    }

    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(scanned_files.len());

    if worker_count <= 1 {
        let mut compressor = zstd::bulk::Compressor::new(1)?;
        let mut hardlinks = HashMap::new();
        for scanned in scanned_files {
            let prepared = prepare_file_with_hardlink_cache(
                scanned,
                seen_hashes,
                &mut compressor,
                &mut hardlinks,
            )?;
            let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
            emit(
                progress,
                ProgressEvent::ProcessFile {
                    completed,
                    total: total_to_process,
                },
            );
            on_prepared(prepared)?;
        }
        return Ok(());
    }

    let chunks = split_scanned_refs_preserving_hardlinks(scanned_files, worker_count);

    std::thread::scope(|scope| -> Result<()> {
        let (sender, receiver) = mpsc::sync_channel::<Result<PreparedFile>>(
            PREPARED_FILE_PIPELINE_SLOTS.max(worker_count),
        );

        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let sender = sender.clone();
                scope.spawn(move || {
                    let mut compressor = match zstd::bulk::Compressor::new(1) {
                        Ok(compressor) => compressor,
                        Err(error) => {
                            let _ = sender.send(Err(error.into()));
                            return;
                        }
                    };
                    let mut hardlinks = HashMap::new();

                    for scanned in chunk {
                        let result = prepare_file_with_hardlink_cache(
                            scanned,
                            seen_hashes,
                            &mut compressor,
                            &mut hardlinks,
                        );
                        let completed = progress_counter.fetch_add(1, Ordering::Relaxed) + 1;
                        emit(
                            progress,
                            ProgressEvent::ProcessFile {
                                completed,
                                total: total_to_process,
                            },
                        );
                        if sender.send(result).is_err() {
                            return;
                        }
                    }
                })
            })
            .collect();
        drop(sender);

        let mut consumer_result = Ok(());
        loop {
            match receiver.recv() {
                Ok(Ok(prepared)) => {
                    if let Err(error) = on_prepared(prepared) {
                        consumer_result = Err(error);
                        break;
                    }
                }
                Ok(Err(error)) => {
                    consumer_result = Err(error);
                    break;
                }
                Err(_) => break,
            }
        }
        drop(receiver);

        for handle in handles {
            handle.join().unwrap();
        }

        consumer_result
    })
}

/// Build tree structure bottom-up from processed files.
///
/// Groups files by their directory path, creates tree entries for each file,
/// recursively builds subtrees for subdirectories, and returns the root tree hash.
fn build_tree(processed_files: &[ProcessedFile], tree_store: &TreeStore) -> Result<String> {
    // Group files by parent directory
    let mut dir_files: BTreeMap<String, Vec<&ProcessedFile>> = BTreeMap::new();
    let mut all_dirs: BTreeSet<String> = BTreeSet::new();
    let mut child_dirs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    all_dirs.insert(String::new()); // root always exists

    for pf in processed_files {
        let parent = if let Some(pos) = pf.relative_path.rfind('/') {
            pf.relative_path[..pos].to_string()
        } else {
            String::new()
        };
        dir_files.entry(parent.clone()).or_default().push(pf);
        register_directory_hierarchy(&parent, &mut all_dirs, &mut child_dirs);
    }

    // Sort directories bottom-up (deepest first)
    let mut dir_list: Vec<String> = all_dirs.into_iter().collect();
    dir_list.sort_unstable_by(|a, b| {
        let depth_a = if a.is_empty() {
            0
        } else {
            a.matches('/').count() + 1
        };
        let depth_b = if b.is_empty() {
            0
        } else {
            b.matches('/').count() + 1
        };
        depth_b.cmp(&depth_a).then_with(|| a.cmp(b))
    });

    // Phase 1: Compute all tree hashes and encoded data in memory
    let mut dir_hashes: BTreeMap<String, String> = BTreeMap::new();
    let mut pack_entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(dir_list.len());
    let mut known_hashes: HashSet<[u8; 32]> = HashSet::with_capacity(dir_list.len());

    for dir in &dir_list {
        let file_count = dir_files.get(dir).map(|files| files.len()).unwrap_or(0);
        let child_count = child_dirs
            .get(dir)
            .map(|children| children.len())
            .unwrap_or(0);
        let mut entries: Vec<TreeEntry> = Vec::with_capacity(file_count + child_count);

        if let Some(files) = dir_files.get(dir) {
            for pf in files {
                let name = if let Some(pos) = pf.relative_path.rfind('/') {
                    pf.relative_path[pos + 1..].to_string()
                } else {
                    pf.relative_path.clone()
                };
                entries.push(TreeEntry {
                    name,
                    entry_type: pf.entry_type,
                    hash: pf.blob_hash_bytes,
                    size: pf.size,
                    mode: pf.mode,
                });
            }
        }

        if let Some(children) = child_dirs.get(dir) {
            for sub_dir in children {
                let sub_hash = dir_hashes.get(sub_dir).ok_or_else(|| {
                    ChkpttError::Other(format!("Missing tree hash for directory '{}'", sub_dir))
                })?;
                let sub_name = if let Some(pos) = sub_dir.rfind('/') {
                    sub_dir[pos + 1..].to_string()
                } else {
                    sub_dir.clone()
                };
                entries.push(TreeEntry {
                    name: sub_name,
                    entry_type: EntryType::Dir,
                    hash: hex_to_bytes(sub_hash)?,
                    size: 0,
                    mode: 0o040755,
                });
            }
        }

        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        let encoded = bitcode::encode(&entries);
        let hash = blake3::hash(&encoded);
        let hash_bytes = *hash.as_bytes();
        let hash_hex = hash.to_hex().to_string();

        dir_hashes.insert(dir.clone(), hash_hex.clone());
        if known_hashes.insert(hash_bytes) {
            pack_entries.push((hash_hex, encoded));
        }
    }

    // Phase 2: Write all trees to a single pack file (1 write instead of 37K+)
    tree_store.write_pack(&pack_entries)?;

    dir_hashes
        .get("")
        .cloned()
        .ok_or_else(|| ChkpttError::Other("Failed to build root tree".into()))
}

fn register_directory_hierarchy(
    dir: &str,
    all_dirs: &mut BTreeSet<String>,
    child_dirs: &mut BTreeMap<String, Vec<String>>,
) {
    if dir.is_empty() {
        return;
    }

    let mut parent = String::new();
    let mut current = String::with_capacity(dir.len());
    for segment in dir.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);

        if all_dirs.insert(current.clone()) {
            child_dirs
                .entry(parent.clone())
                .or_default()
                .push(current.clone());
        }
        parent.clear();
        parent.push_str(&current);
    }
}

/// Check if file extension indicates already-compressed content.
#[allow(dead_code)]
fn should_skip_compression(path: &str) -> bool {
    let ext = match path.rsplit_once('.') {
        Some((_, ext)) => ext,
        None => return false,
    };
    matches!(
        ext,
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "avif"
            | "heic"
            | "mp4"
            | "mkv"
            | "avi"
            | "mov"
            | "webm"
            | "mp3"
            | "flac"
            | "ogg"
            | "aac"
            | "opus"
            | "zip"
            | "gz"
            | "bz2"
            | "xz"
            | "zst"
            | "lz4"
            | "br"
            | "7z"
            | "rar"
            | "woff2"
            | "woff"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn test_hex_to_bytes() {
        let hex = "a3b2c1d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
        let bytes = hex_to_bytes(hex).unwrap();
        assert_eq!(bytes[0], 0xa3);
        assert_eq!(bytes[1], 0xb2);
        assert_eq!(bytes[31], 0x90);
    }

    #[test]
    fn test_hex_to_bytes_invalid_length() {
        assert!(hex_to_bytes("abc").is_err());
    }

    #[test]
    fn test_store_has_external_objects_false_for_empty_store() {
        let dir = TempDir::new().unwrap();
        let objects_dir = dir.path().join("objects");
        let packs_dir = dir.path().join("packs");
        std::fs::create_dir_all(&objects_dir).unwrap();
        std::fs::create_dir_all(&packs_dir).unwrap();

        assert!(!store_has_external_objects(&objects_dir, &packs_dir).unwrap());
    }

    #[test]
    fn test_store_has_external_objects_true_for_pack_objects() {
        let dir = TempDir::new().unwrap();
        let objects_dir = dir.path().join("objects");
        let packs_dir = dir.path().join("packs");
        std::fs::create_dir_all(&packs_dir).unwrap();
        std::fs::write(packs_dir.join("pack-demo.dat"), b"pack").unwrap();

        assert!(store_has_external_objects(&objects_dir, &packs_dir).unwrap());
    }

    #[test]
    fn test_compress_with_worker_context_roundtrip() {
        let content = b"compression-context-roundtrip-data";
        let mut compressor = zstd::bulk::Compressor::new(1).unwrap();

        let compressed = compress_with_worker_context(content, &mut compressor).unwrap();
        let decompressed = zstd::decode_all(&compressed[..]).unwrap();

        assert_eq!(decompressed, content);
    }

    #[test]
    fn test_sharded_seen_hashes_dedups_single_thread() {
        let seen = ShardedSeenHashes::new(8);
        let hash = [7u8; 32];

        assert!(seen.insert(hash));
        assert!(!seen.insert(hash));
    }

    #[test]
    fn test_sharded_seen_hashes_dedups_multi_thread() {
        let seen = Arc::new(ShardedSeenHashes::new(1024));
        let unique_inserts = Arc::new(AtomicUsize::new(0));
        let duplicate_hash = [9u8; 32];

        std::thread::scope(|scope| {
            for i in 0..8u8 {
                let seen = Arc::clone(&seen);
                let unique_inserts = Arc::clone(&unique_inserts);
                scope.spawn(move || {
                    if seen.insert(duplicate_hash) {
                        unique_inserts.fetch_add(1, Ordering::Relaxed);
                    }
                    let mut unique_hash = [0u8; 32];
                    unique_hash[0] = i;
                    if seen.insert(unique_hash) {
                        unique_inserts.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });

        assert_eq!(unique_inserts.load(Ordering::Relaxed), 9);
    }

    #[test]
    fn test_split_scanned_refs_preserves_hardlink_groups() {
        let f1 = scanned("a.txt", Some(1));
        let f2 = scanned("b.txt", Some(1));
        let f3 = scanned("c.txt", Some(2));
        let f4 = scanned("d.txt", Some(3));
        let f5 = scanned("e.txt", Some(3));
        let refs = vec![&f1, &f2, &f3, &f4, &f5];

        let chunks = split_scanned_refs_preserving_hardlinks(&refs, 2);
        let paths: Vec<Vec<&str>> = chunks
            .into_iter()
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|file| file.relative_path.as_str())
                    .collect::<Vec<_>>()
            })
            .collect();

        assert_eq!(
            paths,
            vec![vec!["a.txt", "b.txt", "c.txt"], vec!["d.txt", "e.txt"]]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_prepare_file_with_hardlink_cache_reuses_existing_read() {
        let dir = TempDir::new().unwrap();
        let original = dir.path().join("original.txt");
        let alias = dir.path().join("alias.txt");
        fs::write(&original, "same-content").unwrap();
        fs::hard_link(&original, &alias).unwrap();

        let original_scanned = scanned_from_path("original.txt", &original);
        let alias_scanned = scanned_from_path("alias.txt", &alias);
        let seen_hashes = ShardedSeenHashes::new(2);
        let mut compressor = zstd::bulk::Compressor::new(1).unwrap();
        let mut hardlinks = HashMap::new();

        let first = prepare_file_with_hardlink_cache(
            &original_scanned,
            &seen_hashes,
            &mut compressor,
            &mut hardlinks,
        )
        .unwrap();
        let second = prepare_file_with_hardlink_cache(
            &alias_scanned,
            &seen_hashes,
            &mut compressor,
            &mut hardlinks,
        )
        .unwrap();

        assert!(first.compressed.is_some());
        assert!(second.compressed.is_none());
        assert_eq!(first.blob_hash_bytes, second.blob_hash_bytes);
        assert_eq!(first.blob_hash_hex, second.blob_hash_hex);
    }

    fn scanned(relative_path: &str, inode: Option<u64>) -> ScannedFile {
        ScannedFile {
            relative_path: relative_path.to_string(),
            absolute_path: std::path::PathBuf::from(relative_path),
            size: 1,
            mtime_secs: 1,
            mtime_nanos: 1,
            device: Some(1),
            inode,
            mode: 0o100644,
            is_symlink: false,
        }
    }

    #[cfg(unix)]
    fn scanned_from_path(relative_path: &str, path: &Path) -> ScannedFile {
        use std::os::unix::fs::MetadataExt;

        let metadata = fs::metadata(path).unwrap();
        ScannedFile {
            relative_path: relative_path.to_string(),
            absolute_path: path.to_path_buf(),
            size: metadata.len(),
            mtime_secs: metadata.mtime(),
            mtime_nanos: metadata.mtime_nsec(),
            device: Some(metadata.dev()),
            inode: Some(metadata.ino()),
            mode: metadata.mode(),
            is_symlink: metadata.file_type().is_symlink(),
        }
    }

    #[test]
    fn test_should_skip_compression_by_extension() {
        assert!(should_skip_compression("photo.jpg"));
        assert!(should_skip_compression("archive.zip"));
        assert!(should_skip_compression("image.png"));
        assert!(should_skip_compression("video.mp4"));
        assert!(should_skip_compression("data.gz"));
        assert!(should_skip_compression("dir/nested/file.jpeg"));

        assert!(!should_skip_compression("code.rs"));
        assert!(!should_skip_compression("readme.md"));
        assert!(!should_skip_compression("data.json"));
        assert!(!should_skip_compression("no_extension"));
    }
}
