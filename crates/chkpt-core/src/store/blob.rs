use crate::error::Result;
use memmap2::Mmap;
use std::io::{BufReader, Read};
use std::path::Path;

const HASH_FILE_MMAP_THRESHOLD: u64 = 256 * 1024;

/// Zero-copy file content: small files are heap-allocated, large files are memory-mapped.
pub enum FileContent {
    Vec(Vec<u8>),
    Mmap(Mmap),
}

impl AsRef<[u8]> for FileContent {
    fn as_ref(&self) -> &[u8] {
        match self {
            FileContent::Vec(v) => v.as_slice(),
            FileContent::Mmap(m) => m.as_ref(),
        }
    }
}

/// Read a file into a `FileContent`. Files >= 256 KB are memory-mapped; smaller files
/// are read into a heap-allocated `Vec<u8>`.
pub fn read_or_mmap(path: &Path) -> Result<FileContent> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() >= HASH_FILE_MMAP_THRESHOLD {
        // SAFETY: the file is opened read-only and we do not mutate it through the mapping.
        let mmap = unsafe { Mmap::map(&file) }?;
        #[cfg(unix)]
        {
            let _ = mmap.advise(memmap2::Advice::Sequential);
        }
        return Ok(FileContent::Mmap(mmap));
    }
    let mut buf = Vec::with_capacity(metadata.len() as usize);
    let mut file = file;
    file.read_to_end(&mut buf)?;
    Ok(FileContent::Vec(buf))
}

/// Compute XXH3-128 hash of content as raw bytes.
pub fn hash_content_bytes(content: &[u8]) -> [u8; 16] {
    xxhash_rust::xxh3::xxh3_128(content).to_le_bytes()
}

/// Compute XXH3-128 hash of content, return 32-char hex string.
pub fn hash_content(content: &[u8]) -> String {
    bytes_to_hex(&hash_content_bytes(content))
}

/// Compute XXH3-128 hash of a file without loading the full file into memory.
pub fn hash_file_bytes(path: &Path) -> Result<[u8; 16]> {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() >= HASH_FILE_MMAP_THRESHOLD {
            if let Ok(file) = std::fs::File::open(path) {
                // SAFETY: file is opened read-only and kept alive alongside the mmap.
                if let Ok(mmap) = unsafe { Mmap::map(&file) } {
                    return Ok(xxhash_rust::xxh3::xxh3_128(&mmap).to_le_bytes());
                }
            }
        }
    }

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.digest128().to_le_bytes())
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

pub fn hash_path_bytes(path: &Path, is_symlink: bool) -> Result<[u8; 16]> {
    if is_symlink {
        return Ok(hash_content_bytes(&read_link_bytes(path)?));
    }

    hash_file_bytes(path)
}

/// Convert a 32-char hex string to [u8; 16].
pub fn hex_to_bytes(hex: &str) -> Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    if hex.len() != 32 {
        return Err(crate::error::ChkpttError::Other(format!(
            "Invalid hash length: {}",
            hex.len()
        )));
    }
    for i in 0..16 {
        let slice = hex
            .get(i * 2..i * 2 + 2)
            .ok_or_else(|| crate::error::ChkpttError::Other("Invalid hex: byte index out of bounds".into()))?;
        bytes[i] = u8::from_str_radix(slice, 16)
            .map_err(|_| crate::error::ChkpttError::Other("Invalid hex".into()))?;
    }
    Ok(bytes)
}

/// Convert [u8; 16] to a 32-char hex string.
pub fn bytes_to_hex(bytes: &[u8; 16]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(32);
    for byte in bytes {
        write!(hex, "{byte:02x}").unwrap();
    }
    hex
}
