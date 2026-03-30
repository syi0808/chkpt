use crate::error::to_napi_error;
use crate::store::{bytes32_to_hex, hex_to_bytes32};
use chkpt_core::index::FileEntry;
use chkpt_core::index::FileIndex;
use serde::Deserialize;

/// Output struct for returning file entries to JavaScript.
#[napi(object)]
pub struct JsFileEntry {
    pub path: String,
    pub blob_hash: String,
    pub size: i64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub inode: Option<i64>,
    pub mode: u32,
}

/// Input struct for receiving file entries from JavaScript via serde.
/// Using serde instead of #[napi(object)] to properly handle null values
/// for Option fields (napi-rs has issues converting JS null to Option<i64>).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerdeFileEntry {
    path: String,
    blob_hash: String,
    size: i64,
    mtime_secs: i64,
    mtime_nanos: i64,
    inode: Option<i64>,
    mode: u32,
}

fn core_entry_to_js(entry: &FileEntry) -> JsFileEntry {
    JsFileEntry {
        path: entry.path.clone(),
        blob_hash: bytes32_to_hex(&entry.blob_hash),
        size: entry.size as i64,
        mtime_secs: entry.mtime_secs,
        mtime_nanos: entry.mtime_nanos,
        inode: entry.inode.map(|i| i as i64),
        mode: entry.mode,
    }
}

fn serde_entry_to_core(js: &SerdeFileEntry) -> napi::Result<FileEntry> {
    let blob_hash = hex_to_bytes32(&js.blob_hash)?;
    Ok(FileEntry {
        path: js.path.clone(),
        blob_hash,
        size: js.size as u64,
        mtime_secs: js.mtime_secs,
        mtime_nanos: js.mtime_nanos,
        inode: js.inode.map(|i| i as u64),
        mode: js.mode,
    })
}

#[napi]
pub async fn index_open(db_path: String) -> napi::Result<()> {
    FileIndex::open(&db_path).map_err(to_napi_error)?;
    Ok(())
}

#[napi]
pub async fn index_lookup(db_path: String, path: String) -> napi::Result<Option<JsFileEntry>> {
    let idx = FileIndex::open(&db_path).map_err(to_napi_error)?;
    let entry = idx.get(&path).map_err(to_napi_error)?;
    Ok(entry.as_ref().map(core_entry_to_js))
}

#[napi(
    ts_args_type = "dbPath: string, entries: Array<{ path: string, blobHash: string, size: number, mtimeSecs: number, mtimeNanos: number, inode: number | null, mode: number }>"
)]
pub async fn index_upsert(db_path: String, entries: Vec<serde_json::Value>) -> napi::Result<()> {
    let idx = FileIndex::open(&db_path).map_err(to_napi_error)?;
    let core_entries: Vec<FileEntry> = entries
        .iter()
        .map(|v| {
            let serde_entry: SerdeFileEntry = serde_json::from_value(v.clone()).map_err(|e| {
                napi::Error::new(napi::Status::InvalidArg, format!("invalid entry: {}", e))
            })?;
            serde_entry_to_core(&serde_entry)
        })
        .collect::<napi::Result<Vec<_>>>()?;
    idx.bulk_upsert(&core_entries).map_err(to_napi_error)?;
    Ok(())
}

#[napi]
pub async fn index_all_entries(db_path: String) -> napi::Result<Vec<JsFileEntry>> {
    let idx = FileIndex::open(&db_path).map_err(to_napi_error)?;
    let entries = idx.all_entries().map_err(to_napi_error)?;
    Ok(entries.iter().map(core_entry_to_js).collect())
}

#[napi]
pub async fn index_clear(db_path: String) -> napi::Result<()> {
    let idx = FileIndex::open(&db_path).map_err(to_napi_error)?;
    idx.clear().map_err(to_napi_error)?;
    Ok(())
}
