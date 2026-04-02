use std::path::{Path, PathBuf};

/// Generate a 16-hex-char project ID from workspace path.
pub fn project_id_from_path(path: &Path) -> String {
    let canonical = path.to_string_lossy();
    let hash = blake3::hash(canonical.as_bytes());
    hash.to_hex()[..16].to_string()
}

/// Store directory layout for a project.
pub struct StoreLayout {
    base: PathBuf,
}

impl StoreLayout {
    pub fn new(project_id: &str) -> Self {
        let home_dir = std::env::var_os("CHKPT_HOME")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        Self::from_home_dir(home_dir, project_id)
    }

    pub fn from_home_dir<P: AsRef<Path>>(home_dir: P, project_id: &str) -> Self {
        let base = home_dir
            .as_ref()
            .join(".chkpt")
            .join("stores")
            .join(project_id);
        Self { base }
    }

    pub fn base_dir(&self) -> &Path {
        &self.base
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.base.join("snapshots")
    }

    pub fn catalog_path(&self) -> PathBuf {
        self.base.join("catalog.sqlite")
    }

    pub fn trees_dir(&self) -> PathBuf {
        self.base.join("trees")
    }

    pub fn packs_dir(&self) -> PathBuf {
        self.base.join("packs")
    }

    pub fn index_path(&self) -> PathBuf {
        self.base.join("index.bin")
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.base.join("locks")
    }

    pub fn attachments_deps_dir(&self) -> PathBuf {
        self.base.join("attachments").join("deps")
    }

    pub fn attachments_git_dir(&self) -> PathBuf {
        self.base.join("attachments").join("git")
    }

    /// Tree path with 2-char prefix: trees/a3/rest_of_hash
    pub fn tree_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base.join("trees").join(prefix).join(rest)
    }

    /// Create all required directories.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [
            self.base.clone(),
            self.snapshots_dir(),
            self.trees_dir(),
            self.packs_dir(),
            self.locks_dir(),
            self.attachments_deps_dir(),
            self.attachments_git_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }

        // Prevent macOS Spotlight from indexing the store directory.
        // mdworker_shared processes spike CPU after bulk writes without this.
        #[cfg(target_os = "macos")]
        {
            let marker = self.base.join(".metadata_never_index");
            if !marker.exists() {
                std::fs::File::create(marker)?;
            }
        }

        Ok(())
    }
}
