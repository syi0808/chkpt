use crate::error::Result;
use memmap2::Mmap;
use std::io::{BufReader, Read};
use std::path::Path;

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
