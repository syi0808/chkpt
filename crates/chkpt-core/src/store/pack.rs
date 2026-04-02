use crate::error::{ChkpttError, Result};
use crate::store::blob::hash_content;
use memmap2::Mmap;
use std::collections::HashMap;
use std::io::{BufWriter, Cursor, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const PACK_MAGIC: &[u8; 4] = b"CHKP";
const PACK_VERSION: u32 = 1;
const IDX_ENTRY_SIZE: usize = 32 + 8 + 8; // hash + offset + size
const HEADER_SIZE: u64 = 12; // MAGIC(4) + VERSION(4) + COUNT(4)

/// In-memory index entry for a pack.
#[derive(Debug, Clone)]
struct IndexEntry {
    hash: [u8; 32],
    offset: u64,
    size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackLocation {
    pub(crate) reader_index: usize,
    pub(crate) offset: u64,
    pub(crate) size: u64,
}

pub struct PackWriter {
    writer: BufWriter<NamedTempFile>,
    hasher: blake3::Hasher,
    idx_entries: Vec<IndexEntry>,
    offset: u64,
    packs_dir: PathBuf,
}

impl PackWriter {
    pub fn new(packs_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(packs_dir)?;
        let dat_tmp = NamedTempFile::new_in(packs_dir)?;
        let mut writer = BufWriter::with_capacity(256 * 1024, dat_tmp);
        // Write 12-byte placeholder header (will be overwritten in finish)
        let placeholder = [0u8; HEADER_SIZE as usize];
        writer.write_all(&placeholder)?;
        // Start incremental hasher — header will be re-hashed in finish()
        let hasher = blake3::Hasher::new();
        Ok(Self {
            writer,
            hasher,
            idx_entries: Vec::new(),
            offset: HEADER_SIZE,
            packs_dir: packs_dir.to_path_buf(),
        })
    }

    pub fn add(&mut self, content: &[u8]) -> Result<String> {
        let hash_hex = hash_content(content);
        let hash = hex_to_bytes(&hash_hex)?;
        let compressed = zstd::encode_all(content, 1)?;
        self.add_pre_compressed_bytes(hash, compressed)?;
        Ok(hash_hex)
    }

    pub fn add_pre_compressed(&mut self, hash_hex: String, compressed: Vec<u8>) -> Result<()> {
        let hash = hex_to_bytes(&hash_hex)?;
        self.add_pre_compressed_bytes(hash, compressed)
    }

    pub fn add_pre_compressed_bytes(&mut self, hash: [u8; 32], compressed: Vec<u8>) -> Result<()> {
        let compressed_len = compressed.len() as u64;

        // Write entry to BufWriter: hash(32) + compressed_len(8) + data(N)
        self.writer.write_all(&hash)?;
        self.writer.write_all(&compressed_len.to_le_bytes())?;
        self.writer.write_all(&compressed)?;

        // Incremental hash of entry data
        self.hasher.update(&hash);
        self.hasher.update(&compressed_len.to_le_bytes());
        self.hasher.update(&compressed);

        self.idx_entries.push(IndexEntry {
            hash,
            offset: self.offset,
            size: compressed_len,
        });
        self.offset += 32 + 8 + compressed_len;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.idx_entries.is_empty()
    }

    /// Finalize: write real header, persist .dat, write .idx.
    /// Returns pack hash.
    pub fn finish(mut self) -> Result<String> {
        if self.idx_entries.is_empty() {
            return Err(ChkpttError::Other("No entries to pack".into()));
        }

        // Flush BufWriter and get the underlying file
        self.writer.flush()?;
        let mut dat_tmp = self.writer.into_inner().map_err(|e| e.into_error())?;

        let count = self.idx_entries.len() as u32;

        // Write real header
        let mut header = [0u8; HEADER_SIZE as usize];
        header[0..4].copy_from_slice(PACK_MAGIC);
        header[4..8].copy_from_slice(&PACK_VERSION.to_le_bytes());
        header[8..12].copy_from_slice(&count.to_le_bytes());

        dat_tmp.seek(SeekFrom::Start(0))?;
        dat_tmp.write_all(&header)?;
        dat_tmp.flush()?;

        // Finalize hash: include header in the hash
        self.hasher.update(&header);
        let pack_hash = self.hasher.finalize().to_hex()[..16].to_string();

        // Persist .dat file
        let dat_path = self.packs_dir.join(format!("pack-{}.dat", pack_hash));
        if let Err(error) = dat_tmp.persist_noclobber(&dat_path) {
            if error.error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(ChkpttError::Other(error.error.to_string()));
            }
        }

        // Sort idx entries by hash for binary search
        self.idx_entries
            .sort_unstable_by(|a, b| a.hash.cmp(&b.hash));

        // Write .idx file
        let idx_path = self.packs_dir.join(format!("pack-{}.idx", pack_hash));
        let mut idx_buf: Vec<u8> = Vec::with_capacity(self.idx_entries.len() * IDX_ENTRY_SIZE);
        for entry in &self.idx_entries {
            idx_buf.extend_from_slice(&entry.hash);
            idx_buf.extend_from_slice(&entry.offset.to_le_bytes());
            idx_buf.extend_from_slice(&entry.size.to_le_bytes());
        }
        std::fs::write(&idx_path, &idx_buf)?;

        Ok(pack_hash)
    }
}

pub struct PackReader {
    _dat_file: std::fs::File,
    dat: Mmap,
    _idx_file: std::fs::File,
    idx: Mmap,
    entry_count: usize,
}

impl PackReader {
    pub fn open(packs_dir: &Path, pack_hash: &str) -> Result<Self> {
        let dat_path = packs_dir.join(format!("pack-{}.dat", pack_hash));
        let idx_path = packs_dir.join(format!("pack-{}.idx", pack_hash));

        let dat_file = std::fs::File::open(&dat_path)?;
        let idx_file = std::fs::File::open(&idx_path)?;

        // SAFETY: The file handles are kept alive alongside the mmaps.
        let dat = unsafe { Mmap::map(&dat_file)? };
        let idx = unsafe { Mmap::map(&idx_file)? };

        let entry_count = idx.len() / IDX_ENTRY_SIZE;

        Ok(Self {
            _dat_file: dat_file,
            dat,
            _idx_file: idx_file,
            idx,
            entry_count,
        })
    }

    /// Extract an IndexEntry from the mmap'd idx at a given index position.
    fn idx_entry(&self, index: usize) -> IndexEntry {
        let pos = index * IDX_ENTRY_SIZE;
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&self.idx[pos..pos + 32]);
        let offset = u64::from_le_bytes(self.idx[pos + 32..pos + 40].try_into().unwrap());
        let size = u64::from_le_bytes(self.idx[pos + 40..pos + 48].try_into().unwrap());
        IndexEntry { hash, offset, size }
    }

    /// Binary search for hash in the mmap'd index.
    fn find_bytes(&self, hash_bytes: &[u8; 32]) -> Option<IndexEntry> {
        let mut lo = 0usize;
        let mut hi = self.entry_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_hash = &self.idx[mid * IDX_ENTRY_SIZE..mid * IDX_ENTRY_SIZE + 32];
            match mid_hash.cmp(&hash_bytes[..]) {
                std::cmp::Ordering::Equal => return Some(self.idx_entry(mid)),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    fn find(&self, hash_hex: &str) -> Option<IndexEntry> {
        let hash_bytes = hex_to_bytes(hash_hex).ok()?;
        self.find_bytes(&hash_bytes)
    }

    pub fn contains_bytes(&self, hash: &[u8; 32]) -> bool {
        self.find_bytes(hash).is_some()
    }

    fn compressed_bytes(&self, offset: u64, size: u64) -> Option<&[u8]> {
        let data_start = (offset as usize).checked_add(32 + 8)?; // skip hash + compressed_size
        let data_end = data_start.checked_add(size as usize)?;
        if data_end > self.dat.len() {
            return None;
        }
        Some(&self.dat[data_start..data_end])
    }

    fn copy_at<W: Write>(&self, offset: u64, size: u64, mut writer: W) -> Result<()> {
        let compressed = self.compressed_bytes(offset, size).ok_or_else(|| {
            ChkpttError::StoreCorrupted("Pack entry points outside pack data".into())
        })?;
        zstd::stream::copy_decode(Cursor::new(compressed), &mut writer)?;
        Ok(())
    }

    /// Read and decompress an object. Returns None if not found.
    pub fn try_read(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let entry = self.find(hash_hex)?;
        let mut decompressed = Vec::new();
        self.copy_at(entry.offset, entry.size, &mut decompressed)
            .ok()?;
        Some(decompressed)
    }

    /// Read and decompress an object. Error if not found.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<u8>> {
        self.try_read(hash_hex)
            .ok_or_else(|| ChkpttError::ObjectNotFound(hash_hex.to_string()))
    }
}

pub struct PackSet {
    readers: Vec<PackReader>,
    reader_indices: HashMap<String, usize>,
}

impl PackSet {
    pub fn open_all(packs_dir: &Path) -> Result<Self> {
        let pack_hashes = list_packs(packs_dir)?;
        Self::open_selected(packs_dir, &pack_hashes)
    }

    pub fn open_selected(packs_dir: &Path, pack_hashes: &[String]) -> Result<Self> {
        let mut readers = Vec::with_capacity(pack_hashes.len());
        let mut reader_indices = HashMap::with_capacity(pack_hashes.len());
        for pack_hash in pack_hashes {
            let reader_index = readers.len();
            readers.push(PackReader::open(packs_dir, pack_hash)?);
            reader_indices.insert(pack_hash.clone(), reader_index);
        }
        Ok(Self {
            readers,
            reader_indices,
        })
    }

    pub fn empty() -> Self {
        Self {
            readers: Vec::new(),
            reader_indices: HashMap::new(),
        }
    }

    pub fn try_read(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let location = self.locate(hash_hex)?;
        let mut decompressed = Vec::new();
        self.copy_to_writer(&location, &mut decompressed).ok()?;
        Some(decompressed)
    }

    pub fn contains_bytes(&self, hash: &[u8; 32]) -> bool {
        self.readers
            .iter()
            .any(|reader| reader.contains_bytes(hash))
    }

    pub fn read(&self, hash_hex: &str) -> Result<Vec<u8>> {
        self.try_read(hash_hex)
            .ok_or_else(|| ChkpttError::ObjectNotFound(hash_hex.to_string()))
    }

    pub(crate) fn locate(&self, hash_hex: &str) -> Option<PackLocation> {
        self.readers
            .iter()
            .enumerate()
            .find_map(|(reader_index, reader)| {
                reader.find(hash_hex).map(|entry| PackLocation {
                    reader_index,
                    offset: entry.offset,
                    size: entry.size,
                })
            })
    }

    pub(crate) fn locate_in_pack_bytes(
        &self,
        pack_hash: &str,
        hash: &[u8; 32],
    ) -> Option<PackLocation> {
        let reader_index = *self.reader_indices.get(pack_hash)?;
        let reader = self.readers.get(reader_index)?;
        reader.find_bytes(hash).map(|entry| PackLocation {
            reader_index,
            offset: entry.offset,
            size: entry.size,
        })
    }

    pub(crate) fn copy_to_writer<W: Write>(
        &self,
        location: &PackLocation,
        writer: W,
    ) -> Result<()> {
        let reader = self.readers.get(location.reader_index).ok_or_else(|| {
            ChkpttError::StoreCorrupted(format!(
                "Pack reader index {} is out of range",
                location.reader_index
            ))
        })?;
        reader.copy_at(location.offset, location.size, writer)
    }
}

/// List all pack hashes in a directory.
pub fn list_packs(packs_dir: &Path) -> Result<Vec<String>> {
    let mut packs = Vec::new();
    let entries = match std::fs::read_dir(packs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(packs),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("pack-") && name.ends_with(".dat") {
            let hash = name
                .strip_prefix("pack-")
                .unwrap()
                .strip_suffix(".dat")
                .unwrap();
            packs.push(hash.to_owned());
        }
    }
    Ok(packs)
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
