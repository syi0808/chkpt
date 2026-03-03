use crate::config::{project_id_from_path, StoreLayout};
use crate::error::Result;
use crate::store::snapshot::{Snapshot, SnapshotStore};
use std::path::Path;

pub fn list(workspace_root: &Path, limit: Option<usize>) -> Result<Vec<Snapshot>> {
    let project_id = project_id_from_path(workspace_root);
    let layout = StoreLayout::new(&project_id);
    let snapshot_store = SnapshotStore::new(layout.snapshots_dir());
    snapshot_store.list(limit)
}
