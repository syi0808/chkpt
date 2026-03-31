pub mod schema;

use crate::error::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub blob_hash: [u8; 32],
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub inode: Option<u64>,
    pub mode: u32,
}

pub struct FileIndex {
    conn: Connection,
}

fn row_to_entry(row: &Row<'_>) -> rusqlite::Result<FileEntry> {
    let hash_blob: Vec<u8> = row.get(1)?;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hash_blob);
    Ok(FileEntry {
        path: row.get(0)?,
        blob_hash: hash,
        size: row.get::<_, i64>(2)? as u64,
        mtime_secs: row.get(3)?,
        mtime_nanos: row.get(4)?,
        inode: row.get::<_, Option<i64>>(5)?.map(|i| i as u64),
        mode: row.get(6)?,
    })
}

impl FileIndex {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(schema::CREATE_TABLES)?;
        Ok(Self { conn })
    }

    pub fn upsert(&self, entry: &FileEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_index (path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                blob_hash=excluded.blob_hash, size=excluded.size,
                mtime_secs=excluded.mtime_secs, mtime_nanos=excluded.mtime_nanos,
                inode=excluded.inode, mode=excluded.mode",
            params![
                entry.path,
                entry.blob_hash.as_slice(),
                entry.size as i64,
                entry.mtime_secs,
                entry.mtime_nanos,
                entry.inode.map(|i| i as i64),
                entry.mode,
            ],
        )?;
        Ok(())
    }

    pub fn bulk_upsert(&self, entries: &[FileEntry]) -> Result<()> {
        self.apply_changes(&[], entries)
    }

    pub fn bulk_remove(&self, paths: &[String]) -> Result<()> {
        self.apply_changes(paths, &[])
    }

    pub fn apply_changes(
        &self,
        paths_to_remove: &[String],
        entries_to_upsert: &[FileEntry],
    ) -> Result<()> {
        if paths_to_remove.is_empty() && entries_to_upsert.is_empty() {
            return Ok(());
        }

        let tx = self.conn.unchecked_transaction()?;
        {
            if !paths_to_remove.is_empty() {
                let mut delete_stmt = tx.prepare("DELETE FROM file_index WHERE path = ?1")?;
                for path in paths_to_remove {
                    delete_stmt.execute(params![path])?;
                }
            }

            if !entries_to_upsert.is_empty() {
                let mut upsert_stmt = tx.prepare(
                    "INSERT INTO file_index (path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(path) DO UPDATE SET
                        blob_hash=excluded.blob_hash, size=excluded.size,
                        mtime_secs=excluded.mtime_secs, mtime_nanos=excluded.mtime_nanos,
                        inode=excluded.inode, mode=excluded.mode",
                )?;
                for entry in entries_to_upsert {
                    upsert_stmt.execute(params![
                        entry.path,
                        entry.blob_hash.as_slice(),
                        entry.size as i64,
                        entry.mtime_secs,
                        entry.mtime_nanos,
                        entry.inode.map(|i| i as i64),
                        entry.mode,
                    ])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self, path: &str) -> Result<Option<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode FROM file_index WHERE path = ?1"
        )?;
        let result = stmt.query_row(params![path], row_to_entry).optional()?;
        Ok(result)
    }

    pub fn remove(&self, path: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM file_index WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn all_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM file_index")?;
        let paths = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn all_entries(&self) -> Result<Vec<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode FROM file_index",
        )?;
        let entries = stmt
            .query_map([], row_to_entry)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    pub fn entries_by_path(&self) -> Result<HashMap<String, FileEntry>> {
        let mut entries = HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode FROM file_index",
        )?;
        for entry in stmt.query_map([], row_to_entry)? {
            let entry = entry?;
            entries.insert(entry.path.clone(), entry);
        }
        Ok(entries)
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM file_index", [])?;
        Ok(())
    }
}
