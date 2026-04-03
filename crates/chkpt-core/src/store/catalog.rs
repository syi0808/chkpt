use crate::error::{ChkpttError, Result};
use crate::store::snapshot::SnapshotStats;
use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const CREATE_SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    message TEXT,
    parent_snapshot_id TEXT,
    manifest_snapshot_id TEXT,
    root_tree_hash BLOB,
    total_files INTEGER NOT NULL,
    total_bytes INTEGER NOT NULL,
    new_objects INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshot_files (
    snapshot_id TEXT NOT NULL,
    path TEXT NOT NULL,
    blob_hash BLOB NOT NULL,
    size INTEGER NOT NULL,
    mode INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, path),
    FOREIGN KEY (snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_snapshot_files_snapshot_id
    ON snapshot_files (snapshot_id);

CREATE TABLE IF NOT EXISTS blob_index (
    blob_hash BLOB PRIMARY KEY,
    pack_hash TEXT,
    size INTEGER NOT NULL
);
"#;

const SNAPSHOT_SELECT_COLUMNS: &str =
    "id, created_at, message, parent_snapshot_id, manifest_snapshot_id, root_tree_hash, total_files, total_bytes, new_objects";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSnapshot {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub message: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub manifest_snapshot_id: Option<String>,
    pub root_tree_hash: Option<[u8; 16]>,
    pub stats: SnapshotStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub blob_hash: [u8; 16],
    pub size: u64,
    pub mode: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobLocation {
    pub pack_hash: Option<String>,
    pub size: u64,
}

pub struct MetadataCatalog {
    conn: Connection,
}

fn row_to_snapshot(row: &Row<'_>) -> rusqlite::Result<CatalogSnapshot> {
    let created_at_raw: String = row.get(1)?;
    let created_at = DateTime::parse_from_rfc3339(&created_at_raw)
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(err))
        })?
        .with_timezone(&Utc);
    let root_tree_hash = row
        .get::<_, Option<Vec<u8>>>(5)?
        .map(|bytes| {
            if bytes.len() != 16 {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Blob,
                    format!("expected 16-byte root tree hash, got {}", bytes.len()).into(),
                ));
            }
            let mut hash = [0u8; 16];
            hash.copy_from_slice(&bytes);
            Ok(hash)
        })
        .transpose()?;
    Ok(CatalogSnapshot {
        id: row.get(0)?,
        created_at,
        message: row.get(2)?,
        parent_snapshot_id: row.get(3)?,
        manifest_snapshot_id: row.get(4)?,
        root_tree_hash,
        stats: SnapshotStats {
            total_files: row.get::<_, i64>(6)? as u64,
            total_bytes: row.get::<_, i64>(7)? as u64,
            new_objects: row.get::<_, i64>(8)? as u64,
        },
    })
}

fn row_to_manifest_entry(row: &Row<'_>) -> rusqlite::Result<ManifestEntry> {
    let blob_hash: Vec<u8> = row.get(1)?;
    if blob_hash.len() != 16 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Blob,
            format!("expected 16-byte blob hash, got {}", blob_hash.len()).into(),
        ));
    }
    let mut hash = [0u8; 16];
    hash.copy_from_slice(&blob_hash);
    Ok(ManifestEntry {
        path: row.get(0)?,
        blob_hash: hash,
        size: row.get::<_, i64>(2)? as u64,
        mode: row.get(3)?,
    })
}

fn row_to_blob_location(
    hash: Vec<u8>,
    row: &Row<'_>,
) -> rusqlite::Result<([u8; 16], BlobLocation)> {
    if hash.len() != 16 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!("expected 16-byte blob hash, got {}", hash.len()).into(),
        ));
    }
    let mut blob_hash = [0u8; 16];
    blob_hash.copy_from_slice(&hash);
    Ok((
        blob_hash,
        BlobLocation {
            pack_hash: row.get(1)?,
            size: row.get::<_, i64>(2)? as u64,
        },
    ))
}

impl MetadataCatalog {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA cache_size=-64000;
             PRAGMA temp_store=MEMORY;
             PRAGMA mmap_size=268435456;",
        )?;
        conn.execute_batch(CREATE_SCHEMA)?;
        ensure_manifest_snapshot_column(&conn)?;
        ensure_root_tree_hash_column(&conn)?;
        Ok(Self { conn })
    }

    pub fn insert_snapshot(
        &self,
        snapshot: &CatalogSnapshot,
        manifest: &[ManifestEntry],
    ) -> Result<()> {
        self.insert_snapshot_with_manifest_owner(snapshot, manifest, &snapshot.id)
    }

    pub fn insert_snapshot_metadata_only(
        &self,
        snapshot: &CatalogSnapshot,
        manifest_snapshot_id: &str,
    ) -> Result<()> {
        self.insert_snapshot_row(snapshot, manifest_snapshot_id)
    }

    fn insert_snapshot_with_manifest_owner(
        &self,
        snapshot: &CatalogSnapshot,
        manifest: &[ManifestEntry],
        manifest_snapshot_id: &str,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        insert_snapshot_row_tx(&tx, snapshot, manifest_snapshot_id)?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO snapshot_files (snapshot_id, path, blob_hash, size, mode)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for entry in manifest {
                stmt.execute(params![
                    snapshot.id,
                    entry.path,
                    entry.blob_hash.as_slice(),
                    entry.size as i64,
                    entry.mode,
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn load_snapshot(&self, snapshot_id: &str) -> Result<CatalogSnapshot> {
        let query = format!("SELECT {SNAPSHOT_SELECT_COLUMNS} FROM snapshots WHERE id = ?1");
        let mut stmt = self.conn.prepare(&query)?;
        stmt.query_row(params![snapshot_id], row_to_snapshot)
            .optional()?
            .ok_or_else(|| ChkpttError::SnapshotNotFound(snapshot_id.to_string()))
    }

    pub fn latest_snapshot(&self) -> Result<Option<CatalogSnapshot>> {
        let query = format!(
            "SELECT {SNAPSHOT_SELECT_COLUMNS}
             FROM snapshots
             ORDER BY created_at DESC, id DESC
             LIMIT 1"
        );
        let mut stmt = self.conn.prepare(&query)?;
        Ok(stmt.query_row([], row_to_snapshot).optional()?)
    }

    pub fn resolve_snapshot_ref(&self, snapshot_ref: &str) -> Result<CatalogSnapshot> {
        if snapshot_ref == "latest" {
            return self.latest_snapshot()?.ok_or_else(|| {
                ChkpttError::SnapshotNotFound("latest (no snapshots exist)".into())
            });
        }

        if let Ok(snapshot) = self.load_snapshot(snapshot_ref) {
            return Ok(snapshot);
        }

        let query = format!(
            "SELECT {SNAPSHOT_SELECT_COLUMNS}
             FROM snapshots
             WHERE id LIKE ?1
             ORDER BY created_at DESC, id DESC"
        );
        let mut stmt = self.conn.prepare(&query)?;
        let prefix = format!("{snapshot_ref}%");
        let matches = stmt
            .query_map(params![prefix], row_to_snapshot)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        match matches.len() {
            0 => Err(ChkpttError::SnapshotNotFound(snapshot_ref.to_string())),
            1 => Ok(matches.into_iter().next().unwrap()),
            count => Err(ChkpttError::Other(format!(
                "Ambiguous snapshot prefix '{}': matches {} snapshots",
                snapshot_ref, count
            ))),
        }
    }

    pub fn list_snapshots(&self, limit: Option<usize>) -> Result<Vec<CatalogSnapshot>> {
        let query = if limit.is_some() {
            format!(
                "SELECT {SNAPSHOT_SELECT_COLUMNS}
                 FROM snapshots
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?1"
            )
        } else {
            format!(
                "SELECT {SNAPSHOT_SELECT_COLUMNS}
                 FROM snapshots
                 ORDER BY created_at DESC, id DESC"
            )
        };
        let mut stmt = self.conn.prepare(&query)?;
        let rows = if let Some(limit) = limit {
            stmt.query_map(params![limit as i64], row_to_snapshot)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], row_to_snapshot)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn snapshot_manifest(&self, snapshot_id: &str) -> Result<Vec<ManifestEntry>> {
        let Some(manifest_snapshot_id) = self.manifest_snapshot_owner(snapshot_id)? else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mode
             FROM snapshot_files
             WHERE snapshot_id = ?1
             ORDER BY path",
        )?;
        let entries = stmt
            .query_map(params![manifest_snapshot_id], row_to_manifest_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    pub fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let snapshot = self.load_snapshot(snapshot_id)?;
        let manifest_owner = snapshot
            .manifest_snapshot_id
            .clone()
            .unwrap_or_else(|| snapshot.id.clone());
        let tx = self.conn.unchecked_transaction()?;

        if manifest_owner == snapshot_id {
            let aliases = {
                let mut stmt = tx.prepare(
                    "SELECT id
                     FROM snapshots
                     WHERE manifest_snapshot_id = ?1
                     ORDER BY created_at DESC, id DESC",
                )?;
                let rows = stmt.query_map(params![snapshot_id], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };

            if let Some(new_owner_id) = aliases.first() {
                tx.execute(
                    "UPDATE snapshot_files SET snapshot_id = ?1 WHERE snapshot_id = ?2",
                    params![new_owner_id, snapshot_id],
                )?;
                tx.execute(
                    "UPDATE snapshots
                     SET manifest_snapshot_id = ?1
                     WHERE manifest_snapshot_id = ?2",
                    params![new_owner_id, snapshot_id],
                )?;
                tx.execute(
                    "UPDATE snapshots
                     SET manifest_snapshot_id = NULL
                     WHERE id = ?1",
                    params![new_owner_id],
                )?;
            }
        }

        let deleted = tx.execute("DELETE FROM snapshots WHERE id = ?1", params![snapshot_id])?;
        if deleted == 0 {
            return Err(ChkpttError::SnapshotNotFound(snapshot_id.to_string()));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn bulk_upsert_blob_locations(&self, entries: &[([u8; 16], BlobLocation)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO blob_index (blob_hash, pack_hash, size)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(blob_hash) DO UPDATE SET
                    pack_hash=excluded.pack_hash,
                    size=excluded.size",
            )?;
            for (blob_hash, location) in entries {
                stmt.execute(params![
                    blob_hash.as_slice(),
                    location.pack_hash,
                    location.size as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn blob_location(&self, blob_hash: &[u8; 16]) -> Result<Option<BlobLocation>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pack_hash, size FROM blob_index WHERE blob_hash = ?1")?;
        Ok(stmt
            .query_row(params![blob_hash.as_slice()], |row: &Row<'_>| {
                Ok(BlobLocation {
                    pack_hash: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                })
            })
            .optional()?)
    }

    pub fn blob_locations_for_hashes(
        &self,
        blob_hashes: &[[u8; 16]],
    ) -> Result<HashMap<[u8; 16], BlobLocation>> {
        const SQLITE_MAX_VARS: usize = 512;

        if blob_hashes.is_empty() {
            return Ok(HashMap::new());
        }

        let mut locations = HashMap::with_capacity(blob_hashes.len());
        for chunk in blob_hashes.chunks(SQLITE_MAX_VARS) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT blob_hash, pack_hash, size FROM blob_index WHERE blob_hash IN ({})",
                placeholders
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt
                .query_map(
                    params_from_iter(chunk.iter().map(|hash| hash.as_slice())),
                    |row: &Row<'_>| {
                        let hash: Vec<u8> = row.get(0)?;
                        row_to_blob_location(hash, row)
                    },
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            locations.extend(rows);
        }

        Ok(locations)
    }

    pub fn all_blob_hashes(&self) -> Result<HashSet<[u8; 16]>> {
        let mut stmt = self.conn.prepare("SELECT blob_hash FROM blob_index")?;
        let hashes = stmt
            .query_map([], |row: &Row<'_>| {
                let hash: Vec<u8> = row.get(0)?;
                if hash.len() != 16 {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        format!("expected 16-byte blob hash, got {}", hash.len()).into(),
                    ));
                }
                let mut blob_hash = [0u8; 16];
                blob_hash.copy_from_slice(&hash);
                Ok(blob_hash)
            })?
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        Ok(hashes)
    }

    pub fn delete_blob_location(&self, blob_hash: &[u8; 16]) -> Result<()> {
        self.conn.execute(
            "DELETE FROM blob_index WHERE blob_hash = ?1",
            params![blob_hash.as_slice()],
        )?;
        Ok(())
    }

    pub fn pack_reference_count(&self, pack_hash: &str) -> Result<u64> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM blob_index WHERE pack_hash = ?1")?;
        Ok(stmt.query_row(params![pack_hash], |row: &Row<'_>| row.get::<_, i64>(0))? as u64)
    }

    fn manifest_snapshot_owner(&self, snapshot_id: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT COALESCE(manifest_snapshot_id, id)
             FROM snapshots
             WHERE id = ?1",
        )?;
        Ok(stmt
            .query_row(params![snapshot_id], |row| row.get::<_, String>(0))
            .optional()?)
    }

    fn insert_snapshot_row(
        &self,
        snapshot: &CatalogSnapshot,
        manifest_snapshot_id: &str,
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        insert_snapshot_row_tx(&tx, snapshot, manifest_snapshot_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Expose the inner connection for testing/diagnostics.
    #[cfg(test)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn insert_snapshot_row_tx(
    tx: &rusqlite::Transaction<'_>,
    snapshot: &CatalogSnapshot,
    manifest_snapshot_id: &str,
) -> Result<()> {
    let manifest_snapshot_id = if manifest_snapshot_id == snapshot.id {
        None::<String>
    } else {
        Some(manifest_snapshot_id.to_string())
    };
    tx.execute(
        "INSERT INTO snapshots (id, created_at, message, parent_snapshot_id, manifest_snapshot_id, root_tree_hash, total_files, total_bytes, new_objects)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            snapshot.id,
            snapshot.created_at.to_rfc3339(),
            snapshot.message,
            snapshot.parent_snapshot_id,
            manifest_snapshot_id,
            snapshot.root_tree_hash.as_ref().map(|hash| hash.as_slice()),
            snapshot.stats.total_files as i64,
            snapshot.stats.total_bytes as i64,
            snapshot.stats.new_objects as i64,
        ],
    )?;
    Ok(())
}

fn ensure_manifest_snapshot_column(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(snapshots)")?;
    let has_column = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .any(|name| name == "manifest_snapshot_id");
    if !has_column {
        conn.execute(
            "ALTER TABLE snapshots ADD COLUMN manifest_snapshot_id TEXT",
            [],
        )?;
    }
    Ok(())
}

fn ensure_root_tree_hash_column(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(snapshots)")?;
    let has_column = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .any(|name| name == "root_tree_hash");
    if !has_column {
        conn.execute("ALTER TABLE snapshots ADD COLUMN root_tree_hash BLOB", [])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_blob_locations_for_hashes_returns_requested_rows() {
        let dir = TempDir::new().unwrap();
        let catalog = MetadataCatalog::open(dir.path().join("catalog.db")).unwrap();

        let hash_a = xxhash_rust::xxh3::xxh3_128(b"a").to_le_bytes();
        let hash_b = xxhash_rust::xxh3::xxh3_128(b"b").to_le_bytes();
        let hash_missing = xxhash_rust::xxh3::xxh3_128(b"missing").to_le_bytes();

        catalog
            .bulk_upsert_blob_locations(&[
                (
                    hash_a,
                    BlobLocation {
                        pack_hash: Some("pack-a".to_string()),
                        size: 10,
                    },
                ),
                (
                    hash_b,
                    BlobLocation {
                        pack_hash: None,
                        size: 20,
                    },
                ),
            ])
            .unwrap();

        let locations = catalog
            .blob_locations_for_hashes(&[hash_a, hash_b, hash_missing])
            .unwrap();

        assert_eq!(
            locations.get(&hash_a),
            Some(&BlobLocation {
                pack_hash: Some("pack-a".to_string()),
                size: 10,
            })
        );
        assert_eq!(
            locations.get(&hash_b),
            Some(&BlobLocation {
                pack_hash: None,
                size: 20,
            })
        );
        assert!(!locations.contains_key(&hash_missing));
    }

    #[test]
    fn test_catalog_pragmas_are_set() {
        let dir = TempDir::new().unwrap();
        let catalog = MetadataCatalog::open(dir.path().join("catalog.db")).unwrap();
        let conn = catalog.connection();

        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let synchronous: i64 = conn
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 1); // NORMAL = 1

        let temp_store: i64 = conn
            .query_row("PRAGMA temp_store", [], |row| row.get(0))
            .unwrap();
        assert_eq!(temp_store, 2); // MEMORY = 2
    }
}
