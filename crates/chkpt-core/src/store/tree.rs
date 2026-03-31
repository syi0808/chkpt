use crate::error::{ChkpttError, Result};
use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Encode, Decode)]
pub enum EntryType {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Encode, Decode)]
pub struct TreeEntry {
    pub name: String,
    pub entry_type: EntryType,
    pub hash: [u8; 32],
    pub size: u64,
    pub mode: u32,
}

pub struct TreeStore {
    base_dir: PathBuf,
}

impl TreeStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn tree_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base_dir.join(prefix).join(rest)
    }

    /// Write tree entries (sorted by name). Returns hash hex.
    pub fn write(&self, entries: &[TreeEntry]) -> Result<String> {
        let mut sorted = entries.to_vec();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        let encoded = bitcode::encode(&sorted);
        let hash_hex = blake3::hash(&encoded).to_hex().to_string();
        let path = self.tree_path(&hash_hex);
        if path.exists() {
            return Ok(hash_hex);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &encoded)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(hash_hex)
    }

    /// Read tree entries by hash.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<TreeEntry>> {
        let path = self.tree_path(hash_hex);
        if !path.exists() {
            return Err(ChkpttError::ObjectNotFound(hash_hex.to_string()));
        }
        let data = std::fs::read(&path)?;
        let entries: Vec<TreeEntry> = bitcode::decode(&data)?;
        Ok(entries)
    }

    pub fn exists(&self, hash_hex: &str) -> bool {
        self.tree_path(hash_hex).exists()
    }

    pub fn remove(&self, hash_hex: &str) -> Result<()> {
        let path = self.tree_path(hash_hex);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// List all loose tree hashes.
    pub fn list_loose(&self) -> Result<Vec<String>> {
        let mut hashes = Vec::new();
        if !self.base_dir.exists() {
            return Ok(hashes);
        }
        for prefix_entry in std::fs::read_dir(&self.base_dir)? {
            let prefix_entry = prefix_entry?;
            if !prefix_entry.file_type()?.is_dir() {
                continue;
            }
            let prefix = prefix_entry.file_name().to_string_lossy().to_string();
            for entry in std::fs::read_dir(prefix_entry.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let rest = entry.file_name().to_string_lossy().to_string();
                    if !rest.ends_with(".tmp") {
                        hashes.push(format!("{}{}", prefix, rest));
                    }
                }
            }
        }
        Ok(hashes)
    }
}
