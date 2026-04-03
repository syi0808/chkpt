use crate::error::to_napi_error;
use chkpt_core::store::blob::hash_content;
use chkpt_core::store::tree::{EntryType, TreeEntry, TreeStore};
use napi::bindgen_prelude::*;
use std::path::PathBuf;

// ── helpers ──────────────────────────────────────────────────────────

/// Convert a 32-char hex string to a [u8; 16] array.
pub(crate) fn hex_to_bytes32(hex: &str) -> napi::Result<[u8; 16]> {
    if hex.len() != 32 || !hex.is_ascii() {
        return Err(napi::Error::new(
            napi::Status::InvalidArg,
            format!("expected 32-char hex string, got {} chars", hex.len()),
        ));
    }
    let mut bytes = [0u8; 16];
    for i in 0..16 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| {
            napi::Error::new(
                napi::Status::InvalidArg,
                format!("invalid hex at position {}", i * 2),
            )
        })?;
    }
    Ok(bytes)
}

/// Convert a [u8; 16] array to a 32-char hex string.
pub(crate) fn bytes32_to_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ── blob bindings ────────────────────────────────────────────────────

#[napi]
pub fn blob_hash(content: Buffer) -> String {
    hash_content(content.as_ref())
}

// ── tree bindings ────────────────────────────────────────────────────

#[napi(object)]
pub struct JsTreeEntry {
    pub name: String,
    pub entry_type: String,
    pub hash: String,
    pub size: i64,
    pub mode: i64,
}

#[napi(object)]
pub struct JsTreeBuildResult {
    pub hash: String,
}

fn js_entry_to_tree_entry(js: &JsTreeEntry) -> napi::Result<TreeEntry> {
    let entry_type = match js.entry_type.as_str() {
        "file" => EntryType::File,
        "directory" => EntryType::Dir,
        "symlink" => EntryType::Symlink,
        other => {
            return Err(napi::Error::new(
                napi::Status::InvalidArg,
                format!("unknown entry type: {}", other),
            ))
        }
    };
    let hash = hex_to_bytes32(&js.hash)?;
    Ok(TreeEntry {
        name: js.name.clone(),
        entry_type,
        hash,
        size: js.size as u64,
        mode: js.mode as u32,
    })
}

fn tree_entry_to_js(entry: &TreeEntry) -> JsTreeEntry {
    let entry_type = match entry.entry_type {
        EntryType::File => "file",
        EntryType::Dir => "directory",
        EntryType::Symlink => "symlink",
    };
    JsTreeEntry {
        name: entry.name.clone(),
        entry_type: entry_type.to_string(),
        hash: bytes32_to_hex(&entry.hash),
        size: entry.size as i64,
        mode: entry.mode as i64,
    }
}

#[napi]
pub async fn tree_build(
    trees_dir: String,
    entries: Vec<JsTreeEntry>,
) -> napi::Result<JsTreeBuildResult> {
    let tree_entries: Vec<TreeEntry> = entries
        .iter()
        .map(js_entry_to_tree_entry)
        .collect::<napi::Result<Vec<_>>>()?;
    let store = TreeStore::new(PathBuf::from(trees_dir));
    let hash = store.write(&tree_entries).map_err(to_napi_error)?;
    Ok(JsTreeBuildResult { hash })
}

#[napi]
pub async fn tree_load(trees_dir: String, hash: String) -> napi::Result<Vec<JsTreeEntry>> {
    let store = TreeStore::new(PathBuf::from(trees_dir));
    let entries = store.read(&hash).map_err(to_napi_error)?;
    Ok(entries.iter().map(tree_entry_to_js).collect())
}
