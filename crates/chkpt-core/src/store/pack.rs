use crate::error::{ChkpttError, Result};
use crate::store::blob::BlobStore;
use std::path::Path;

const PACK_MAGIC: &[u8; 4] = b"CHKP";
const PACK_VERSION: u32 = 1;
const IDX_ENTRY_SIZE: usize = 32 + 8 + 8; // hash + offset + size

/// In-memory index entry for a pack.
#[derive(Debug, Clone)]
struct IndexEntry {
    hash: [u8; 32],
    offset: u64,
    size: u64,
}

pub struct PackWriter {
    entries: Vec<(String, Vec<u8>)>, // (hash_hex, compressed_data)
}

impl Default for PackWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl PackWriter {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn add(&mut self, content: &[u8]) -> Result<String> {
        let hash_hex = crate::store::blob::hash_content(content);
        let compressed = zstd::encode_all(content, 3)?;
        self.entries.push((hash_hex.clone(), compressed));
        Ok(hash_hex)
    }

    pub fn add_pre_compressed(&mut self, hash_hex: String, compressed: Vec<u8>) {
        self.entries.push((hash_hex, compressed));
    }

    /// Write pack .dat and .idx files. Returns pack hash.
    pub fn finish(mut self, packs_dir: &Path) -> Result<String> {
        if self.entries.is_empty() {
            return Err(ChkpttError::Other("No entries to pack".into()));
        }

        // Sort entries by hash for binary search in idx
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build .dat
        let mut dat_buf: Vec<u8> = Vec::new();
        dat_buf.extend_from_slice(PACK_MAGIC);
        dat_buf.extend_from_slice(&PACK_VERSION.to_le_bytes());
        dat_buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        let mut idx_entries: Vec<IndexEntry> = Vec::new();

        for (hash_hex, compressed) in &self.entries {
            let hash_bytes = hex_to_bytes(hash_hex)?;
            let offset = dat_buf.len() as u64;
            dat_buf.extend_from_slice(&hash_bytes);
            dat_buf.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
            dat_buf.extend_from_slice(compressed);
            idx_entries.push(IndexEntry {
                hash: hash_bytes,
                offset,
                size: compressed.len() as u64,
            });
        }

        let pack_hash = blake3::hash(&dat_buf).to_hex()[..16].to_string();
        let dat_path = packs_dir.join(format!("pack-{}.dat", pack_hash));
        let idx_path = packs_dir.join(format!("pack-{}.idx", pack_hash));

        // Write .dat
        std::fs::create_dir_all(packs_dir)?;
        std::fs::write(&dat_path, &dat_buf)?;

        // Write .idx (sorted by hash)
        let mut idx_buf: Vec<u8> = Vec::new();
        for entry in &idx_entries {
            idx_buf.extend_from_slice(&entry.hash);
            idx_buf.extend_from_slice(&entry.offset.to_le_bytes());
            idx_buf.extend_from_slice(&entry.size.to_le_bytes());
        }
        std::fs::write(&idx_path, &idx_buf)?;

        Ok(pack_hash)
    }
}

pub struct PackReader {
    dat: Vec<u8>,
    idx: Vec<IndexEntry>,
}

impl PackReader {
    pub fn open(packs_dir: &Path, pack_hash: &str) -> Result<Self> {
        let dat_path = packs_dir.join(format!("pack-{}.dat", pack_hash));
        let idx_path = packs_dir.join(format!("pack-{}.idx", pack_hash));
        let dat = std::fs::read(&dat_path)?;
        let idx_raw = std::fs::read(&idx_path)?;

        let mut idx = Vec::new();
        let mut pos = 0;
        while pos + IDX_ENTRY_SIZE <= idx_raw.len() {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&idx_raw[pos..pos + 32]);
            let offset = u64::from_le_bytes(idx_raw[pos + 32..pos + 40].try_into().unwrap());
            let size = u64::from_le_bytes(idx_raw[pos + 40..pos + 48].try_into().unwrap());
            idx.push(IndexEntry { hash, offset, size });
            pos += IDX_ENTRY_SIZE;
        }

        Ok(Self { dat, idx })
    }

    /// Binary search for hash in index.
    fn find(&self, hash_hex: &str) -> Option<&IndexEntry> {
        let hash_bytes = hex_to_bytes(hash_hex).ok()?;
        self.idx
            .binary_search_by(|e| e.hash.cmp(&hash_bytes))
            .ok()
            .map(|i| &self.idx[i])
    }

    /// Read and decompress an object. Returns None if not found.
    pub fn try_read(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let entry = self.find(hash_hex)?;
        let data_start = entry.offset as usize + 32 + 8; // skip hash + compressed_size
        let data_end = data_start + entry.size as usize;
        if data_end > self.dat.len() {
            return None;
        }
        let compressed = &self.dat[data_start..data_end];
        zstd::decode_all(compressed).ok()
    }

    /// Read and decompress an object. Error if not found.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<u8>> {
        self.try_read(hash_hex)
            .ok_or_else(|| ChkpttError::ObjectNotFound(hash_hex.to_string()))
    }

    /// List all hashes in this pack.
    pub fn hashes(&self) -> Vec<String> {
        self.idx.iter().map(|e| bytes_to_hex(&e.hash)).collect()
    }
}

/// List all pack hashes in a directory.
pub fn list_packs(packs_dir: &Path) -> Result<Vec<String>> {
    let mut packs = Vec::new();
    if !packs_dir.exists() {
        return Ok(packs);
    }
    for entry in std::fs::read_dir(packs_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("pack-") && name.ends_with(".dat") {
            let hash = name.strip_prefix("pack-").unwrap().strip_suffix(".dat").unwrap();
            packs.push(hash.to_string());
        }
    }
    Ok(packs)
}

/// Pack all loose objects from a BlobStore into a pack file, then delete loose objects.
pub fn pack_loose_objects(blob_store: &BlobStore, packs_dir: &Path) -> Result<String> {
    let loose = blob_store.list_loose()?;
    if loose.is_empty() {
        return Err(ChkpttError::Other("No loose objects to pack".into()));
    }
    let mut writer = PackWriter::new();
    for hash in &loose {
        let content = blob_store.read(hash)?;
        // Re-compress from raw content
        writer.add(&content)?;
    }
    let pack_hash = writer.finish(packs_dir)?;
    // Delete loose objects
    for hash in &loose {
        blob_store.remove(hash)?;
    }
    Ok(pack_hash)
}

/// Read an object: first check loose, then packs.
pub fn read_object(blob_store: &BlobStore, packs_dir: &Path, hash_hex: &str) -> Result<Vec<u8>> {
    // 1. Check loose
    if blob_store.exists(hash_hex) {
        return blob_store.read(hash_hex);
    }
    // 2. Check packs
    for pack_hash in list_packs(packs_dir)? {
        let reader = PackReader::open(packs_dir, &pack_hash)?;
        if let Some(data) = reader.try_read(hash_hex) {
            return Ok(data);
        }
    }
    Err(ChkpttError::ObjectNotFound(hash_hex.to_string()))
}

fn hex_to_bytes(hex: &str) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if hex.len() != 64 {
        return Err(ChkpttError::Other(format!("Invalid hash length: {}", hex.len())));
    }
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| ChkpttError::Other("Invalid hex".into()))?;
    }
    Ok(bytes)
}

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
