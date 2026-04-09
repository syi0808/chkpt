use crate::error::{ChkpttError, Result};
use crate::store::blob::{hash_content, hex_to_bytes};
use memmap2::Mmap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

const PACK_MAGIC: &[u8; 4] = b"CHKL";
const IDX_ENTRY_SIZE: usize = 16 + 8 + 8; // hash + offset + size
const HEADER_SIZE: u64 = 8; // MAGIC(4) + COUNT(4)
const PART_READ_BUFFER_SIZE: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PackFinishOptions {
    pub chunk_bytes: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PackPartsManifest {
    version: u32,
    pack_hash: String,
    dat_size: u64,
    chunk_bytes: u64,
    parts: Vec<PackPartManifestEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PackPartManifestEntry {
    path: String,
    offset: u64,
    size: u64,
}

/// In-memory index entry for a pack.
#[derive(Debug, Clone)]
struct IndexEntry {
    hash: [u8; 16],
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
    hasher: xxhash_rust::xxh3::Xxh3,
    idx_entries: Vec<IndexEntry>,
    offset: u64,
    packs_dir: PathBuf,
}

impl PackWriter {
    pub fn new(packs_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(packs_dir)?;
        let dat_tmp = NamedTempFile::new_in(packs_dir)?;
        let mut writer = BufWriter::with_capacity(256 * 1024, dat_tmp);
        // Write 8-byte placeholder header (will be overwritten in finish)
        let placeholder = [0u8; HEADER_SIZE as usize];
        writer.write_all(&placeholder)?;
        // Start incremental hasher — header will be re-hashed in finish()
        let hasher = xxhash_rust::xxh3::Xxh3::new();
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
        let compressed = {
            use lz4_flex::frame::FrameEncoder;
            let mut encoder = FrameEncoder::new(Vec::new());
            std::io::Write::write_all(&mut encoder, content).unwrap();
            encoder.finish().unwrap()
        };
        self.add_pre_compressed_bytes(hash, compressed)?;
        Ok(hash_hex)
    }

    pub fn add_pre_compressed(&mut self, hash_hex: String, compressed: Vec<u8>) -> Result<()> {
        let hash = hex_to_bytes(&hash_hex)?;
        self.add_pre_compressed_bytes(hash, compressed)
    }

    pub fn add_pre_compressed_bytes(&mut self, hash: [u8; 16], compressed: Vec<u8>) -> Result<()> {
        let compressed_len = compressed.len() as u64;

        // Write entry to BufWriter: hash(16) + compressed_len(8) + data(N)
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
        self.offset += 16 + 8 + compressed_len;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.idx_entries.is_empty()
    }

    /// Finalize: write real header, persist .dat, write .idx.
    /// Returns pack hash.
    pub fn finish(self) -> Result<String> {
        self.finish_with_options(PackFinishOptions::default())
    }

    /// Finalize the pack and optionally split the resulting .dat file into
    /// contiguous part files. The existing unchunked finish path is the default.
    pub fn finish_with_options(mut self, options: PackFinishOptions) -> Result<String> {
        if self.idx_entries.is_empty() {
            return Err(ChkpttError::Other("No entries to pack".into()));
        }
        if options.chunk_bytes == Some(0) {
            return Err(ChkpttError::Other(
                "pack chunk size must be greater than zero".into(),
            ));
        }

        // Flush BufWriter and get the underlying file
        self.writer.flush()?;
        let mut dat_tmp = self.writer.into_inner().map_err(|e| e.into_error())?;

        let count = self.idx_entries.len() as u32;

        // Write real header
        let mut header = [0u8; HEADER_SIZE as usize];
        header[0..4].copy_from_slice(PACK_MAGIC);
        header[4..8].copy_from_slice(&count.to_le_bytes());

        dat_tmp.seek(SeekFrom::Start(0))?;
        dat_tmp.write_all(&header)?;
        dat_tmp.flush()?;

        // Finalize hash: include header in the hash
        self.hasher.update(&header);
        let pack_hash = format!("{:032x}", self.hasher.digest128())[..16].to_string();

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

        if let Some(chunk_bytes) = options.chunk_bytes {
            split_pack_dat_file(&self.packs_dir, &pack_hash, chunk_bytes)?;
        }

        Ok(pack_hash)
    }
}

fn split_pack_dat_file(packs_dir: &Path, pack_hash: &str, chunk_bytes: u64) -> Result<()> {
    if chunk_bytes == 0 {
        return Err(ChkpttError::Other(
            "pack chunk size must be greater than zero".into(),
        ));
    }

    let dat_path = pack_dat_path(packs_dir, pack_hash);
    let dat_file = File::open(&dat_path)?;
    let dat_size = dat_file.metadata()?.len();
    let mut reader = BufReader::with_capacity(PART_READ_BUFFER_SIZE, dat_file);
    let mut buffer = vec![0u8; (chunk_bytes as usize).min(PART_READ_BUFFER_SIZE).max(1)];

    let mut offset = 0u64;
    let mut part_index = 0usize;
    let mut parts = Vec::new();

    while offset < dat_size {
        let part_file_name = pack_part_file_name(pack_hash, part_index);
        let part_path = packs_dir.join(&part_file_name);
        let mut part_tmp = NamedTempFile::new_in(packs_dir)?;
        let mut remaining = (dat_size - offset).min(chunk_bytes);
        let part_offset = offset;

        while remaining > 0 {
            let read_len = (remaining as usize).min(buffer.len());
            reader.read_exact(&mut buffer[..read_len])?;
            part_tmp.write_all(&buffer[..read_len])?;
            remaining -= read_len as u64;
            offset += read_len as u64;
        }
        part_tmp.flush()?;
        part_tmp
            .persist(&part_path)
            .map_err(|error| ChkpttError::Other(error.error.to_string()))?;

        parts.push(PackPartManifestEntry {
            path: part_file_name,
            offset: part_offset,
            size: offset - part_offset,
        });
        part_index += 1;
    }

    let manifest = PackPartsManifest {
        version: 1,
        pack_hash: pack_hash.to_string(),
        dat_size,
        chunk_bytes,
        parts,
    };
    let manifest_path = pack_parts_manifest_path(packs_dir, pack_hash);
    let mut manifest_tmp = NamedTempFile::new_in(packs_dir)?;
    serde_json::to_writer(&mut manifest_tmp, &manifest)
        .map_err(|error| ChkpttError::Other(error.to_string()))?;
    manifest_tmp.write_all(b"\n")?;
    manifest_tmp.flush()?;
    manifest_tmp
        .persist(&manifest_path)
        .map_err(|error| ChkpttError::Other(error.error.to_string()))?;

    std::fs::remove_file(&dat_path)?;
    Ok(())
}

enum PackData {
    SingleFile { _dat_file: File, dat: Mmap },
    Chunked(ChunkedPackData),
}

struct ChunkedPackData {
    dat_size: u64,
    _part_files: Vec<File>,
    parts: Vec<PackPartData>,
}

struct PackPartData {
    offset: u64,
    size: u64,
    dat: Mmap,
}

struct ChunkedRangeReader<'a> {
    parts: &'a [PackPartData],
    part_index: usize,
    position: u64,
    end: u64,
}

impl<'a> ChunkedRangeReader<'a> {
    fn new(parts: &'a [PackPartData], offset: u64, size: u64) -> Self {
        let part_index = parts.partition_point(|part| offset >= part.offset + part.size);
        Self {
            parts,
            part_index,
            position: offset,
            end: offset + size,
        }
    }
}

impl Read for ChunkedRangeReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() || self.position >= self.end {
            return Ok(0);
        }

        while self.part_index < self.parts.len() {
            let part = &self.parts[self.part_index];
            let part_end = part.offset + part.size;
            if self.position >= part_end {
                self.part_index += 1;
                continue;
            }
            if self.position < part.offset {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "gap in chunked pack data",
                ));
            }

            let in_part_offset = (self.position - part.offset) as usize;
            let available_in_part = (part.size as usize).saturating_sub(in_part_offset);
            let remaining_in_range = (self.end - self.position) as usize;
            let to_copy = output.len().min(available_in_part).min(remaining_in_range);
            if to_copy == 0 {
                self.part_index += 1;
                continue;
            }

            output[..to_copy].copy_from_slice(&part.dat[in_part_offset..in_part_offset + to_copy]);
            self.position += to_copy as u64;
            return Ok(to_copy);
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "chunked pack data ended before requested range",
        ))
    }
}

pub struct PackReader {
    data: PackData,
    _idx_file: File,
    idx: Mmap,
    entry_count: usize,
}

impl PackReader {
    pub fn open(packs_dir: &Path, pack_hash: &str) -> Result<Self> {
        let dat_path = pack_dat_path(packs_dir, pack_hash);
        let idx_path = pack_idx_path(packs_dir, pack_hash);

        let data = match File::open(&dat_path) {
            Ok(dat_file) => {
                // SAFETY: The file handles are kept alive alongside the mmaps.
                let dat = unsafe { Mmap::map(&dat_file)? };
                PackData::SingleFile {
                    _dat_file: dat_file,
                    dat,
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                PackData::Chunked(open_chunked_pack_data(packs_dir, pack_hash)?)
            }
            Err(error) => return Err(error.into()),
        };
        let idx_file = File::open(&idx_path)?;

        // SAFETY: The file handle is kept alive alongside the mmap.
        let idx = unsafe { Mmap::map(&idx_file)? };

        // Hint kernel about expected access patterns.
        // .dat uses WillNeed (not Sequential) because parallel restore workers
        // read different regions of the same mmap concurrently — Sequential
        // causes aggressive page reclaim that hurts other threads.
        // .idx is binary-searched so Random is appropriate.
        #[cfg(unix)]
        {
            match &data {
                PackData::SingleFile { dat, .. } => {
                    let _ = dat.advise(memmap2::Advice::WillNeed);
                }
                PackData::Chunked(chunked) => {
                    for part in &chunked.parts {
                        let _ = part.dat.advise(memmap2::Advice::WillNeed);
                    }
                }
            }
            let _ = idx.advise(memmap2::Advice::Random);
        }

        let entry_count = idx.len() / IDX_ENTRY_SIZE;

        Ok(Self {
            data,
            _idx_file: idx_file,
            idx,
            entry_count,
        })
    }

    /// Extract an IndexEntry from the mmap'd idx at a given index position.
    fn idx_entry(&self, index: usize) -> IndexEntry {
        let pos = index * IDX_ENTRY_SIZE;
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&self.idx[pos..pos + 16]);
        let offset = u64::from_le_bytes(self.idx[pos + 16..pos + 24].try_into().unwrap());
        let size = u64::from_le_bytes(self.idx[pos + 24..pos + 32].try_into().unwrap());
        IndexEntry { hash, offset, size }
    }

    /// Binary search for hash in the mmap'd index.
    fn find_bytes(&self, hash_bytes: &[u8; 16]) -> Option<IndexEntry> {
        let mut lo = 0usize;
        let mut hi = self.entry_count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let mid_hash = &self.idx[mid * IDX_ENTRY_SIZE..mid * IDX_ENTRY_SIZE + 16];
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

    pub fn contains_bytes(&self, hash: &[u8; 16]) -> bool {
        self.find_bytes(hash).is_some()
    }

    fn single_file_compressed_bytes(dat: &[u8], offset: u64, size: u64) -> Option<&[u8]> {
        let data_start = (offset as usize).checked_add(16 + 8)?; // skip hash + compressed_size
        let data_end = data_start.checked_add(size as usize)?;
        if data_end > dat.len() {
            return None;
        }
        Some(&dat[data_start..data_end])
    }

    fn copy_at<W: Write>(&self, offset: u64, size: u64, mut writer: W) -> Result<()> {
        match &self.data {
            PackData::SingleFile { dat, .. } => {
                let compressed =
                    Self::single_file_compressed_bytes(dat, offset, size).ok_or_else(|| {
                        ChkpttError::StoreCorrupted("Pack entry points outside pack data".into())
                    })?;
                copy_lz4_to_writer(compressed, &mut writer)?;
            }
            PackData::Chunked(chunked) => {
                let data_start = offset.checked_add(16 + 8).ok_or_else(|| {
                    ChkpttError::StoreCorrupted("Pack entry offset overflows".into())
                })?;
                let data_end = data_start.checked_add(size).ok_or_else(|| {
                    ChkpttError::StoreCorrupted("Pack entry size overflows".into())
                })?;
                if data_end > chunked.dat_size {
                    return Err(ChkpttError::StoreCorrupted(
                        "Pack entry points outside chunked pack data".into(),
                    ));
                }
                let compressed = ChunkedRangeReader::new(&chunked.parts, data_start, size);
                copy_lz4_to_writer(compressed, &mut writer)?;
            }
        }
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

    pub fn contains_bytes(&self, hash: &[u8; 16]) -> bool {
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

    pub fn locate_bytes(&self, hash: &[u8; 16]) -> Option<PackLocation> {
        self.readers
            .iter()
            .enumerate()
            .find_map(|(reader_index, reader)| {
                reader.find_bytes(hash).map(|entry| PackLocation {
                    reader_index,
                    offset: entry.offset,
                    size: entry.size,
                })
            })
    }

    pub(crate) fn locate_in_pack_bytes(
        &self,
        pack_hash: &str,
        hash: &[u8; 16],
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

fn pack_dat_path(packs_dir: &Path, pack_hash: &str) -> PathBuf {
    packs_dir.join(format!("pack-{}.dat", pack_hash))
}

fn pack_idx_path(packs_dir: &Path, pack_hash: &str) -> PathBuf {
    packs_dir.join(format!("pack-{}.idx", pack_hash))
}

fn pack_parts_manifest_path(packs_dir: &Path, pack_hash: &str) -> PathBuf {
    packs_dir.join(format!("pack-{}.dat.parts.json", pack_hash))
}

fn pack_part_file_name(pack_hash: &str, part_index: usize) -> String {
    format!("pack-{}.dat.part-{:06}", pack_hash, part_index)
}

fn copy_lz4_to_writer<R: Read, W: Write>(compressed: R, mut writer: W) -> Result<()> {
    let mut decoder = lz4_flex::frame::FrameDecoder::new(compressed);
    std::io::copy(&mut decoder, &mut writer).map_err(|e| {
        if e.kind() == std::io::ErrorKind::InvalidData {
            ChkpttError::StoreCorrupted(format!("LZ4 decompression failed: {}", e))
        } else {
            ChkpttError::Io(e)
        }
    })?;
    Ok(())
}

fn open_chunked_pack_data(packs_dir: &Path, pack_hash: &str) -> Result<ChunkedPackData> {
    let manifest_path = pack_parts_manifest_path(packs_dir, pack_hash);
    let manifest_file = File::open(&manifest_path)?;
    let manifest: PackPartsManifest = serde_json::from_reader(BufReader::new(manifest_file))
        .map_err(|error| {
            ChkpttError::StoreCorrupted(format!(
                "Pack parts manifest {} is invalid JSON: {}",
                manifest_path.display(),
                error
            ))
        })?;

    if manifest.version != 1 {
        return Err(ChkpttError::StoreCorrupted(format!(
            "Unsupported pack parts manifest version {}",
            manifest.version
        )));
    }
    if manifest.pack_hash != pack_hash {
        return Err(ChkpttError::StoreCorrupted(format!(
            "Pack parts manifest hash {} does not match requested pack {}",
            manifest.pack_hash, pack_hash
        )));
    }
    if manifest.dat_size < HEADER_SIZE {
        return Err(ChkpttError::StoreCorrupted(format!(
            "Pack parts manifest data size {} is smaller than pack header",
            manifest.dat_size
        )));
    }
    if manifest.chunk_bytes == 0 {
        return Err(ChkpttError::StoreCorrupted(
            "Pack parts manifest has a zero chunk size".into(),
        ));
    }

    let mut next_offset = 0u64;
    let mut part_files = Vec::with_capacity(manifest.parts.len());
    let mut parts = Vec::with_capacity(manifest.parts.len());
    for part in manifest.parts {
        if part.offset != next_offset {
            return Err(ChkpttError::StoreCorrupted(format!(
                "Pack parts manifest has a gap before offset {}",
                next_offset
            )));
        }
        if part.size == 0 {
            return Err(ChkpttError::StoreCorrupted(
                "Pack parts manifest contains an empty part".into(),
            ));
        }

        let relative_path = Path::new(&part.path);
        if relative_path.is_absolute() || relative_path.components().count() != 1 {
            return Err(ChkpttError::StoreCorrupted(format!(
                "Pack parts manifest contains an invalid part path: {}",
                part.path
            )));
        }

        let part_path = packs_dir.join(relative_path);
        let part_file = File::open(&part_path)?;
        let actual_size = part_file.metadata()?.len();
        if actual_size != part.size {
            return Err(ChkpttError::StoreCorrupted(format!(
                "Pack part {} has size {}, expected {}",
                part_path.display(),
                actual_size,
                part.size
            )));
        }

        // SAFETY: The part file handles are kept alive in ChunkedPackData.
        let dat = unsafe { Mmap::map(&part_file)? };
        parts.push(PackPartData {
            offset: part.offset,
            size: part.size,
            dat,
        });
        part_files.push(part_file);
        next_offset += part.size;
    }

    if next_offset != manifest.dat_size {
        return Err(ChkpttError::StoreCorrupted(format!(
            "Pack parts manifest covers {} bytes, expected {}",
            next_offset, manifest.dat_size
        )));
    }

    Ok(ChunkedPackData {
        dat_size: manifest.dat_size,
        _part_files: part_files,
        parts,
    })
}

/// List all pack hashes in a directory.
pub fn list_packs(packs_dir: &Path) -> Result<Vec<String>> {
    let mut packs = BTreeSet::new();
    let entries = match std::fs::read_dir(packs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(hash) = name
            .strip_prefix("pack-")
            .and_then(|name| name.strip_suffix(".dat"))
        {
            packs.insert(hash.to_owned());
        } else if let Some(hash) = name
            .strip_prefix("pack-")
            .and_then(|name| name.strip_suffix(".dat.parts.json"))
        {
            packs.insert(hash.to_owned());
        }
    }
    Ok(packs.into_iter().collect())
}

pub(crate) fn remove_pack_files(packs_dir: &Path, pack_hash: &str) -> Result<()> {
    remove_file_if_exists(pack_dat_path(packs_dir, pack_hash))?;
    remove_file_if_exists(pack_idx_path(packs_dir, pack_hash))?;
    remove_file_if_exists(pack_parts_manifest_path(packs_dir, pack_hash))?;

    let part_prefix = format!("pack-{}.dat.part-", pack_hash);
    let entries = match std::fs::read_dir(packs_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(&part_prefix) {
            remove_file_if_exists(entry.path())?;
        }
    }

    Ok(())
}

fn remove_file_if_exists(path: PathBuf) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
