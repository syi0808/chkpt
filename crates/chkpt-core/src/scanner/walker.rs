use crate::error::{ChkpttError, Result};
use crate::scanner::matcher::IgnoreMatcher;
use crate::scanner::ScannedFile;
use ignore::{ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Walk a workspace directory, collecting file metadata and respecting ignore rules.
///
/// If `chkptignore_override` is provided, it is used as the .chkptignore path.
/// Otherwise, `root/.chkptignore` is checked automatically.
pub fn walk(
    root: &Path,
    chkptignore_override: Option<&Path>,
    include_deps: bool,
) -> Result<Vec<ScannedFile>> {
    let chkptignore_path = resolve_chkptignore_path(root, chkptignore_override);
    let matcher = Arc::new(IgnoreMatcher::new(
        chkptignore_path.as_deref(),
        include_deps,
    ));
    let mut files = Vec::new();

    for entry in build_walk_builder(root, matcher).build() {
        let entry = entry.map_err(|error| ChkpttError::Other(error.to_string()))?;
        let path = entry.path();
        if path == root {
            continue;
        }

        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }

        let relative = relative_path(root, path);
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| ChkpttError::Other(error.to_string()))?;
        files.push(build_scanned_file(path, &relative, &metadata));
    }

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

pub fn walk_parallel(
    root: &Path,
    chkptignore_override: Option<&Path>,
    include_deps: bool,
) -> Result<Vec<ScannedFile>> {
    let chkptignore_path = resolve_chkptignore_path(root, chkptignore_override);
    let root = Arc::new(root.to_path_buf());
    let matcher = Arc::new(IgnoreMatcher::new(
        chkptignore_path.as_deref(),
        include_deps,
    ));
    let files = Arc::new(Mutex::new(Vec::new()));
    let error = Arc::new(Mutex::new(None));

    let mut builder = build_walk_builder(root.as_path(), matcher);
    builder.standard_filters(false).follow_links(false).threads(
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0),
    );

    let mut visitor_builder = CollectBuilder {
        root: Arc::clone(&root),
        files: Arc::clone(&files),
        error: Arc::clone(&error),
    };
    builder.build_parallel().visit(&mut visitor_builder);

    if let Some(error) = error.lock().unwrap().take() {
        return Err(error);
    }

    let mut files = files.lock().unwrap().clone();
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(files)
}

fn resolve_chkptignore_path(root: &Path, chkptignore_override: Option<&Path>) -> Option<PathBuf> {
    match chkptignore_override {
        Some(path) => Some(path.to_path_buf()),
        None => {
            let default_path = root.join(".chkptignore");
            if default_path.exists() {
                Some(default_path)
            } else {
                None
            }
        }
    }
}

fn build_walk_builder(root: &Path, matcher: Arc<IgnoreMatcher>) -> WalkBuilder {
    let root_path = root.to_path_buf();
    let filter_root = root_path.clone();
    let filter_matcher = Arc::clone(&matcher);

    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == filter_root.as_path() {
                return true;
            }

            let Some(file_type) = entry.file_type() else {
                return true;
            };
            let relative = relative_path(filter_root.as_path(), path);
            if file_type.is_dir() {
                !filter_matcher.is_ignored(&relative, true)
            } else if file_type.is_symlink() {
                !filter_matcher.is_ignored(&relative, false)
                    && !filter_matcher.is_ignored(&relative, true)
            } else if file_type.is_file() {
                !filter_matcher.is_ignored(&relative, false)
            } else {
                false
            }
        });
    builder
}

fn relative_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if cfg!(windows) {
        relative.to_string_lossy().replace('\\', "/")
    } else {
        relative.to_string_lossy().into_owned()
    }
}

struct CollectBuilder {
    root: Arc<PathBuf>,
    files: Arc<Mutex<Vec<ScannedFile>>>,
    error: Arc<Mutex<Option<ChkpttError>>>,
}

impl<'s> ParallelVisitorBuilder<'s> for CollectBuilder {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        Box::new(CollectVisitor {
            root: Arc::clone(&self.root),
            files: Arc::clone(&self.files),
            error: Arc::clone(&self.error),
            local_files: Vec::new(),
        })
    }
}

struct CollectVisitor {
    root: Arc<PathBuf>,
    files: Arc<Mutex<Vec<ScannedFile>>>,
    error: Arc<Mutex<Option<ChkpttError>>>,
    local_files: Vec<ScannedFile>,
}

impl CollectVisitor {
    fn store_error(&self, error: ChkpttError) {
        let mut slot = self.error.lock().unwrap();
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

impl Drop for CollectVisitor {
    fn drop(&mut self) {
        if self.local_files.is_empty() {
            return;
        }
        let mut files = self.files.lock().unwrap();
        files.append(&mut self.local_files);
    }
}

impl ParallelVisitor for CollectVisitor {
    fn visit(&mut self, entry: std::result::Result<ignore::DirEntry, ignore::Error>) -> WalkState {
        if self.error.lock().unwrap().is_some() {
            return WalkState::Quit;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                self.store_error(ChkpttError::Other(error.to_string()));
                return WalkState::Quit;
            }
        };

        let path = entry.path();
        if path == self.root.as_path() {
            return WalkState::Continue;
        }

        let Some(file_type) = entry.file_type() else {
            return WalkState::Continue;
        };

        if !file_type.is_file() && !file_type.is_symlink() {
            return WalkState::Continue;
        }

        let relative = relative_path(self.root.as_path(), path);
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.store_error(ChkpttError::Other(error.to_string()));
                return WalkState::Quit;
            }
        };

        self.local_files
            .push(build_scanned_file(path, &relative, &metadata));
        WalkState::Continue
    }
}

#[cfg(unix)]
fn build_scanned_file(
    path: &Path,
    relative_path: &str,
    metadata: &std::fs::Metadata,
) -> ScannedFile {
    use std::os::unix::fs::MetadataExt;

    ScannedFile {
        relative_path: relative_path.to_string(),
        absolute_path: path.to_path_buf(),
        size: metadata.len(),
        mtime_secs: metadata.mtime(),
        mtime_nanos: metadata.mtime_nsec(),
        device: Some(metadata.dev()),
        inode: Some(metadata.ino()),
        mode: metadata.mode(),
        is_symlink: metadata.file_type().is_symlink(),
    }
}

#[cfg(not(unix))]
fn build_scanned_file(
    path: &Path,
    relative_path: &str,
    metadata: &std::fs::Metadata,
) -> ScannedFile {
    use std::time::UNIX_EPOCH;

    let (mtime_secs, mtime_nanos) = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| (d.as_secs() as i64, d.subsec_nanos() as i64))
        .unwrap_or((0, 0));

    ScannedFile {
        relative_path: relative_path.to_string(),
        absolute_path: path.to_path_buf(),
        size: metadata.len(),
        mtime_secs,
        mtime_nanos,
        device: None,
        inode: None,
        mode: 0o644,
        is_symlink: metadata.file_type().is_symlink(),
    }
}
