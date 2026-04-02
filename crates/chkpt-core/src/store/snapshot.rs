use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotStats {
    pub total_files: u64,
    pub total_bytes: u64,
    pub new_objects: u64,
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub message: Option<String>,
    pub root_tree_hash: [u8; 32],
    pub parent_snapshot_id: Option<String>,
    pub stats: SnapshotStats,
}

impl Snapshot {
    pub fn new(
        message: Option<String>,
        root_tree_hash: [u8; 32],
        parent_snapshot_id: Option<String>,
        stats: SnapshotStats,
    ) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            created_at: Utc::now(),
            message,
            root_tree_hash,
            parent_snapshot_id,
            stats,
        }
    }
}
