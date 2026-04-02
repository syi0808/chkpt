use chkpt_core::error::ChkpttError;
use chkpt_core::store::catalog::{
    BlobLocation, CatalogSnapshot, ManifestEntry, MetadataCatalog,
};
use chkpt_core::store::snapshot::SnapshotStats;
use chrono::{TimeZone, Utc};
use tempfile::TempDir;

fn sample_snapshot(
    id: &str,
    second: u32,
    message: Option<&str>,
    parent_snapshot_id: Option<&str>,
) -> CatalogSnapshot {
    CatalogSnapshot {
        id: id.to_string(),
        created_at: Utc.with_ymd_and_hms(2026, 3, 31, 1, 0, second).unwrap(),
        message: message.map(|value| value.to_string()),
        parent_snapshot_id: parent_snapshot_id.map(|value| value.to_string()),
        manifest_snapshot_id: None,
        stats: SnapshotStats {
            total_files: 2,
            total_bytes: 15,
            new_objects: 1,
        },
    }
}

fn sample_manifest() -> Vec<ManifestEntry> {
    vec![
        ManifestEntry {
            path: "a.txt".into(),
            blob_hash: [1u8; 32],
            size: 5,
            mode: 0o100644,
        },
        ManifestEntry {
            path: "nested/b.txt".into(),
            blob_hash: [2u8; 32],
            size: 10,
            mode: 0o100644,
        },
    ]
}

#[test]
fn test_catalog_snapshot_manifest_round_trip() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();
    let snapshot = sample_snapshot("snap-a", 0, Some("first"), None);
    let manifest = sample_manifest();

    catalog.insert_snapshot(&snapshot, &manifest).unwrap();

    let loaded = catalog.load_snapshot("snap-a").unwrap();
    assert_eq!(loaded.id, snapshot.id);
    assert_eq!(loaded.message.as_deref(), Some("first"));
    assert_eq!(catalog.snapshot_manifest("snap-a").unwrap(), manifest);
}

#[test]
fn test_catalog_latest_and_prefix_resolution() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();
    let older = sample_snapshot("019d417e-older", 0, Some("older"), None);
    let newer = sample_snapshot("019d417e-newer", 1, Some("newer"), Some("019d417e-older"));
    let manifest = sample_manifest();

    catalog.insert_snapshot(&older, &manifest).unwrap();
    catalog.insert_snapshot(&newer, &manifest).unwrap();

    assert_eq!(catalog.latest_snapshot().unwrap().unwrap().id, newer.id);
    assert_eq!(
        catalog.resolve_snapshot_ref("latest").unwrap().id,
        newer.id
    );
    assert_eq!(
        catalog.resolve_snapshot_ref("019d417e-new").unwrap().id,
        newer.id
    );
}

#[test]
fn test_catalog_rejects_ambiguous_prefix() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();
    let manifest = sample_manifest();

    catalog
        .insert_snapshot(&sample_snapshot("019d417e-aa", 0, None, None), &manifest)
        .unwrap();
    catalog
        .insert_snapshot(&sample_snapshot("019d417e-ab", 1, None, None), &manifest)
        .unwrap();

    let err = catalog.resolve_snapshot_ref("019d417e-a").unwrap_err();
    assert!(matches!(err, ChkpttError::Other(message) if message.contains("Ambiguous snapshot prefix")));
}

#[test]
fn test_catalog_tracks_blob_locations_and_cascades_manifest_rows() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();
    let snapshot = sample_snapshot("snap-a", 0, Some("first"), None);
    let manifest = sample_manifest();

    catalog.insert_snapshot(&snapshot, &manifest).unwrap();
    catalog
        .upsert_blob_location(
            [9u8; 32],
            &BlobLocation {
                pack_hash: Some("pack-1".into()),
                size: 42,
            },
        )
        .unwrap();

    let location = catalog.blob_location(&[9u8; 32]).unwrap().unwrap();
    assert_eq!(location.pack_hash.as_deref(), Some("pack-1"));
    assert_eq!(location.size, 42);

    catalog.delete_snapshot("snap-a").unwrap();
    assert!(matches!(
        catalog.load_snapshot("snap-a"),
        Err(ChkpttError::SnapshotNotFound(_))
    ));
    assert!(catalog.snapshot_manifest("snap-a").unwrap().is_empty());
}

#[test]
fn test_catalog_reuses_manifest_for_metadata_only_snapshots() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();
    let base = sample_snapshot("snap-a", 0, Some("first"), None);
    let alias = sample_snapshot("snap-b", 1, Some("second"), Some("snap-a"));
    let manifest = sample_manifest();

    catalog.insert_snapshot(&base, &manifest).unwrap();
    catalog
        .insert_snapshot_metadata_only(&alias, &base.id)
        .unwrap();

    assert_eq!(catalog.snapshot_manifest("snap-b").unwrap(), manifest);
}

#[test]
fn test_catalog_delete_transfers_manifest_ownership_to_alias() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();
    let base = sample_snapshot("snap-a", 0, Some("first"), None);
    let alias = sample_snapshot("snap-b", 1, Some("second"), Some("snap-a"));
    let manifest = sample_manifest();

    catalog.insert_snapshot(&base, &manifest).unwrap();
    catalog
        .insert_snapshot_metadata_only(&alias, &base.id)
        .unwrap();

    catalog.delete_snapshot("snap-a").unwrap();

    assert_eq!(catalog.snapshot_manifest("snap-b").unwrap(), manifest);
}

#[test]
fn test_catalog_bulk_upserts_and_lists_blob_hashes() {
    let dir = TempDir::new().unwrap();
    let catalog = MetadataCatalog::open(dir.path().join("catalog.sqlite")).unwrap();

    catalog
        .bulk_upsert_blob_locations(&[
            (
                [3u8; 32],
                BlobLocation {
                    pack_hash: Some("pack-a".into()),
                    size: 10,
                },
            ),
            (
                [4u8; 32],
                BlobLocation {
                    pack_hash: None,
                    size: 20,
                },
            ),
        ])
        .unwrap();

    let known = catalog.all_blob_hashes().unwrap();
    assert!(known.contains(&[3u8; 32]));
    assert!(known.contains(&[4u8; 32]));
}
