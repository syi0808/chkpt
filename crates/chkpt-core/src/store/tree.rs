use crate::error::{ChkpttError, Result};
use bitcode::{Decode, Encode};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const TREE_PACK_MAGIC: &[u8; 4] = b"CKTR";
const TREE_PACK_VERSION: u32 = 1;
const TREE_IDX_ENTRY_SIZE: usize = 32 + 8 + 8; // hash(32) + offset(8) + size(8)
const TREE_HEADER_SIZE: u64 = 12;

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

/// Index entry for a tree in the pack.
#[derive(Debug, Clone)]
struct TreeIdxEntry {
    hash: [u8; 32],
    offset: u64,
    size: u64,
}

pub struct TreeStore {
    base_dir: PathBuf,
    /// mmap'd tree pack data file (if exists)
    pack_dat: Option<Mmap>,
    /// mmap'd tree pack index file (if exists)
    pack_idx: Option<Mmap>,
    pack_entry_count: usize,
}

impl TreeStore {
    pub fn new(base_dir: PathBuf) -> Self {
        let dat_path = base_dir.join("trees.dat");
        let idx_path = base_dir.join("trees.idx");

        let (pack_dat, pack_idx, pack_entry_count) =
            if dat_path.exists() && idx_path.exists() {
                match (
                    std::fs::File::open(&dat_path)
                        .and_then(|f| unsafe { Mmap::map(&f) }),
                    std::fs::File::open(&idx_path)
                        .and_then(|f| unsafe { Mmap::map(&f) }),
                ) {
                    (Ok(dat), Ok(idx)) => {
                        let count = idx.len() / TREE_IDX_ENTRY_SIZE;
                        (Some(dat), Some(idx), count)
                    }
                    _ => (None, None, 0),
                }
            } else {
                (None, None, 0)
            };

        Self {
            base_dir,
            pack_dat,
            pack_idx,
            pack_entry_count,
        }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn tree_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base_dir.join(prefix).join(rest)
    }

    /// Write tree entries (sorted by name). Returns hash hex.
    /// Used for single-tree writes (tests, small operations).
    pub fn write(&self, entries: &[TreeEntry]) -> Result<String> {
        let mut sorted = entries.to_vec();
        sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));
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

    /// Write a batch of pre-computed trees to a pack file.
    /// Each entry is (hash_hex, encoded_data).
    pub fn write_pack(
        &self,
        entries: &[(String, Vec<u8>)],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.base_dir)?;

        let dat_path = self.base_dir.join("trees.dat");
        let idx_path = self.base_dir.join("trees.idx");

        // Collect existing idx entries
        let mut all_idx_entries: Vec<TreeIdxEntry> = Vec::new();
        let mut existing_hashes: std::collections::HashSet<[u8; 32]> =
            std::collections::HashSet::new();

        let existing_dat_len = if let (Some(dat), Some(idx)) = (&self.pack_dat, &self.pack_idx) {
            for i in 0..self.pack_entry_count {
                let pos = i * TREE_IDX_ENTRY_SIZE;
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&idx[pos..pos + 32]);
                let offset =
                    u64::from_le_bytes(idx[pos + 32..pos + 40].try_into().unwrap());
                let size =
                    u64::from_le_bytes(idx[pos + 40..pos + 48].try_into().unwrap());
                existing_hashes.insert(hash);
                all_idx_entries.push(TreeIdxEntry { hash, offset, size });
            }
            dat.len() as u64
        } else {
            TREE_HEADER_SIZE
        };

        // Write new .dat
        let mut dat_tmp = NamedTempFile::new_in(&self.base_dir)?;
        {
            let mut writer = BufWriter::with_capacity(256 * 1024, &mut dat_tmp);

            if let Some(dat) = &self.pack_dat {
                writer.write_all(dat)?;
            } else {
                writer.write_all(&[0u8; TREE_HEADER_SIZE as usize])?;
            }

            let mut offset = existing_dat_len;

            for (hash_hex, encoded) in entries {
                let hash = hex_to_bytes(hash_hex)?;
                if existing_hashes.contains(&hash) {
                    continue;
                }
                let data_len = encoded.len() as u64;
                // Write: hash(32) + size(8) + data(N)
                writer.write_all(&hash)?;
                writer.write_all(&data_len.to_le_bytes())?;
                writer.write_all(encoded)?;

                all_idx_entries.push(TreeIdxEntry {
                    hash,
                    offset,
                    size: data_len,
                });
                offset += 32 + 8 + data_len;
            }

            writer.flush()?;
        }

        // Write header
        let total_count = all_idx_entries.len() as u32;
        dat_tmp.seek(SeekFrom::Start(0))?;
        dat_tmp.write_all(TREE_PACK_MAGIC)?;
        dat_tmp.write_all(&TREE_PACK_VERSION.to_le_bytes())?;
        dat_tmp.write_all(&total_count.to_le_bytes())?;
        dat_tmp.flush()?;

        // Persist .dat
        dat_tmp
            .persist(&dat_path)
            .map_err(|e| ChkpttError::Other(e.error.to_string()))?;

        // Sort idx and write
        all_idx_entries.sort_unstable_by(|a, b| a.hash.cmp(&b.hash));
        let mut idx_buf: Vec<u8> =
            Vec::with_capacity(all_idx_entries.len() * TREE_IDX_ENTRY_SIZE);
        for entry in &all_idx_entries {
            idx_buf.extend_from_slice(&entry.hash);
            idx_buf.extend_from_slice(&entry.offset.to_le_bytes());
            idx_buf.extend_from_slice(&entry.size.to_le_bytes());
        }
        let idx_tmp_path = idx_path.with_extension("idx.tmp");
        std::fs::write(&idx_tmp_path, &idx_buf)?;
        std::fs::rename(&idx_tmp_path, &idx_path)?;

        Ok(())
    }

    /// Read tree entries by hash. Checks pack first, then loose files.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<TreeEntry>> {
        // Check pack first
        if let Some(data) = self.read_from_pack(hash_hex) {
            let entries: Vec<TreeEntry> = bitcode::decode(&data)?;
            return Ok(entries);
        }

        // Fall back to loose file
        let path = self.tree_path(hash_hex);
        if !path.exists() {
            return Err(ChkpttError::ObjectNotFound(hash_hex.to_string()));
        }
        let data = std::fs::read(&path)?;
        let entries: Vec<TreeEntry> = bitcode::decode(&data)?;
        Ok(entries)
    }

    /// Read raw data from the tree pack by hash.
    fn read_from_pack(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let idx = self.pack_idx.as_ref()?;
        let dat = self.pack_dat.as_ref()?;
        let hash_bytes = hex_to_bytes(hash_hex).ok()?;

        // Binary search in idx
        let mut lo = 0usize;
        let mut hi = self.pack_entry_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let pos = mid * TREE_IDX_ENTRY_SIZE;
            let mid_hash = &idx[pos..pos + 32];
            match mid_hash.cmp(&hash_bytes) {
                std::cmp::Ordering::Equal => {
                    let offset =
                        u64::from_le_bytes(idx[pos + 32..pos + 40].try_into().unwrap());
                    let size =
                        u64::from_le_bytes(idx[pos + 40..pos + 48].try_into().unwrap());
                    let data_start = offset as usize + 32 + 8;
                    let data_end = data_start + size as usize;
                    if data_end > dat.len() {
                        return None;
                    }
                    return Some(dat[data_start..data_end].to_vec());
                }
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    pub fn exists(&self, hash_hex: &str) -> bool {
        // Check pack
        if self.read_from_pack(hash_hex).is_some() {
            return true;
        }
        // Check loose
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

fn hex_to_bytes(hex: &str) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if hex.len() != 64 {
        return Err(ChkpttError::Other(format!(
            "Invalid hash length: {}",
            hex.len()
        )));
    }
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| ChkpttError::Other("Invalid hex".into()))?;
    }
    Ok(bytes)
}
