use chkpt_core::config::{project_id_from_path, StoreLayout};
use std::path::Path;

#[napi(object)]
pub struct JsStoreLayout {
    pub root: String,
    pub objects_dir: String,
    pub trees_dir: String,
    pub snapshots_dir: String,
    pub packs_dir: String,
    pub index_path: String,
    pub locks_dir: String,
    pub attachments_deps_dir: String,
    pub attachments_git_dir: String,
}

#[napi]
pub fn get_project_id(workspace_path: String) -> String {
    let path = Path::new(&workspace_path);
    project_id_from_path(path)
}

#[napi]
pub fn get_store_layout(workspace_path: String) -> JsStoreLayout {
    let path = Path::new(&workspace_path);
    let project_id = project_id_from_path(path);
    let layout = StoreLayout::new(&project_id);
    JsStoreLayout {
        root: layout.base_dir().to_string_lossy().to_string(),
        objects_dir: layout.objects_dir().to_string_lossy().to_string(),
        trees_dir: layout.trees_dir().to_string_lossy().to_string(),
        snapshots_dir: layout.snapshots_dir().to_string_lossy().to_string(),
        packs_dir: layout.packs_dir().to_string_lossy().to_string(),
        index_path: layout.index_path().to_string_lossy().to_string(),
        locks_dir: layout.locks_dir().to_string_lossy().to_string(),
        attachments_deps_dir: layout.attachments_deps_dir().to_string_lossy().to_string(),
        attachments_git_dir: layout.attachments_git_dir().to_string_lossy().to_string(),
    }
}
