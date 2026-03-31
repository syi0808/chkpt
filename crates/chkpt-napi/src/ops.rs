use crate::error::to_napi_error;
use chkpt_core::ops::delete;
use chkpt_core::ops::list as ops_list;
use chkpt_core::ops::restore::{self, RestoreOptions};
use chkpt_core::ops::save::{self, SaveOptions};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── save ────────────────────────────────────────────────────────────

#[napi(object)]
pub struct JsSaveResult {
    pub snapshot_id: String,
    pub total_files: i64,
    pub total_bytes: i64,
    pub new_objects: i64,
}

#[napi]
pub async fn save(
    workspace_path: String,
    message: Option<String>,
    include_deps: Option<bool>,
) -> napi::Result<JsSaveResult> {
    let root = PathBuf::from(workspace_path);
    let options = SaveOptions {
        message,
        include_deps: include_deps.unwrap_or(false),
        ..Default::default()
    };
    let result = save::save(&root, options).map_err(to_napi_error)?;
    Ok(JsSaveResult {
        snapshot_id: result.snapshot_id,
        total_files: result.stats.total_files as i64,
        total_bytes: result.stats.total_bytes as i64,
        new_objects: result.stats.new_objects as i64,
    })
}

// ── list ────────────────────────────────────────────────────────────

/// Serde-compatible snapshot for list results.
/// Reuses the same JSON shape as the store module's snapshot type.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerdeListSnapshot {
    id: String,
    created_at: String,
    message: Option<String>,
    root_tree_hash: String,
    parent_snapshot_id: Option<String>,
    stats: SerdeListStats,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerdeListStats {
    total_files: i64,
    total_bytes: i64,
    new_objects: i64,
}

fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[napi(
    ts_return_type = "Array<{ id: string, createdAt: string, message: string | null, rootTreeHash: string, parentSnapshotId: string | null, stats: { totalFiles: number, totalBytes: number, newObjects: number } }>"
)]
pub async fn list(workspace_path: String, limit: Option<u32>) -> napi::Result<serde_json::Value> {
    let root = PathBuf::from(workspace_path);
    let snapshots = ops_list::list(&root, limit.map(|l| l as usize)).map_err(to_napi_error)?;
    let serde_snaps: Vec<SerdeListSnapshot> = snapshots
        .iter()
        .map(|s| SerdeListSnapshot {
            id: s.id.clone(),
            created_at: s.created_at.to_rfc3339(),
            message: s.message.clone(),
            root_tree_hash: bytes32_to_hex(&s.root_tree_hash),
            parent_snapshot_id: s.parent_snapshot_id.clone(),
            stats: SerdeListStats {
                total_files: s.stats.total_files as i64,
                total_bytes: s.stats.total_bytes as i64,
                new_objects: s.stats.new_objects as i64,
            },
        })
        .collect();
    serde_json::to_value(&serde_snaps).map_err(|e| {
        napi::Error::new(
            napi::Status::GenericFailure,
            format!("serialization error: {}", e),
        )
    })
}

// ── restore ─────────────────────────────────────────────────────────

#[napi(object)]
pub struct JsRestoreResult {
    pub snapshot_id: String,
    pub files_added: i64,
    pub files_changed: i64,
    pub files_removed: i64,
    pub files_unchanged: i64,
}

#[napi]
pub async fn restore(
    workspace_path: String,
    snapshot_id: String,
    dry_run: Option<bool>,
) -> napi::Result<JsRestoreResult> {
    let root = PathBuf::from(workspace_path);
    let options = RestoreOptions {
        dry_run: dry_run.unwrap_or(false),
        ..Default::default()
    };
    let result = restore::restore(&root, &snapshot_id, options).map_err(to_napi_error)?;
    Ok(JsRestoreResult {
        snapshot_id: result.snapshot_id,
        files_added: result.files_added as i64,
        files_changed: result.files_changed as i64,
        files_removed: result.files_removed as i64,
        files_unchanged: result.files_unchanged as i64,
    })
}

// ── delete ──────────────────────────────────────────────────────────

#[napi]
pub async fn delete_snapshot(workspace_path: String, snapshot_id: String) -> napi::Result<()> {
    let root = PathBuf::from(workspace_path);
    delete::delete(&root, &snapshot_id).map_err(to_napi_error)?;
    Ok(())
}
