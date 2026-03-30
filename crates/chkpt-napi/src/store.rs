use crate::error::to_napi_error;
use chkpt_core::store::blob::{hash_content, BlobStore};
use chkpt_core::store::snapshot::{Snapshot, SnapshotAttachments, SnapshotStats, SnapshotStore};
use chkpt_core::store::tree::{EntryType, TreeEntry, TreeStore};
use chrono::DateTime;
use napi::bindgen_prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── helpers ──────────────────────────────────────────────────────────

/// Convert a 64-char hex string to a [u8; 32] array.
pub(crate) fn hex_to_bytes32(hex: &str) -> napi::Result<[u8; 32]> {
    if hex.len() != 64 {
        return Err(napi::Error::new(
            napi::Status::InvalidArg,
            format!("expected 64-char hex string, got {} chars", hex.len()),
        ));
    }
    let mut bytes = [0u8; 32];
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| {
            napi::Error::new(
                napi::Status::InvalidArg,
                format!("invalid hex at position {}", i * 2),
            )
        })?;
    }
    Ok(bytes)
}

/// Convert a [u8; 32] array to a 64-char hex string.
pub(crate) fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── blob bindings ────────────────────────────────────────────────────

#[napi]
pub fn blob_hash(content: Buffer) -> String {
    hash_content(content.as_ref())
}

#[napi]
pub async fn blob_store(objects_dir: String, _hash: String, content: Buffer) -> napi::Result<()> {
    let content = content.to_vec();
    let store = BlobStore::new(PathBuf::from(objects_dir));
    store.write(&content).map_err(to_napi_error)?;
    Ok(())
}

#[napi]
pub async fn blob_load(objects_dir: String, hash: String) -> napi::Result<Buffer> {
    let store = BlobStore::new(PathBuf::from(objects_dir));
    let data = store.read(&hash).map_err(to_napi_error)?;
    Ok(Buffer::from(data))
}

#[napi]
pub fn blob_exists(objects_dir: String, hash: String) -> bool {
    let store = BlobStore::new(PathBuf::from(objects_dir));
    store.exists(&hash)
}

// ── tree bindings ────────────────────────────────────────────────────

#[napi(object)]
pub struct JsTreeEntry {
    pub name: String,
    pub entry_type: String,
    pub hash: String,
    pub size: i64,
    pub mode: i64,
}

#[napi(object)]
pub struct JsTreeBuildResult {
    pub hash: String,
}

fn js_entry_to_tree_entry(js: &JsTreeEntry) -> napi::Result<TreeEntry> {
    let entry_type = match js.entry_type.as_str() {
        "file" => EntryType::File,
        "directory" => EntryType::Dir,
        "symlink" => EntryType::Symlink,
        other => {
            return Err(napi::Error::new(
                napi::Status::InvalidArg,
                format!("unknown entry type: {}", other),
            ))
        }
    };
    let hash = hex_to_bytes32(&js.hash)?;
    Ok(TreeEntry {
        name: js.name.clone(),
        entry_type,
        hash,
        size: js.size as u64,
        mode: js.mode as u32,
    })
}

fn tree_entry_to_js(entry: &TreeEntry) -> JsTreeEntry {
    let entry_type = match entry.entry_type {
        EntryType::File => "file",
        EntryType::Dir => "directory",
        EntryType::Symlink => "symlink",
    };
    JsTreeEntry {
        name: entry.name.clone(),
        entry_type: entry_type.to_string(),
        hash: bytes32_to_hex(&entry.hash),
        size: entry.size as i64,
        mode: entry.mode as i64,
    }
}

#[napi]
pub async fn tree_build(
    trees_dir: String,
    entries: Vec<JsTreeEntry>,
) -> napi::Result<JsTreeBuildResult> {
    let tree_entries: Vec<TreeEntry> = entries
        .iter()
        .map(js_entry_to_tree_entry)
        .collect::<napi::Result<Vec<_>>>()?;
    let store = TreeStore::new(PathBuf::from(trees_dir));
    let hash = store.write(&tree_entries).map_err(to_napi_error)?;
    Ok(JsTreeBuildResult { hash })
}

#[napi]
pub async fn tree_load(trees_dir: String, hash: String) -> napi::Result<Vec<JsTreeEntry>> {
    let store = TreeStore::new(PathBuf::from(trees_dir));
    let entries = store.read(&hash).map_err(to_napi_error)?;
    Ok(entries.iter().map(tree_entry_to_js).collect())
}

// ── snapshot bindings ────────────────────────────────────────────────

/// Serde-compatible snapshot attachments for JSON interop.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerdeSnapshotAttachments {
    deps_key: Option<String>,
    git_key: Option<String>,
}

/// Serde-compatible snapshot stats for JSON interop.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerdeSnapshotStats {
    total_files: i64,
    total_bytes: i64,
    new_objects: i64,
}

/// Serde-compatible snapshot for JSON interop.
/// Using serde instead of #[napi(object)] to properly handle null values
/// from JavaScript (napi-rs #[napi(object)] treats null differently from undefined
/// for Option<String> fields).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerdeSnapshot {
    id: String,
    created_at: String,
    message: Option<String>,
    root_tree_hash: String,
    parent_snapshot_id: Option<String>,
    attachments: SerdeSnapshotAttachments,
    stats: SerdeSnapshotStats,
}

fn serde_snapshot_to_core(js: &SerdeSnapshot) -> napi::Result<Snapshot> {
    let created_at = DateTime::parse_from_rfc3339(&js.created_at)
        .map_err(|e| {
            napi::Error::new(
                napi::Status::InvalidArg,
                format!("invalid createdAt: {}", e),
            )
        })?
        .with_timezone(&chrono::Utc);
    let root_tree_hash = hex_to_bytes32(&js.root_tree_hash)?;
    Ok(Snapshot {
        id: js.id.clone(),
        created_at,
        message: js.message.clone(),
        root_tree_hash,
        parent_snapshot_id: js.parent_snapshot_id.clone(),
        attachments: SnapshotAttachments {
            deps_key: js.attachments.deps_key.clone(),
            git_key: js.attachments.git_key.clone(),
        },
        stats: SnapshotStats {
            total_files: js.stats.total_files as u64,
            total_bytes: js.stats.total_bytes as u64,
            new_objects: js.stats.new_objects as u64,
        },
    })
}

fn core_snapshot_to_serde(snap: &Snapshot) -> SerdeSnapshot {
    SerdeSnapshot {
        id: snap.id.clone(),
        created_at: snap.created_at.to_rfc3339(),
        message: snap.message.clone(),
        root_tree_hash: bytes32_to_hex(&snap.root_tree_hash),
        parent_snapshot_id: snap.parent_snapshot_id.clone(),
        attachments: SerdeSnapshotAttachments {
            deps_key: snap.attachments.deps_key.clone(),
            git_key: snap.attachments.git_key.clone(),
        },
        stats: SerdeSnapshotStats {
            total_files: snap.stats.total_files as i64,
            total_bytes: snap.stats.total_bytes as i64,
            new_objects: snap.stats.new_objects as i64,
        },
    }
}

#[napi(
    ts_args_type = "snapshotsDir: string, snap: { id: string, createdAt: string, message: string | null, rootTreeHash: string, parentSnapshotId: string | null, attachments: { depsKey: string | null, gitKey: string | null }, stats: { totalFiles: number, totalBytes: number, newObjects: number } }"
)]
pub async fn snapshot_save(snapshots_dir: String, snap: serde_json::Value) -> napi::Result<()> {
    let serde_snap: SerdeSnapshot = serde_json::from_value(snap).map_err(|e| {
        napi::Error::new(napi::Status::InvalidArg, format!("invalid snapshot: {}", e))
    })?;
    let core_snap = serde_snapshot_to_core(&serde_snap)?;
    let store = SnapshotStore::new(PathBuf::from(snapshots_dir));
    store.save(&core_snap).map_err(to_napi_error)?;
    Ok(())
}

#[napi(
    ts_return_type = "{ id: string, createdAt: string, message: string | null, rootTreeHash: string, parentSnapshotId: string | null, attachments: { depsKey: string | null, gitKey: string | null }, stats: { totalFiles: number, totalBytes: number, newObjects: number } }"
)]
pub async fn snapshot_load(snapshots_dir: String, id: String) -> napi::Result<serde_json::Value> {
    let store = SnapshotStore::new(PathBuf::from(snapshots_dir));
    let snap = store.load(&id).map_err(to_napi_error)?;
    let serde_snap = core_snapshot_to_serde(&snap);
    serde_json::to_value(&serde_snap).map_err(|e| {
        napi::Error::new(
            napi::Status::GenericFailure,
            format!("serialization error: {}", e),
        )
    })
}

#[napi(
    ts_return_type = "Array<{ id: string, createdAt: string, message: string | null, rootTreeHash: string, parentSnapshotId: string | null, attachments: { depsKey: string | null, gitKey: string | null }, stats: { totalFiles: number, totalBytes: number, newObjects: number } }>"
)]
pub async fn snapshot_list(snapshots_dir: String) -> napi::Result<serde_json::Value> {
    let store = SnapshotStore::new(PathBuf::from(snapshots_dir));
    let snaps = store.list(None).map_err(to_napi_error)?;
    let serde_snaps: Vec<SerdeSnapshot> = snaps.iter().map(core_snapshot_to_serde).collect();
    serde_json::to_value(&serde_snaps).map_err(|e| {
        napi::Error::new(
            napi::Status::GenericFailure,
            format!("serialization error: {}", e),
        )
    })
}
