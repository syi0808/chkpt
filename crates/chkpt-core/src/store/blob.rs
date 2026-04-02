use crate::error::{ChkpttError, Result};
use memmap2::Mmap;
use std::io::{BufReader, Read};
use std::path::Path;
use std::path::PathBuf;

const HASH_FILE_MMAP_THRESHOLD: u64 = 256 * 1024;

/// Compute BLAKE3 hash of content as raw bytes.
pub fn hash_content_bytes(content: &[u8]) -> [u8; 32] {
    *blake3::hash(content).as_bytes()
}

/// Compute BLAKE3 hash of content, return 64-char hex string.
pub fn hash_content(content: &[u8]) -> String {
    blake3::Hash::from(hash_content_bytes(content))
        .to_hex()
        .to_string()
}

/// Compute BLAKE3 hash of a file without loading the full file into memory.
pub fn hash_file_bytes(path: &Path) -> Result<[u8; 32]> {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() >= HASH_FILE_MMAP_THRESHOLD {
            if let Ok(file) = std::fs::File::open(path) {
                if let Ok(mmap) = unsafe { Mmap::map(&file) } {
                    return Ok(*blake3::hash(&mmap).as_bytes());
                }
            }
        }
    }

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(*hasher.finalize().as_bytes())
}

pub fn hash_file(path: &Path) -> Result<String> {
    Ok(blake3::Hash::from(hash_file_bytes(path)?).to_hex().to_string())
}

fn read_link_bytes(path: &Path) -> Result<Vec<u8>> {
    let target = std::fs::read_link(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        return Ok(target.as_os_str().as_bytes().to_vec());
    }

    #[cfg(not(unix))]
    {
        Ok(target.to_string_lossy().into_owned().into_bytes())
    }
}

pub fn read_path_bytes(path: &Path, is_symlink: bool) -> Result<Vec<u8>> {
    if is_symlink {
        return read_link_bytes(path);
    }

    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let mut buf = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn hash_path_bytes(path: &Path, is_symlink: bool) -> Result<[u8; 32]> {
    if is_symlink {
        return Ok(hash_content_bytes(&read_link_bytes(path)?));
    }

    hash_file_bytes(path)
}

pub fn hash_path(path: &Path, is_symlink: bool) -> Result<String> {
    Ok(blake3::Hash::from(hash_path_bytes(path, is_symlink)?)
        .to_hex()
        .to_string())
}

pub struct BlobStore {
    base_dir: PathBuf,
}

impl BlobStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn object_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base_dir.join(prefix).join(rest)
    }

    /// Check if a blob exists in the store.
    pub fn exists(&self, hash_hex: &str) -> bool {
        self.object_path(hash_hex).exists()
    }

    /// Write content to store. Returns the hash hex string.
    /// Deduplicates: skips write if hash already exists.
    pub fn write(&self, content: &[u8]) -> Result<String> {
        let hash_hex = hash_content(content);
        self.write_if_missing(&hash_hex, content)?;
        Ok(hash_hex)
    }

    /// Write content using a caller-provided hash.
    pub fn write_with_hash(&self, hash_hex: &str, content: &[u8]) -> Result<String> {
        self.write_if_missing(hash_hex, content)?;
        Ok(hash_hex.to_string())
    }

    /// Write already-compressed content if the object is not already present.
    pub fn write_precompressed_if_missing(
        &self,
        hash_hex: &str,
        compressed: &[u8],
    ) -> Result<bool> {
        let path = self.object_path(hash_hex);
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, compressed)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(true)
    }

    /// Write content if the object is not already present.
    pub fn write_if_missing(&self, hash_hex: &str, content: &[u8]) -> Result<bool> {
        let compressed = zstd::encode_all(content, 3)?;
        self.write_precompressed_if_missing(hash_hex, &compressed)
    }

    /// Read and decompress a blob by hash.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash_hex);
        let compressed = match std::fs::read(&path) {
            Ok(compressed) => compressed,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ChkpttError::ObjectNotFound(hash_hex.to_string()));
            }
            Err(error) => return Err(error.into()),
        };
        let decompressed = zstd::decode_all(&compressed[..])?;
        Ok(decompressed)
    }

    /// List all loose object hashes.
    pub fn list_loose(&self) -> Result<Vec<String>> {
        let mut hashes = Vec::new();
        let prefix_entries = match std::fs::read_dir(&self.base_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(hashes),
            Err(error) => return Err(error.into()),
        };
        for prefix_entry in prefix_entries {
            let prefix_entry = prefix_entry?;
            if !prefix_entry.file_type()?.is_dir() {
                continue;
            }
            let prefix = prefix_entry.file_name();
            let prefix = prefix.to_string_lossy();
            for obj_entry in std::fs::read_dir(prefix_entry.path())? {
                let obj_entry = obj_entry?;
                if obj_entry.file_type()?.is_file() {
                    let rest = obj_entry.file_name();
                    let rest = rest.to_string_lossy();
                    if !rest.ends_with(".tmp") {
                        let mut hash = String::with_capacity(prefix.len() + rest.len());
                        hash.push_str(&prefix);
                        hash.push_str(&rest);
                        hashes.push(hash);
                    }
                }
            }
        }
        Ok(hashes)
    }

    /// Remove a loose object by hash.
    pub fn remove(&self, hash_hex: &str) -> Result<()> {
        let path = self.object_path(hash_hex);
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                return Err(error.into());
            }
        }
        Ok(())
    }
}
