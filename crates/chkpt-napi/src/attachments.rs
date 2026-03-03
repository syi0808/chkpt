use crate::error::to_napi_error;
use chkpt_core::attachments::{deps, git};
use std::path::Path;

#[napi]
pub fn compute_deps_key(lockfile_path: String) -> napi::Result<String> {
    deps::compute_deps_key(Path::new(&lockfile_path)).map_err(to_napi_error)
}

#[napi]
pub async fn deps_archive(
    deps_dir: String,
    archive_dir: String,
    deps_key: String,
) -> napi::Result<String> {
    deps::archive_deps(Path::new(&deps_dir), Path::new(&archive_dir), &deps_key)
        .map_err(to_napi_error)
}

#[napi]
pub async fn deps_restore(
    deps_dir: String,
    archive_dir: String,
    deps_key: String,
) -> napi::Result<()> {
    deps::restore_deps(Path::new(&deps_dir), Path::new(&archive_dir), &deps_key)
        .map_err(to_napi_error)
}

#[napi]
pub async fn git_bundle_create(
    repo_path: String,
    archive_dir: String,
) -> napi::Result<String> {
    git::create_git_bundle(Path::new(&repo_path), Path::new(&archive_dir)).map_err(to_napi_error)
}

#[napi]
pub async fn git_bundle_restore(
    repo_path: String,
    archive_dir: String,
    git_key: String,
) -> napi::Result<()> {
    git::restore_git_bundle(Path::new(&repo_path), Path::new(&archive_dir), &git_key)
        .map_err(to_napi_error)
}
