use crate::error::to_napi_error;
use chkpt_core::scanner;
use std::path::Path;

#[napi(object)]
pub struct JsScannedFile {
    pub relative_path: String,
    pub absolute_path: String,
    pub size: i64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub inode: Option<i64>,
    pub mode: u32,
}

#[napi]
pub async fn scan_workspace(
    workspace_path: String,
    include_deps: Option<bool>,
) -> napi::Result<Vec<JsScannedFile>> {
    let path = Path::new(&workspace_path);
    let files = scanner::scan_workspace_with_options(path, None, include_deps.unwrap_or(false))
        .map_err(to_napi_error)?;
    Ok(files
        .iter()
        .map(|f| JsScannedFile {
            relative_path: f.relative_path.clone(),
            absolute_path: f.absolute_path.to_string_lossy().to_string(),
            size: f.size as i64,
            mtime_secs: f.mtime_secs,
            mtime_nanos: f.mtime_nanos,
            inode: f.inode.map(|i| i as i64),
            mode: f.mode,
        })
        .collect())
}
