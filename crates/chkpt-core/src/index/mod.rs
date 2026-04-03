use crate::error::Result;
use bitcode::{Decode, Encode};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Encode, Decode)]
pub struct FileEntry {
    pub path: String,
    pub blob_hash: [u8; 16],
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub inode: Option<u64>,
    pub mode: u32,
}

pub struct FileIndex {
    path: PathBuf,
    entries: HashMap<String, FileEntry>,
}

impl FileIndex {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let entries = match std::fs::read(&path) {
            Ok(data) => {
                let entries_vec: Vec<FileEntry> = bitcode::decode(&data)?;
                let mut entries = HashMap::with_capacity(entries_vec.len());
                for entry in entries_vec {
                    entries.insert(entry.path.clone(), entry);
                }
                entries
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => return Err(error.into()),
        };
        Ok(Self { path, entries })
    }

    fn flush(&self) -> Result<()> {
        let mut entries_vec = Vec::with_capacity(self.entries.len());
        entries_vec.extend(self.entries.values().cloned());
        let encoded = bitcode::encode(&entries_vec);
        let tmp_path = self.path.with_extension("bin.tmp");
        std::fs::write(&tmp_path, &encoded)?;
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }

    pub fn upsert(&mut self, entry: &FileEntry) -> Result<()> {
        self.entries.insert(entry.path.clone(), entry.clone());
        self.flush()
    }

    pub fn bulk_upsert(&mut self, entries: &[FileEntry]) -> Result<()> {
        self.apply_changes(&[], entries)
    }

    pub fn bulk_remove(&mut self, paths: &[String]) -> Result<()> {
        self.apply_changes(paths, &[])
    }

    pub fn apply_changes(
        &mut self,
        paths_to_remove: &[String],
        entries_to_upsert: &[FileEntry],
    ) -> Result<()> {
        if paths_to_remove.is_empty() && entries_to_upsert.is_empty() {
            return Ok(());
        }
        for path in paths_to_remove {
            self.entries.remove(path);
        }
        for entry in entries_to_upsert {
            self.entries.insert(entry.path.clone(), entry.clone());
        }
        self.flush()
    }

    pub fn get(&self, path: &str) -> Result<Option<FileEntry>> {
        Ok(self.entries.get(path).cloned())
    }

    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.entries.remove(path);
        self.flush()
    }

    pub fn all_paths(&self) -> Result<Vec<String>> {
        Ok(self.entries.keys().cloned().collect())
    }

    pub fn all_entries(&self) -> Result<Vec<FileEntry>> {
        Ok(self.entries.values().cloned().collect())
    }

    pub fn entries(&self) -> &HashMap<String, FileEntry> {
        &self.entries
    }

    pub fn entries_by_path(&self) -> Result<HashMap<String, FileEntry>> {
        Ok(self.entries.clone())
    }

    pub fn clear(&mut self) -> Result<()> {
        self.entries.clear();
        self.flush()
    }
}
