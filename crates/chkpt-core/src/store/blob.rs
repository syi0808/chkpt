use crate::error::{ChkpttError, Result};
use std::io::{BufReader, Read};
use std::path::Path;
use std::path::PathBuf;

/// Compute BLAKE3 hash of content, return 64-char hex string.
pub fn hash_content(content: &[u8]) -> String {
    blake3::hash(content).to_hex().to_string()
}

/// Compute BLAKE3 hash of a file without loading the full file into memory.
pub fn hash_file(path: &Path) -> Result<String> {
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

    Ok(hasher.finalize().to_hex().to_string())
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
        if !path.exists() {
            return Err(ChkpttError::ObjectNotFound(hash_hex.to_string()));
        }
        let compressed = std::fs::read(&path)?;
        let decompressed = zstd::decode_all(&compressed[..])?;
        Ok(decompressed)
    }

    /// List all loose object hashes.
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
            for obj_entry in std::fs::read_dir(prefix_entry.path())? {
                let obj_entry = obj_entry?;
                if obj_entry.file_type()?.is_file() {
                    let rest = obj_entry.file_name().to_string_lossy().to_string();
                    if !rest.ends_with(".tmp") {
                        hashes.push(format!("{}{}", prefix, rest));
                    }
                }
            }
        }
        Ok(hashes)
    }

    /// Remove a loose object by hash.
    pub fn remove(&self, hash_hex: &str) -> Result<()> {
        let path = self.object_path(hash_hex);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
}
