use crate::error::{ChkpttError, Result};
use crate::store::snapshot::SnapshotStats;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashSet;
use std::path::Path;

const CREATE_SCHEMA: &str = r#"
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS snapshots (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    message TEXT,
    parent_snapshot_id TEXT,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSnapshot {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub message: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub stats: SnapshotStats,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    pub path: String,
    pub blob_hash: [u8; 32],
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
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?
        .with_timezone(&Utc);
    Ok(CatalogSnapshot {
        id: row.get(0)?,
        created_at,
        message: row.get(2)?,
        parent_snapshot_id: row.get(3)?,
        stats: SnapshotStats {
            total_files: row.get::<_, i64>(4)? as u64,
            total_bytes: row.get::<_, i64>(5)? as u64,
            new_objects: row.get::<_, i64>(6)? as u64,
        },
    })
}

fn row_to_manifest_entry(row: &Row<'_>) -> rusqlite::Result<ManifestEntry> {
    let blob_hash: Vec<u8> = row.get(1)?;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&blob_hash);
    Ok(ManifestEntry {
        path: row.get(0)?,
        blob_hash: hash,
        size: row.get::<_, i64>(2)? as u64,
        mode: row.get(3)?,
    })
}

fn row_to_blob_location(hash: Vec<u8>, row: &Row<'_>) -> rusqlite::Result<([u8; 32], BlobLocation)> {
    let mut blob_hash = [0u8; 32];
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
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(CREATE_SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert_snapshot(
        &self,
        snapshot: &CatalogSnapshot,
        manifest: &[ManifestEntry],
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO snapshots (id, created_at, message, parent_snapshot_id, total_files, total_bytes, new_objects)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                snapshot.id,
                snapshot.created_at.to_rfc3339(),
                snapshot.message,
                snapshot.parent_snapshot_id,
                snapshot.stats.total_files as i64,
                snapshot.stats.total_bytes as i64,
                snapshot.stats.new_objects as i64,
            ],
        )?;

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
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, message, parent_snapshot_id, total_files, total_bytes, new_objects
             FROM snapshots WHERE id = ?1",
        )?;
        stmt.query_row(params![snapshot_id], row_to_snapshot)
            .optional()?
            .ok_or_else(|| ChkpttError::SnapshotNotFound(snapshot_id.to_string()))
    }

    pub fn latest_snapshot(&self) -> Result<Option<CatalogSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, message, parent_snapshot_id, total_files, total_bytes, new_objects
             FROM snapshots
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
        )?;
        Ok(stmt.query_row([], row_to_snapshot).optional()?)
    }

    pub fn resolve_snapshot_ref(&self, snapshot_ref: &str) -> Result<CatalogSnapshot> {
        if snapshot_ref == "latest" {
            return self
                .latest_snapshot()?
                .ok_or_else(|| ChkpttError::SnapshotNotFound("latest (no snapshots exist)".into()));
        }

        if let Ok(snapshot) = self.load_snapshot(snapshot_ref) {
            return Ok(snapshot);
        }

        let mut stmt = self.conn.prepare(
            "SELECT id, created_at, message, parent_snapshot_id, total_files, total_bytes, new_objects
             FROM snapshots
             WHERE id LIKE ?1
             ORDER BY created_at DESC, id DESC",
        )?;
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
            "SELECT id, created_at, message, parent_snapshot_id, total_files, total_bytes, new_objects
             FROM snapshots
             ORDER BY created_at DESC, id DESC
             LIMIT ?1"
        } else {
            "SELECT id, created_at, message, parent_snapshot_id, total_files, total_bytes, new_objects
             FROM snapshots
             ORDER BY created_at DESC, id DESC"
        };
        let mut stmt = self.conn.prepare(query)?;
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
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mode
             FROM snapshot_files
             WHERE snapshot_id = ?1
             ORDER BY path",
        )?;
        let entries = stmt
            .query_map(params![snapshot_id], row_to_manifest_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    pub fn delete_snapshot(&self, snapshot_id: &str) -> Result<()> {
        let deleted = self
            .conn
            .execute("DELETE FROM snapshots WHERE id = ?1", params![snapshot_id])?;
        if deleted == 0 {
            return Err(ChkpttError::SnapshotNotFound(snapshot_id.to_string()));
        }
        Ok(())
    }

    pub fn upsert_blob_location(&self, blob_hash: [u8; 32], location: &BlobLocation) -> Result<()> {
        self.conn.execute(
            "INSERT INTO blob_index (blob_hash, pack_hash, size)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(blob_hash) DO UPDATE SET
                pack_hash=excluded.pack_hash,
                size=excluded.size",
            params![blob_hash.as_slice(), location.pack_hash, location.size as i64],
        )?;
        Ok(())
    }

    pub fn bulk_upsert_blob_locations(
        &self,
        entries: &[([u8; 32], BlobLocation)],
    ) -> Result<()> {
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

    pub fn blob_location(&self, blob_hash: &[u8; 32]) -> Result<Option<BlobLocation>> {
        let mut stmt = self.conn.prepare(
            "SELECT pack_hash, size FROM blob_index WHERE blob_hash = ?1",
        )?;
        Ok(stmt
            .query_row(params![blob_hash.as_slice()], |row: &Row<'_>| {
                Ok(BlobLocation {
                    pack_hash: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                })
            })
            .optional()?)
    }

    pub fn all_blob_hashes(&self) -> Result<HashSet<[u8; 32]>> {
        let mut stmt = self.conn.prepare("SELECT blob_hash FROM blob_index")?;
        let hashes = stmt
            .query_map([], |row: &Row<'_>| {
                let hash: Vec<u8> = row.get(0)?;
                let mut blob_hash = [0u8; 32];
                blob_hash.copy_from_slice(&hash);
                Ok(blob_hash)
            })?
            .collect::<std::result::Result<HashSet<_>, _>>()?;
        Ok(hashes)
    }

    pub fn unreferenced_blobs(&self) -> Result<Vec<([u8; 32], BlobLocation)>> {
        let mut stmt = self.conn.prepare(
            "SELECT blob_index.blob_hash, blob_index.pack_hash, blob_index.size
             FROM blob_index
             LEFT JOIN snapshot_files ON snapshot_files.blob_hash = blob_index.blob_hash
             WHERE snapshot_files.blob_hash IS NULL",
        )?;
        let rows = stmt
            .query_map([], |row: &Row<'_>| {
                let hash: Vec<u8> = row.get(0)?;
                row_to_blob_location(hash, row)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_blob_location(&self, blob_hash: &[u8; 32]) -> Result<()> {
        self.conn
            .execute("DELETE FROM blob_index WHERE blob_hash = ?1", params![blob_hash.as_slice()])?;
        Ok(())
    }

    pub fn pack_reference_count(&self, pack_hash: &str) -> Result<u64> {
        let mut stmt = self
            .conn
            .prepare("SELECT COUNT(*) FROM blob_index WHERE pack_hash = ?1")?;
        Ok(stmt.query_row(params![pack_hash], |row: &Row<'_>| row.get::<_, i64>(0))? as u64)
    }
}
