use crate::error::{ChkpttError, Result};
use crate::store::blob::hex_to_bytes;
use bitcode::{Decode, Encode};
use memmap2::Mmap;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::PathBuf;
use tempfile::NamedTempFile;

const TREE_PACK_MAGIC: &[u8; 4] = b"CKTL";
const TREE_IDX_ENTRY_SIZE: usize = 16 + 8 + 8; // hash(16) + offset(8) + size(8)
const TREE_HEADER_SIZE: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode)]
pub enum EntryType {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub struct TreeEntry {
    pub name: String,
    pub entry_type: EntryType,
    pub hash: [u8; 16],
    pub size: u64,
    pub mode: u32,
}

/// Index entry for a tree in the pack.
#[derive(Debug, Clone)]
struct TreeIdxEntry {
    hash: [u8; 16],
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

        let (pack_dat, pack_idx, pack_entry_count) = match (
            std::fs::File::open(&dat_path),
            std::fs::File::open(&idx_path),
        ) {
            // SAFETY: files are opened read-only and kept alive alongside the mmaps.
            (Ok(dat_file), Ok(idx_file)) => match (unsafe { Mmap::map(&dat_file) }, unsafe {
                Mmap::map(&idx_file)
            }) {
                (Ok(dat), Ok(idx)) => {
                    #[cfg(unix)]
                    {
                        let _ = dat.advise(memmap2::Advice::Sequential);
                        let _ = idx.advise(memmap2::Advice::Random);
                    }
                    let count = idx.len() / TREE_IDX_ENTRY_SIZE;
                    (Some(dat), Some(idx), count)
                }
                _ => (None, None, 0),
            },
            (Err(dat_error), _) if dat_error.kind() == std::io::ErrorKind::NotFound => {
                (None, None, 0)
            }
            (_, Err(idx_error)) if idx_error.kind() == std::io::ErrorKind::NotFound => {
                (None, None, 0)
            }
            _ => (None, None, 0),
        };

        Self {
            base_dir,
            pack_dat,
            pack_idx,
            pack_entry_count,
        }
    }

    /// Write tree entries (sorted by name). Returns hash hex.
    pub fn write(&self, entries: &[TreeEntry]) -> Result<String> {
        let mut sorted = entries.to_vec();
        sorted.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        let encoded = bitcode::encode(&sorted);
        let hash_bytes = xxhash_rust::xxh3::xxh3_128(&encoded).to_le_bytes();
        let hash_hex = crate::store::blob::bytes_to_hex(&hash_bytes);
        self.write_pack(&[(hash_hex.clone(), encoded)])?;
        Ok(hash_hex)
    }

    /// Write a batch of pre-computed trees to a pack file.
    /// Each entry is (hash_hex, encoded_data).
    pub fn write_pack(&self, entries: &[(String, Vec<u8>)]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        std::fs::create_dir_all(&self.base_dir)?;

        let dat_path = self.base_dir.join("trees.dat");
        let idx_path = self.base_dir.join("trees.idx");

        // Collect existing idx entries
        let mut all_idx_entries: Vec<TreeIdxEntry> = Vec::new();
        let mut existing_hashes: std::collections::HashSet<[u8; 16]> =
            std::collections::HashSet::new();

        let existing_dat_len = if let (Some(dat), Some(idx)) = (&self.pack_dat, &self.pack_idx) {
            for i in 0..self.pack_entry_count {
                let pos = i * TREE_IDX_ENTRY_SIZE;
                let mut hash = [0u8; 16];
                hash.copy_from_slice(&idx[pos..pos + 16]);
                let offset = u64::from_le_bytes(idx[pos + 16..pos + 24].try_into().unwrap());
                let size = u64::from_le_bytes(idx[pos + 24..pos + 32].try_into().unwrap());
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
                // Write: hash(16) + size(8) + data(N)
                writer.write_all(&hash)?;
                writer.write_all(&data_len.to_le_bytes())?;
                writer.write_all(encoded)?;

                all_idx_entries.push(TreeIdxEntry {
                    hash,
                    offset,
                    size: data_len,
                });
                offset += 16 + 8 + data_len;
            }

            writer.flush()?;
        }

        // Write header
        let total_count = all_idx_entries.len() as u32;
        dat_tmp.seek(SeekFrom::Start(0))?;
        dat_tmp.write_all(TREE_PACK_MAGIC)?;
        dat_tmp.write_all(&total_count.to_le_bytes())?;
        dat_tmp.flush()?;

        // Persist .dat
        dat_tmp
            .persist(&dat_path)
            .map_err(|e| ChkpttError::Other(e.error.to_string()))?;

        // Sort idx and write
        all_idx_entries.sort_unstable_by(|a, b| a.hash.cmp(&b.hash));
        let mut idx_buf: Vec<u8> = Vec::with_capacity(all_idx_entries.len() * TREE_IDX_ENTRY_SIZE);
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

    /// Read tree entries by hash from the tree pack.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<TreeEntry>> {
        if let Some(data) = self
            .read_from_pack(hash_hex)
            .or_else(|| self.read_from_current_pack_files(hash_hex))
        {
            let entries: Vec<TreeEntry> = bitcode::decode(&data)?;
            return Ok(entries);
        }

        Err(ChkpttError::ObjectNotFound(hash_hex.to_string()))
    }

    /// Read raw data from the tree pack by hash.
    fn read_from_pack(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let idx = self.pack_idx.as_ref()?;
        let dat = self.pack_dat.as_ref()?;
        Self::read_from_pack_bytes(hash_hex, dat, idx, self.pack_entry_count)
    }

    fn read_from_current_pack_files(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let dat = std::fs::read(self.base_dir.join("trees.dat")).ok()?;
        let idx = std::fs::read(self.base_dir.join("trees.idx")).ok()?;
        let entry_count = idx.len() / TREE_IDX_ENTRY_SIZE;
        Self::read_from_pack_bytes(hash_hex, &dat, &idx, entry_count)
    }

    fn read_from_pack_bytes(
        hash_hex: &str,
        dat: &[u8],
        idx: &[u8],
        entry_count: usize,
    ) -> Option<Vec<u8>> {
        let hash_bytes = hex_to_bytes(hash_hex).ok()?;

        // Binary search in idx
        let mut lo = 0usize;
        let mut hi = entry_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let pos = mid * TREE_IDX_ENTRY_SIZE;
            let mid_hash = &idx[pos..pos + 16];
            match mid_hash.cmp(&hash_bytes) {
                std::cmp::Ordering::Equal => {
                    let offset = u64::from_le_bytes(idx[pos + 16..pos + 24].try_into().unwrap());
                    let size = u64::from_le_bytes(idx[pos + 24..pos + 32].try_into().unwrap());
                    let data_start = offset as usize + 16 + 8;
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
}
