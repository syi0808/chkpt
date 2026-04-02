use crate::error::{ChkpttError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotAttachments {
    pub deps_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

    fn latest_path(&self) -> PathBuf {
        self.dir.join(".latest")
    }

    fn write_latest_id(&self, snapshot_id: &str) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let latest_path = self.latest_path();
        let latest_tmp = latest_path.with_extension("tmp");
        std::fs::write(&latest_tmp, snapshot_id)?;
        std::fs::rename(&latest_tmp, &latest_path)?;
        Ok(())
    }

    fn clear_latest_id(&self) -> Result<()> {
        let latest_path = self.latest_path();
        if let Err(error) = std::fs::remove_file(latest_path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        Ok(())
    }

    pub fn save(&self, snapshot: &Snapshot) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.snapshot_path(&snapshot.id);
        let json = serde_json::to_string_pretty(snapshot)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &path)?;
        self.write_latest_id(&snapshot.id)?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Snapshot> {
        let path = self.snapshot_path(id);
        let json = match std::fs::read_to_string(&path) {
            Ok(json) => json,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ChkpttError::SnapshotNotFound(id.to_string()));
            }
            Err(error) => return Err(error.into()),
        };
        Ok(serde_json::from_str(&json)?)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.snapshot_path(id);
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        let latest_path = self.latest_path();
        match std::fs::read_to_string(&latest_path) {
            Ok(latest_id) if latest_id.trim() == id => self.clear_latest_id()?,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    pub fn list(&self, limit: Option<usize>) -> Result<Vec<Snapshot>> {
        let mut snapshots = Vec::new();
        let read_dir = match std::fs::read_dir(&self.dir) {
            Ok(read_dir) => read_dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(snapshots),
            Err(error) => return Err(error.into()),
        };
        for entry in read_dir {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
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
        let latest_path = self.latest_path();
        match std::fs::read_to_string(&latest_path) {
            Ok(snapshot_id) => {
                let snapshot_id = snapshot_id.trim();
                if !snapshot_id.is_empty() {
                    if let Ok(snapshot) = self.load(snapshot_id) {
                        return Ok(Some(snapshot));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let latest = self.list(Some(1))?.into_iter().next();
        match latest {
            Some(snapshot) => {
                self.write_latest_id(&snapshot.id)?;
                Ok(Some(snapshot))
            }
            None => {
                self.clear_latest_id()?;
                Ok(None)
            }
        }
    }

    /// Return all snapshot IDs.
    pub fn all_ids(&self) -> Result<Vec<String>> {
        Ok(self.list(None)?.into_iter().map(|s| s.id).collect())
    }
}
