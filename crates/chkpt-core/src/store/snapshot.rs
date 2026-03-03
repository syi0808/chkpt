use crate::error::{ChkpttError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotAttachments {
    pub deps_key: Option<String>,
    pub git_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotStats {
    pub total_files: u64,
    pub total_bytes: u64,
    pub new_objects: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub message: Option<String>,
    pub root_tree_hash: [u8; 32],
    pub parent_snapshot_id: Option<String>,
    pub attachments: SnapshotAttachments,
    pub stats: SnapshotStats,
}

impl Snapshot {
    pub fn new(
        message: Option<String>,
        root_tree_hash: [u8; 32],
        parent_snapshot_id: Option<String>,
        attachments: SnapshotAttachments,
        stats: SnapshotStats,
    ) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            created_at: Utc::now(),
            message,
            root_tree_hash,
            parent_snapshot_id,
            attachments,
            stats,
        }
    }
}

pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn snapshot_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn save(&self, snapshot: &Snapshot) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.snapshot_path(&snapshot.id);
        let json = serde_json::to_string_pretty(snapshot)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Snapshot> {
        let path = self.snapshot_path(id);
        if !path.exists() {
            return Err(ChkpttError::SnapshotNotFound(id.to_string()));
        }
        let json = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.snapshot_path(id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn list(&self, limit: Option<usize>) -> Result<Vec<Snapshot>> {
        let mut snapshots = Vec::new();
        if !self.dir.exists() {
            return Ok(snapshots);
        }
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "json") {
                let json = std::fs::read_to_string(&path)?;
                if let Ok(snap) = serde_json::from_str::<Snapshot>(&json) {
                    snapshots.push(snap);
                }
            }
        }
        snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(limit) = limit {
            snapshots.truncate(limit);
        }
        Ok(snapshots)
    }

    pub fn latest(&self) -> Result<Option<Snapshot>> {
        let list = self.list(Some(1))?;
        Ok(list.into_iter().next())
    }

    /// Return all snapshot IDs.
    pub fn all_ids(&self) -> Result<Vec<String>> {
        Ok(self.list(None)?.into_iter().map(|s| s.id).collect())
    }
}
