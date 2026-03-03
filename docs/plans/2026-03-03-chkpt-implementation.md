# chkpt v1 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a filesystem checkpoint engine (save/restore/delete/list) with CLI, MCP server, content-addressed storage, packfiles, and attachments.

**Architecture:** Monolithic core library (`chkpt-core`) with all domain logic in modules. Thin CLI (`chkpt-cli`) and MCP server (`chkpt-mcp`) wrappers. Content-addressed blob/tree storage with BLAKE3 hashing, zstd compression, SQLite index cache, and packfile support.

**Tech Stack:** Rust, tokio, blake3, zstd, rusqlite (bundled), bincode, clap, rmcp, serde, fs4, tempfile, ignore crate, tar, uuid, chrono.

**Reference:** `docs/plans/2026-03-03-chkpt-design.md`, `PRD.md`

---

## Task 1: Workspace & Project Scaffolding

**Files:**

- Create: `Cargo.toml` (workspace root)
- Create: `crates/chkpt-core/Cargo.toml`
- Create: `crates/chkpt-cli/Cargo.toml`
- Create: `crates/chkpt-mcp/Cargo.toml`
- Create: `crates/chkpt-core/src/lib.rs`
- Create: `crates/chkpt-cli/src/main.rs`
- Create: `crates/chkpt-mcp/src/main.rs`

**Step 1: Create workspace Cargo.toml**

```toml
[workspace]
resolver = "2"
members = ["crates/chkpt-core", "crates/chkpt-cli", "crates/chkpt-mcp"]

[workspace.dependencies]
blake3 = "1"
zstd = "0.13"
rusqlite = { version = "0.31", features = ["bundled"] }
bincode = "1"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v7"] }
tar = "0.4"
fs4 = { version = "0.10", features = ["tokio"] }
tempfile = "3"
ignore = "0.4"
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
rmcp = { version = "0.1", features = ["server", "macros", "transport-io"] }
```

**Step 2: Create chkpt-core/Cargo.toml**

```toml
[package]
name = "chkpt-core"
version = "0.1.0"
edition = "2021"

[dependencies]
blake3 = { workspace = true }
zstd = { workspace = true }
rusqlite = { workspace = true }
bincode = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
tar = { workspace = true }
fs4 = { workspace = true }
tempfile = { workspace = true }
ignore = { workspace = true }
thiserror = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
tempfile = { workspace = true }
```

**Step 3: Create chkpt-cli/Cargo.toml**

```toml
[package]
name = "chkpt-cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "chkpt"
path = "src/main.rs"

[dependencies]
chkpt-core = { path = "../chkpt-core" }
clap = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

**Step 4: Create chkpt-mcp/Cargo.toml**

```toml
[package]
name = "chkpt-mcp"
version = "0.1.0"
edition = "2021"

[dependencies]
chkpt-core = { path = "../chkpt-core" }
rmcp = { workspace = true }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
anyhow = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

**Step 5: Create minimal lib.rs and main.rs stubs**

`crates/chkpt-core/src/lib.rs`:

```rust
pub mod config;
pub mod scanner;
pub mod store;
pub mod index;
pub mod ops;
pub mod attachments;
```

`crates/chkpt-cli/src/main.rs`:

```rust
fn main() {
    println!("chkpt cli - not yet implemented");
}
```

`crates/chkpt-mcp/src/main.rs`:

```rust
fn main() {
    println!("chkpt mcp - not yet implemented");
}
```

**Step 6: Verify workspace builds**

Run: `cargo check --workspace`
Expected: SUCCESS (with warnings about unused modules)

**Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/
git commit -m "feat: scaffold workspace with chkpt-core, chkpt-cli, chkpt-mcp crates"
```

---

## Task 2: Error Types & Config Module

**Files:**

- Create: `crates/chkpt-core/src/error.rs`
- Create: `crates/chkpt-core/src/config.rs`
- Create: `crates/chkpt-core/tests/config_test.rs`
- Modify: `crates/chkpt-core/src/lib.rs`

**Step 1: Write failing tests for config**

`crates/chkpt-core/tests/config_test.rs`:

```rust
use chkpt_core::config::{project_id_from_path, ProjectConfig, Guardrails, StoreLayout};
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn test_project_id_deterministic() {
    let path = PathBuf::from("/tmp/test-workspace");
    let id1 = project_id_from_path(&path);
    let id2 = project_id_from_path(&path);
    assert_eq!(id1, id2);
    assert_eq!(id1.len(), 16); // 16 hex chars
}

#[test]
fn test_project_id_different_paths() {
    let id1 = project_id_from_path(&PathBuf::from("/tmp/a"));
    let id2 = project_id_from_path(&PathBuf::from("/tmp/b"));
    assert_ne!(id1, id2);
}

#[test]
fn test_store_layout_paths() {
    let layout = StoreLayout::new("abcdef1234567890");
    let base = layout.base_dir();
    assert!(base.ends_with("abcdef1234567890"));
    assert!(layout.snapshots_dir().ends_with("snapshots"));
    assert!(layout.objects_dir().ends_with("objects"));
    assert!(layout.trees_dir().ends_with("trees"));
    assert!(layout.packs_dir().ends_with("packs"));
    assert!(layout.locks_dir().ends_with("locks"));
}

#[test]
fn test_store_layout_object_path_has_prefix_dir() {
    let layout = StoreLayout::new("abcdef1234567890");
    let hash_hex = "a3b4c5d6e7f8901234567890abcdef1234567890abcdef1234567890abcdef12";
    let path = layout.object_path(hash_hex);
    // Should be objects/a3/b4c5d6...
    let parent = path.parent().unwrap();
    assert!(parent.ends_with("a3"));
}

#[test]
fn test_guardrails_default() {
    let g = Guardrails::default();
    assert!(g.max_total_bytes > 0);
    assert!(g.max_files > 0);
    assert!(g.max_file_size > 0);
}

#[test]
fn test_project_config_roundtrip() {
    let dir = TempDir::new().unwrap();
    let config = ProjectConfig {
        project_root: PathBuf::from("/tmp/test"),
        created_at: chrono::Utc::now(),
        guardrails: Guardrails::default(),
    };
    let path = dir.path().join("config.json");
    config.save(&path).unwrap();
    let loaded = ProjectConfig::load(&path).unwrap();
    assert_eq!(loaded.project_root, config.project_root);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test config_test`
Expected: FAIL — modules don't exist yet

**Step 3: Create error.rs**

`crates/chkpt-core/src/error.rs`:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChkptError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Bincode error: {0}")]
    Bincode(#[from] bincode::Error),

    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),

    #[error("Lock held by another process")]
    LockHeld,

    #[error("Guardrail exceeded: {0}")]
    GuardrailExceeded(String),

    #[error("Store corrupted: {0}")]
    StoreCorrupted(String),

    #[error("Object not found: {0}")]
    ObjectNotFound(String),

    #[error("Restore failed: {0}")]
    RestoreFailed(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ChkptError>;
```

**Step 4: Create config.rs**

`crates/chkpt-core/src/config.rs`:

```rust
use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".chkpt")
            .join("stores")
            .join(project_id);
        Self { base }
    }

    pub fn base_dir(&self) -> &Path { &self.base }
    pub fn config_path(&self) -> PathBuf { self.base.join("config.json") }
    pub fn snapshots_dir(&self) -> PathBuf { self.base.join("snapshots") }
    pub fn objects_dir(&self) -> PathBuf { self.base.join("objects") }
    pub fn trees_dir(&self) -> PathBuf { self.base.join("trees") }
    pub fn packs_dir(&self) -> PathBuf { self.base.join("packs") }
    pub fn index_path(&self) -> PathBuf { self.base.join("index.sqlite") }
    pub fn locks_dir(&self) -> PathBuf { self.base.join("locks") }
    pub fn attachments_deps_dir(&self) -> PathBuf { self.base.join("attachments").join("deps") }
    pub fn attachments_git_dir(&self) -> PathBuf { self.base.join("attachments").join("git") }

    /// Object path with 2-char prefix: objects/a3/rest_of_hash
    pub fn object_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base.join("objects").join(prefix).join(rest)
    }

    /// Tree path with 2-char prefix: trees/a3/rest_of_hash
    pub fn tree_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base.join("trees").join(prefix).join(rest)
    }

    /// Create all required directories.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [
            &self.base,
            &self.snapshots_dir(),
            &self.objects_dir(),
            &self.trees_dir(),
            &self.packs_dir(),
            &self.locks_dir(),
            &self.attachments_deps_dir(),
            &self.attachments_git_dir(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guardrails {
    pub max_total_bytes: u64,
    pub max_files: u64,
    pub max_file_size: u64,
}

impl Default for Guardrails {
    fn default() -> Self {
        Self {
            max_total_bytes: 2 * 1024 * 1024 * 1024, // 2 GB
            max_files: 100_000,
            max_file_size: 100 * 1024 * 1024, // 100 MB
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub project_root: PathBuf,
    pub created_at: DateTime<Utc>,
    pub guardrails: Guardrails,
}

impl ProjectConfig {
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }
}
```

Note: add `dirs = "5"` to chkpt-core Cargo.toml dependencies.

**Step 5: Update lib.rs**

```rust
pub mod error;
pub mod config;
```

(Other modules will be added as stubs or as we implement them.)

**Step 6: Run tests**

Run: `cargo test -p chkpt-core --test config_test`
Expected: PASS

**Step 7: Commit**

```bash
git add -A
git commit -m "feat: add error types and config module with project ID, store layout, guardrails"
```

---

## Task 3: Blob Store (BLAKE3 + zstd)

**Files:**

- Create: `crates/chkpt-core/src/store/mod.rs`
- Create: `crates/chkpt-core/src/store/blob.rs`
- Create: `crates/chkpt-core/tests/blob_test.rs`
- Modify: `crates/chkpt-core/src/lib.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/blob_test.rs`:

```rust
use chkpt_core::store::blob::BlobStore;
use tempfile::TempDir;

#[test]
fn test_store_and_read_blob() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let content = b"hello world";
    let hash = store.write(content).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back, content);
}

#[test]
fn test_blob_hash_deterministic() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let h1 = store.write(b"same content").unwrap();
    let h2 = store.write(b"same content").unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn test_blob_dedup() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    store.write(b"dedup me").unwrap();
    store.write(b"dedup me").unwrap();
    // Only one file should exist (dedup)
    let count: usize = walkdir(dir.path());
    assert_eq!(count, 1);
}

fn walkdir(path: &std::path::Path) -> usize {
    let mut count = 0;
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            count += walkdir(&entry.path());
        } else {
            count += 1;
        }
    }
    count
}

#[test]
fn test_blob_exists() {
    let dir = TempDir::new().unwrap();
    let store = BlobStore::new(dir.path().to_path_buf());
    let hash = store.write(b"exists").unwrap();
    assert!(store.exists(&hash));
    assert!(!store.exists(&"0".repeat(64)));
}

#[test]
fn test_hash_content_without_storing() {
    let hash = chkpt_core::store::blob::hash_content(b"test");
    assert_eq!(hash.len(), 64); // 64 hex chars
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test blob_test`
Expected: FAIL

**Step 3: Implement blob.rs**

`crates/chkpt-core/src/store/blob.rs`:

```rust
use crate::error::{ChkptError, Result};
use std::io::{Read, Write};
use std::path::PathBuf;

/// Compute BLAKE3 hash of content, return 64-char hex string.
pub fn hash_content(content: &[u8]) -> String {
    blake3::hash(content).to_hex().to_string()
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
        let path = self.object_path(&hash_hex);
        if path.exists() {
            return Ok(hash_hex);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let compressed = zstd::encode_all(content, 3)?;
        // Write atomically via temp file + rename
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &compressed)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(hash_hex)
    }

    /// Read and decompress a blob by hash.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<u8>> {
        let path = self.object_path(hash_hex);
        if !path.exists() {
            return Err(ChkptError::ObjectNotFound(hash_hex.to_string()));
        }
        let compressed = std::fs::read(&path)?;
        let decompressed = zstd::decode_all(compressed.as_slice())?;
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
            if !prefix_entry.file_type()?.is_dir() { continue; }
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
```

`crates/chkpt-core/src/store/mod.rs`:

```rust
pub mod blob;
```

**Step 4: Run tests**

Run: `cargo test -p chkpt-core --test blob_test`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add BlobStore with BLAKE3 hashing and zstd compression"
```

---

## Task 4: Tree Store (bincode serialization)

**Files:**

- Create: `crates/chkpt-core/src/store/tree.rs`
- Create: `crates/chkpt-core/tests/tree_test.rs`
- Modify: `crates/chkpt-core/src/store/mod.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/tree_test.rs`:

```rust
use chkpt_core::store::tree::{TreeEntry, EntryType, TreeStore};
use tempfile::TempDir;

#[test]
fn test_tree_roundtrip() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![
        TreeEntry {
            name: "bar.txt".into(),
            entry_type: EntryType::File,
            hash: [1u8; 32],
            size: 100,
            mode: 0o644,
        },
        TreeEntry {
            name: "foo.txt".into(),
            entry_type: EntryType::File,
            hash: [2u8; 32],
            size: 200,
            mode: 0o644,
        },
    ];
    let hash = store.write(&entries).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back.len(), 2);
    assert_eq!(read_back[0].name, "bar.txt"); // sorted
}

#[test]
fn test_tree_hash_deterministic() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![TreeEntry {
        name: "a.txt".into(),
        entry_type: EntryType::File,
        hash: [0u8; 32],
        size: 10,
        mode: 0o644,
    }];
    let h1 = store.write(&entries).unwrap();
    let h2 = store.write(&entries).unwrap();
    assert_eq!(h1, h2);
}

#[test]
fn test_tree_sorts_entries() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![
        TreeEntry { name: "z".into(), entry_type: EntryType::File, hash: [0u8; 32], size: 0, mode: 0o644 },
        TreeEntry { name: "a".into(), entry_type: EntryType::File, hash: [1u8; 32], size: 0, mode: 0o644 },
    ];
    let hash = store.write(&entries).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back[0].name, "a");
    assert_eq!(read_back[1].name, "z");
}

#[test]
fn test_tree_with_dir_entry() {
    let dir = TempDir::new().unwrap();
    let store = TreeStore::new(dir.path().to_path_buf());
    let entries = vec![
        TreeEntry { name: "src".into(), entry_type: EntryType::Dir, hash: [5u8; 32], size: 0, mode: 0o755 },
        TreeEntry { name: "README.md".into(), entry_type: EntryType::File, hash: [6u8; 32], size: 50, mode: 0o644 },
    ];
    let hash = store.write(&entries).unwrap();
    let read_back = store.read(&hash).unwrap();
    assert_eq!(read_back.len(), 2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test tree_test`
Expected: FAIL

**Step 3: Implement tree.rs**

`crates/chkpt-core/src/store/tree.rs`:

```rust
use crate::error::{ChkptError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryType {
    File,
    Dir,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: String,
    pub entry_type: EntryType,
    pub hash: [u8; 32],
    pub size: u64,
    pub mode: u32,
}

pub struct TreeStore {
    base_dir: PathBuf,
}

impl TreeStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn tree_path(&self, hash_hex: &str) -> PathBuf {
        let (prefix, rest) = hash_hex.split_at(2);
        self.base_dir.join(prefix).join(rest)
    }

    /// Write tree entries (sorted by name). Returns hash hex.
    pub fn write(&self, entries: &[TreeEntry]) -> Result<String> {
        let mut sorted = entries.to_vec();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        let encoded = bincode::serialize(&sorted)?;
        let hash_hex = blake3::hash(&encoded).to_hex().to_string();
        let path = self.tree_path(&hash_hex);
        if path.exists() {
            return Ok(hash_hex);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, &encoded)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(hash_hex)
    }

    /// Read tree entries by hash.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<TreeEntry>> {
        let path = self.tree_path(hash_hex);
        if !path.exists() {
            return Err(ChkptError::ObjectNotFound(hash_hex.to_string()));
        }
        let data = std::fs::read(&path)?;
        let entries: Vec<TreeEntry> = bincode::deserialize(&data)?;
        Ok(entries)
    }

    pub fn exists(&self, hash_hex: &str) -> bool {
        self.tree_path(hash_hex).exists()
    }

    pub fn remove(&self, hash_hex: &str) -> Result<()> {
        let path = self.tree_path(hash_hex);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// List all loose tree hashes.
    pub fn list_loose(&self) -> Result<Vec<String>> {
        let mut hashes = Vec::new();
        if !self.base_dir.exists() {
            return Ok(hashes);
        }
        for prefix_entry in std::fs::read_dir(&self.base_dir)? {
            let prefix_entry = prefix_entry?;
            if !prefix_entry.file_type()?.is_dir() { continue; }
            let prefix = prefix_entry.file_name().to_string_lossy().to_string();
            for entry in std::fs::read_dir(prefix_entry.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    let rest = entry.file_name().to_string_lossy().to_string();
                    if !rest.ends_with(".tmp") {
                        hashes.push(format!("{}{}", prefix, rest));
                    }
                }
            }
        }
        Ok(hashes)
    }
}
```

Add to `crates/chkpt-core/src/store/mod.rs`:

```rust
pub mod blob;
pub mod tree;
```

**Step 4: Run tests**

Run: `cargo test -p chkpt-core --test tree_test`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add TreeStore with bincode serialization and 2-char prefix dirs"
```

---

## Task 5: Snapshot Store

**Files:**

- Create: `crates/chkpt-core/src/store/snapshot.rs`
- Create: `crates/chkpt-core/tests/snapshot_test.rs`
- Modify: `crates/chkpt-core/src/store/mod.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/snapshot_test.rs`:

```rust
use chkpt_core::store::snapshot::{Snapshot, SnapshotStore, SnapshotAttachments, SnapshotStats};
use tempfile::TempDir;
use chrono::Utc;

#[test]
fn test_snapshot_save_and_load() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap = Snapshot::new(
        Some("test save".into()),
        [0u8; 32],
        None,
        SnapshotAttachments::default(),
        SnapshotStats { total_files: 10, total_bytes: 1000, new_objects: 5 },
    );
    let id = snap.id.clone();
    store.save(&snap).unwrap();
    let loaded = store.load(&id).unwrap();
    assert_eq!(loaded.id, id);
    assert_eq!(loaded.message.as_deref(), Some("test save"));
    assert_eq!(loaded.root_tree_hash, [0u8; 32]);
}

#[test]
fn test_snapshot_list_sorted() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    for i in 0..3 {
        let snap = Snapshot::new(
            Some(format!("snap {}", i)),
            [i as u8; 32],
            None,
            SnapshotAttachments::default(),
            SnapshotStats { total_files: 0, total_bytes: 0, new_objects: 0 },
        );
        store.save(&snap).unwrap();
    }
    let list = store.list(None).unwrap();
    assert_eq!(list.len(), 3);
    // Should be newest first
    assert!(list[0].created_at >= list[1].created_at);
}

#[test]
fn test_snapshot_delete() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap = Snapshot::new(None, [0u8; 32], None, SnapshotAttachments::default(),
        SnapshotStats { total_files: 0, total_bytes: 0, new_objects: 0 });
    let id = snap.id.clone();
    store.save(&snap).unwrap();
    store.delete(&id).unwrap();
    assert!(store.load(&id).is_err());
}

#[test]
fn test_snapshot_list_with_limit() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    for _ in 0..5 {
        let snap = Snapshot::new(None, [0u8; 32], None, SnapshotAttachments::default(),
            SnapshotStats { total_files: 0, total_bytes: 0, new_objects: 0 });
        store.save(&snap).unwrap();
    }
    let list = store.list(Some(3)).unwrap();
    assert_eq!(list.len(), 3);
}

#[test]
fn test_snapshot_latest() {
    let dir = TempDir::new().unwrap();
    let store = SnapshotStore::new(dir.path().to_path_buf());
    let snap1 = Snapshot::new(Some("first".into()), [1u8; 32], None,
        SnapshotAttachments::default(),
        SnapshotStats { total_files: 0, total_bytes: 0, new_objects: 0 });
    store.save(&snap1).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let snap2 = Snapshot::new(Some("second".into()), [2u8; 32], None,
        SnapshotAttachments::default(),
        SnapshotStats { total_files: 0, total_bytes: 0, new_objects: 0 });
    let id2 = snap2.id.clone();
    store.save(&snap2).unwrap();
    let latest = store.latest().unwrap().unwrap();
    assert_eq!(latest.id, id2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test snapshot_test`
Expected: FAIL

**Step 3: Implement snapshot.rs**

`crates/chkpt-core/src/store/snapshot.rs`:

```rust
use crate::error::{ChkptError, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotAttachments {
    pub deps_key: Option<String>,
    pub git_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotStats {
    pub total_files: u64,
    pub total_bytes: u64,
    pub new_objects: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub message: Option<String>,
    pub root_tree_hash: [u8; 32],
    pub parent_snapshot_id: Option<String>,
    pub attachments: SnapshotAttachments,
    pub stats: SnapshotStats,
}

impl Snapshot {
    pub fn new(
        message: Option<String>,
        root_tree_hash: [u8; 32],
        parent_snapshot_id: Option<String>,
        attachments: SnapshotAttachments,
        stats: SnapshotStats,
    ) -> Self {
        Self {
            id: Uuid::now_v7().to_string(),
            created_at: Utc::now(),
            message,
            root_tree_hash,
            parent_snapshot_id,
            attachments,
            stats,
        }
    }
}

pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn snapshot_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn save(&self, snapshot: &Snapshot) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.snapshot_path(&snapshot.id);
        let json = serde_json::to_string_pretty(snapshot)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Snapshot> {
        let path = self.snapshot_path(id);
        if !path.exists() {
            return Err(ChkptError::SnapshotNotFound(id.to_string()));
        }
        let json = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.snapshot_path(id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn list(&self, limit: Option<usize>) -> Result<Vec<Snapshot>> {
        let mut snapshots = Vec::new();
        if !self.dir.exists() {
            return Ok(snapshots);
        }
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "json") {
                let json = std::fs::read_to_string(&path)?;
                if let Ok(snap) = serde_json::from_str::<Snapshot>(&json) {
                    snapshots.push(snap);
                }
            }
        }
        snapshots.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(limit) = limit {
            snapshots.truncate(limit);
        }
        Ok(snapshots)
    }

    pub fn latest(&self) -> Result<Option<Snapshot>> {
        let list = self.list(Some(1))?;
        Ok(list.into_iter().next())
    }

    /// Return all snapshot IDs.
    pub fn all_ids(&self) -> Result<Vec<String>> {
        Ok(self.list(None)?.into_iter().map(|s| s.id).collect())
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p chkpt-core --test snapshot_test`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add SnapshotStore with UUID v7 IDs and JSON persistence"
```

---

## Task 6: Packfile Store

**Files:**

- Create: `crates/chkpt-core/src/store/pack.rs`
- Create: `crates/chkpt-core/tests/pack_test.rs`
- Modify: `crates/chkpt-core/src/store/mod.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/pack_test.rs`:

```rust
use chkpt_core::store::pack::{PackWriter, PackReader, PackIndex};
use chkpt_core::store::blob::{BlobStore, hash_content};
use tempfile::TempDir;

#[test]
fn test_pack_write_and_read() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let entries: Vec<(String, Vec<u8>)> = vec![
        ("hello".into(), b"hello world".to_vec()),
        ("bye".into(), b"goodbye".to_vec()),
    ];
    let hashes: Vec<String> = entries.iter().map(|(_, data)| hash_content(data)).collect();

    let mut writer = PackWriter::new();
    for (_, data) in &entries {
        writer.add(data).unwrap();
    }
    let pack_hash = writer.finish(&packs_dir).unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    let data0 = reader.read(&hashes[0]).unwrap();
    assert_eq!(data0, b"hello world");
    let data1 = reader.read(&hashes[1]).unwrap();
    assert_eq!(data1, b"goodbye");
}

#[test]
fn test_pack_index_binary_search() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer = PackWriter::new();
    for i in 0..100 {
        let data = format!("content-{}", i);
        writer.add(data.as_bytes()).unwrap();
    }
    let pack_hash = writer.finish(&packs_dir).unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    let target = hash_content(b"content-50");
    let data = reader.read(&target).unwrap();
    assert_eq!(data, b"content-50");
}

#[test]
fn test_pack_not_found_returns_none() {
    let dir = TempDir::new().unwrap();
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&packs_dir).unwrap();

    let mut writer = PackWriter::new();
    writer.add(b"data").unwrap();
    let pack_hash = writer.finish(&packs_dir).unwrap();

    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    let result = reader.try_read(&"0".repeat(64));
    assert!(result.is_none());
}

#[test]
fn test_pack_from_loose_objects() {
    let dir = TempDir::new().unwrap();
    let objects_dir = dir.path().join("objects");
    let packs_dir = dir.path().join("packs");
    std::fs::create_dir_all(&objects_dir).unwrap();
    std::fs::create_dir_all(&packs_dir).unwrap();

    let blob_store = BlobStore::new(objects_dir.clone());
    let mut hashes = Vec::new();
    for i in 0..10 {
        let h = blob_store.write(format!("file-{}", i).as_bytes()).unwrap();
        hashes.push(h);
    }

    // Pack all loose objects
    let pack_hash = chkpt_core::store::pack::pack_loose_objects(
        &blob_store, &packs_dir
    ).unwrap();

    // Loose objects should be deleted
    assert_eq!(blob_store.list_loose().unwrap().len(), 0);

    // All data should be readable from pack
    let reader = PackReader::open(&packs_dir, &pack_hash).unwrap();
    for (i, hash) in hashes.iter().enumerate() {
        let data = reader.read(hash).unwrap();
        assert_eq!(data, format!("file-{}", i).as_bytes());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test pack_test`
Expected: FAIL

**Step 3: Implement pack.rs**

`crates/chkpt-core/src/store/pack.rs`:

```rust
use crate::error::{ChkptError, Result};
use crate::store::blob::BlobStore;
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const PACK_MAGIC: &[u8; 4] = b"CHKP";
const PACK_VERSION: u32 = 1;
const IDX_ENTRY_SIZE: usize = 32 + 8 + 8; // hash + offset + size

/// In-memory index entry for a pack.
#[derive(Debug, Clone)]
struct IndexEntry {
    hash: [u8; 32],
    offset: u64,
    size: u64,
}

pub struct PackWriter {
    entries: Vec<(String, Vec<u8>)>, // (hash_hex, compressed_data)
}

impl PackWriter {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn add(&mut self, content: &[u8]) -> Result<String> {
        let hash_hex = crate::store::blob::hash_content(content);
        let compressed = zstd::encode_all(content, 3)?;
        self.entries.push((hash_hex.clone(), compressed));
        Ok(hash_hex)
    }

    pub fn add_pre_compressed(&mut self, hash_hex: String, compressed: Vec<u8>) {
        self.entries.push((hash_hex, compressed));
    }

    /// Write pack .dat and .idx files. Returns pack hash.
    pub fn finish(mut self, packs_dir: &Path) -> Result<String> {
        if self.entries.is_empty() {
            return Err(ChkptError::Other("No entries to pack".into()));
        }

        // Sort entries by hash for binary search in idx
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Build .dat
        let mut dat_buf: Vec<u8> = Vec::new();
        dat_buf.extend_from_slice(PACK_MAGIC);
        dat_buf.extend_from_slice(&PACK_VERSION.to_le_bytes());
        dat_buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        let mut idx_entries: Vec<IndexEntry> = Vec::new();

        for (hash_hex, compressed) in &self.entries {
            let hash_bytes = hex_to_bytes(hash_hex)?;
            let offset = dat_buf.len() as u64;
            dat_buf.extend_from_slice(&hash_bytes);
            dat_buf.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
            dat_buf.extend_from_slice(compressed);
            idx_entries.push(IndexEntry {
                hash: hash_bytes,
                offset,
                size: compressed.len() as u64,
            });
        }

        let pack_hash = blake3::hash(&dat_buf).to_hex()[..16].to_string();
        let dat_path = packs_dir.join(format!("pack-{}.dat", pack_hash));
        let idx_path = packs_dir.join(format!("pack-{}.idx", pack_hash));

        // Write .dat
        std::fs::create_dir_all(packs_dir)?;
        std::fs::write(&dat_path, &dat_buf)?;

        // Write .idx (sorted by hash)
        let mut idx_buf: Vec<u8> = Vec::new();
        for entry in &idx_entries {
            idx_buf.extend_from_slice(&entry.hash);
            idx_buf.extend_from_slice(&entry.offset.to_le_bytes());
            idx_buf.extend_from_slice(&entry.size.to_le_bytes());
        }
        std::fs::write(&idx_path, &idx_buf)?;

        Ok(pack_hash)
    }
}

pub struct PackReader {
    dat: Vec<u8>,
    idx: Vec<IndexEntry>,
}

impl PackReader {
    pub fn open(packs_dir: &Path, pack_hash: &str) -> Result<Self> {
        let dat_path = packs_dir.join(format!("pack-{}.dat", pack_hash));
        let idx_path = packs_dir.join(format!("pack-{}.idx", pack_hash));
        let dat = std::fs::read(&dat_path)?;
        let idx_raw = std::fs::read(&idx_path)?;

        let mut idx = Vec::new();
        let mut pos = 0;
        while pos + IDX_ENTRY_SIZE <= idx_raw.len() {
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&idx_raw[pos..pos + 32]);
            let offset = u64::from_le_bytes(idx_raw[pos + 32..pos + 40].try_into().unwrap());
            let size = u64::from_le_bytes(idx_raw[pos + 40..pos + 48].try_into().unwrap());
            idx.push(IndexEntry { hash, offset, size });
            pos += IDX_ENTRY_SIZE;
        }

        Ok(Self { dat, idx })
    }

    /// Binary search for hash in index.
    fn find(&self, hash_hex: &str) -> Option<&IndexEntry> {
        let hash_bytes = hex_to_bytes(hash_hex).ok()?;
        self.idx
            .binary_search_by(|e| e.hash.cmp(&hash_bytes))
            .ok()
            .map(|i| &self.idx[i])
    }

    /// Read and decompress an object. Returns None if not found.
    pub fn try_read(&self, hash_hex: &str) -> Option<Vec<u8>> {
        let entry = self.find(hash_hex)?;
        let data_start = entry.offset as usize + 32 + 8; // skip hash + compressed_size
        let data_end = data_start + entry.size as usize;
        if data_end > self.dat.len() {
            return None;
        }
        let compressed = &self.dat[data_start..data_end];
        zstd::decode_all(compressed).ok()
    }

    /// Read and decompress an object. Error if not found.
    pub fn read(&self, hash_hex: &str) -> Result<Vec<u8>> {
        self.try_read(hash_hex)
            .ok_or_else(|| ChkptError::ObjectNotFound(hash_hex.to_string()))
    }

    /// List all hashes in this pack.
    pub fn hashes(&self) -> Vec<String> {
        self.idx.iter().map(|e| bytes_to_hex(&e.hash)).collect()
    }
}

/// List all pack hashes in a directory.
pub fn list_packs(packs_dir: &Path) -> Result<Vec<String>> {
    let mut packs = Vec::new();
    if !packs_dir.exists() {
        return Ok(packs);
    }
    for entry in std::fs::read_dir(packs_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("pack-") && name.ends_with(".dat") {
            let hash = name.strip_prefix("pack-").unwrap().strip_suffix(".dat").unwrap();
            packs.push(hash.to_string());
        }
    }
    Ok(packs)
}

/// Pack all loose objects from a BlobStore into a pack file, then delete loose objects.
pub fn pack_loose_objects(blob_store: &BlobStore, packs_dir: &Path) -> Result<String> {
    let loose = blob_store.list_loose()?;
    if loose.is_empty() {
        return Err(ChkptError::Other("No loose objects to pack".into()));
    }
    let mut writer = PackWriter::new();
    for hash in &loose {
        let path = blob_store.read(hash)?;
        // Re-compress from raw content
        writer.add(&path)?;
    }
    let pack_hash = writer.finish(packs_dir)?;
    // Delete loose objects
    for hash in &loose {
        blob_store.remove(hash)?;
    }
    Ok(pack_hash)
}

/// Read an object: first check loose, then packs.
pub fn read_object(blob_store: &BlobStore, packs_dir: &Path, hash_hex: &str) -> Result<Vec<u8>> {
    // 1. Check loose
    if blob_store.exists(hash_hex) {
        return blob_store.read(hash_hex);
    }
    // 2. Check packs
    for pack_hash in list_packs(packs_dir)? {
        let reader = PackReader::open(packs_dir, &pack_hash)?;
        if let Some(data) = reader.try_read(hash_hex) {
            return Ok(data);
        }
    }
    Err(ChkptError::ObjectNotFound(hash_hex.to_string()))
}

fn hex_to_bytes(hex: &str) -> Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if hex.len() != 64 {
        return Err(ChkptError::Other(format!("Invalid hash length: {}", hex.len())));
    }
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| ChkptError::Other("Invalid hex".into()))?;
    }
    Ok(bytes)
}

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
```

**Step 4: Run tests**

Run: `cargo test -p chkpt-core --test pack_test`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add PackWriter/PackReader with binary index and loose-to-pack migration"
```

---

## Task 7: SQLite Index (Change Detection Cache)

**Files:**

- Create: `crates/chkpt-core/src/index/mod.rs`
- Create: `crates/chkpt-core/src/index/schema.rs`
- Create: `crates/chkpt-core/tests/index_test.rs`
- Modify: `crates/chkpt-core/src/lib.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/index_test.rs`:

```rust
use chkpt_core::index::{FileIndex, FileEntry};
use tempfile::TempDir;

#[test]
fn test_index_insert_and_get() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entry = FileEntry {
        path: "src/main.rs".into(),
        blob_hash: [1u8; 32],
        size: 100,
        mtime_secs: 1000,
        mtime_nanos: 500,
        inode: Some(12345),
        mode: 0o644,
    };
    idx.upsert(&entry).unwrap();
    let loaded = idx.get("src/main.rs").unwrap().unwrap();
    assert_eq!(loaded.size, 100);
    assert_eq!(loaded.blob_hash, [1u8; 32]);
}

#[test]
fn test_index_get_nonexistent() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    assert!(idx.get("nope").unwrap().is_none());
}

#[test]
fn test_index_upsert_updates() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entry = FileEntry {
        path: "a.txt".into(),
        blob_hash: [0u8; 32],
        size: 10,
        mtime_secs: 100,
        mtime_nanos: 0,
        inode: None,
        mode: 0o644,
    };
    idx.upsert(&entry).unwrap();
    let mut updated = entry.clone();
    updated.size = 20;
    updated.blob_hash = [1u8; 32];
    idx.upsert(&updated).unwrap();
    let loaded = idx.get("a.txt").unwrap().unwrap();
    assert_eq!(loaded.size, 20);
    assert_eq!(loaded.blob_hash, [1u8; 32]);
}

#[test]
fn test_index_remove() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entry = FileEntry {
        path: "del.txt".into(),
        blob_hash: [0u8; 32],
        size: 5,
        mtime_secs: 50,
        mtime_nanos: 0,
        inode: None,
        mode: 0o644,
    };
    idx.upsert(&entry).unwrap();
    idx.remove("del.txt").unwrap();
    assert!(idx.get("del.txt").unwrap().is_none());
}

#[test]
fn test_index_all_paths() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    for name in &["a.txt", "b.txt", "c.txt"] {
        idx.upsert(&FileEntry {
            path: name.to_string(),
            blob_hash: [0u8; 32],
            size: 1,
            mtime_secs: 1,
            mtime_nanos: 0,
            inode: None,
            mode: 0o644,
        }).unwrap();
    }
    let paths = idx.all_paths().unwrap();
    assert_eq!(paths.len(), 3);
}

#[test]
fn test_index_bulk_upsert() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    let entries: Vec<FileEntry> = (0..100).map(|i| FileEntry {
        path: format!("file_{}.txt", i),
        blob_hash: [i as u8; 32],
        size: i as u64,
        mtime_secs: 1000 + i as i64,
        mtime_nanos: 0,
        inode: None,
        mode: 0o644,
    }).collect();
    idx.bulk_upsert(&entries).unwrap();
    assert_eq!(idx.all_paths().unwrap().len(), 100);
}

#[test]
fn test_index_clear() {
    let dir = TempDir::new().unwrap();
    let idx = FileIndex::open(dir.path().join("index.sqlite")).unwrap();
    idx.upsert(&FileEntry {
        path: "x.txt".into(), blob_hash: [0u8; 32], size: 1,
        mtime_secs: 1, mtime_nanos: 0, inode: None, mode: 0o644,
    }).unwrap();
    idx.clear().unwrap();
    assert_eq!(idx.all_paths().unwrap().len(), 0);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test index_test`
Expected: FAIL

**Step 3: Implement index module**

`crates/chkpt-core/src/index/schema.rs`:

```rust
pub const CREATE_TABLES: &str = "
CREATE TABLE IF NOT EXISTS file_index (
    path        TEXT PRIMARY KEY,
    blob_hash   BLOB NOT NULL,
    size        INTEGER NOT NULL,
    mtime_secs  INTEGER NOT NULL,
    mtime_nanos INTEGER NOT NULL,
    inode       INTEGER,
    mode        INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";
```

`crates/chkpt-core/src/index/mod.rs`:

```rust
pub mod schema;

use crate::error::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: String,
    pub blob_hash: [u8; 32],
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub inode: Option<u64>,
    pub mode: u32,
}

pub struct FileIndex {
    conn: Connection,
}

impl FileIndex {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(schema::CREATE_TABLES)?;
        Ok(Self { conn })
    }

    pub fn upsert(&self, entry: &FileEntry) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_index (path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(path) DO UPDATE SET
                blob_hash=excluded.blob_hash, size=excluded.size,
                mtime_secs=excluded.mtime_secs, mtime_nanos=excluded.mtime_nanos,
                inode=excluded.inode, mode=excluded.mode",
            params![
                entry.path, entry.blob_hash.as_slice(), entry.size as i64,
                entry.mtime_secs, entry.mtime_nanos,
                entry.inode.map(|i| i as i64), entry.mode,
            ],
        )?;
        Ok(())
    }

    pub fn bulk_upsert(&self, entries: &[FileEntry]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for entry in entries {
            tx.execute(
                "INSERT INTO file_index (path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(path) DO UPDATE SET
                    blob_hash=excluded.blob_hash, size=excluded.size,
                    mtime_secs=excluded.mtime_secs, mtime_nanos=excluded.mtime_nanos,
                    inode=excluded.inode, mode=excluded.mode",
                params![
                    entry.path, entry.blob_hash.as_slice(), entry.size as i64,
                    entry.mtime_secs, entry.mtime_nanos,
                    entry.inode.map(|i| i as i64), entry.mode,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self, path: &str) -> Result<Option<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode FROM file_index WHERE path = ?1"
        )?;
        let result = stmt.query_row(params![path], |row| {
            let hash_blob: Vec<u8> = row.get(1)?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_blob);
            Ok(FileEntry {
                path: row.get(0)?,
                blob_hash: hash,
                size: row.get::<_, i64>(2)? as u64,
                mtime_secs: row.get(3)?,
                mtime_nanos: row.get(4)?,
                inode: row.get::<_, Option<i64>>(5)?.map(|i| i as u64),
                mode: row.get(6)?,
            })
        }).optional()?;
        Ok(result)
    }

    pub fn remove(&self, path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM file_index WHERE path = ?1", params![path])?;
        Ok(())
    }

    pub fn all_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM file_index")?;
        let paths = stmt.query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn all_entries(&self) -> Result<Vec<FileEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, blob_hash, size, mtime_secs, mtime_nanos, inode, mode FROM file_index"
        )?;
        let entries = stmt.query_map([], |row| {
            let hash_blob: Vec<u8> = row.get(1)?;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&hash_blob);
            Ok(FileEntry {
                path: row.get(0)?,
                blob_hash: hash,
                size: row.get::<_, i64>(2)? as u64,
                mtime_secs: row.get(3)?,
                mtime_nanos: row.get(4)?,
                inode: row.get::<_, Option<i64>>(5)?.map(|i| i as u64),
                mode: row.get(6)?,
            })
        })?.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM file_index", [])?;
        Ok(())
    }
}
```

**Step 4: Run tests**

Run: `cargo test -p chkpt-core --test index_test`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add SQLite-based FileIndex with WAL mode and bulk upsert"
```

---

## Task 8: Scanner (.chkptignore + filesystem walking)

**Files:**

- Create: `crates/chkpt-core/src/scanner/mod.rs`
- Create: `crates/chkpt-core/src/scanner/walker.rs`
- Create: `crates/chkpt-core/src/scanner/matcher.rs`
- Create: `crates/chkpt-core/tests/scanner_test.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/scanner_test.rs`:

```rust
use chkpt_core::scanner::{scan_workspace, ScannedFile};
use tempfile::TempDir;
use std::fs;

#[test]
fn test_scan_basic_files() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "hello").unwrap();
    fs::write(dir.path().join("b.txt"), "world").unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main(){}").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert_eq!(files.len(), 3);
}

#[test]
fn test_scan_respects_chkptignore() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "keep").unwrap();
    fs::write(dir.path().join("b.log"), "ignore").unwrap();
    fs::create_dir_all(dir.path().join("build")).unwrap();
    fs::write(dir.path().join("build/out.o"), "ignore").unwrap();
    fs::write(dir.path().join(".chkptignore"), "*.log\nbuild/\n").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(!paths.contains(&"b.log"));
    assert!(!paths.contains(&"build/out.o"));
    // .chkptignore itself should be included
    assert!(paths.contains(&".chkptignore"));
}

#[test]
fn test_scan_excludes_chkpt_dir() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "data").unwrap();
    fs::create_dir_all(dir.path().join(".chkpt")).unwrap();
    fs::write(dir.path().join(".chkpt/config"), "x").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    assert!(paths.contains(&"a.txt"));
    assert!(!paths.iter().any(|p| p.starts_with(".chkpt")));
}

#[test]
fn test_scan_excludes_git_dir_by_default() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.txt"), "data").unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/HEAD"), "ref").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert!(!files.iter().any(|f| f.relative_path.starts_with(".git")));
}

#[test]
fn test_scan_excludes_node_modules_by_default() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("index.js"), "code").unwrap();
    fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
    fs::write(dir.path().join("node_modules/pkg/index.js"), "dep").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert!(!files.iter().any(|f| f.relative_path.starts_with("node_modules")));
}

#[test]
fn test_scanned_file_has_metadata() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("test.txt"), "content").unwrap();

    let files = scan_workspace(dir.path(), None).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "test.txt");
    assert_eq!(files[0].size, 7);
    assert!(files[0].mtime_secs > 0);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p chkpt-core --test scanner_test`
Expected: FAIL

**Step 3: Implement scanner module**

See design doc for details. The scanner uses the `ignore` crate for .chkptignore support (gitignore-compatible syntax) and walks the filesystem collecting `ScannedFile` structs with path, size, mtime, inode, mode. Built-in exclusions: `.git/`, `node_modules/`, `.chkpt/`.

`crates/chkpt-core/src/scanner/matcher.rs` — wraps `ignore::gitignore::GitignoreBuilder` to parse `.chkptignore`.

`crates/chkpt-core/src/scanner/walker.rs` — recursive directory walk with ignore matching.

`crates/chkpt-core/src/scanner/mod.rs` — exports `scan_workspace(root, chkptignore_path) -> Result<Vec<ScannedFile>>`.

```rust
// scanner/mod.rs
pub mod walker;
pub mod matcher;

use crate::error::Result;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub relative_path: String,
    pub absolute_path: std::path::PathBuf,
    pub size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: i64,
    pub inode: Option<u64>,
    pub mode: u32,
}

/// Scan workspace, respecting .chkptignore and built-in exclusions.
pub fn scan_workspace(root: &Path, chkptignore: Option<&Path>) -> Result<Vec<ScannedFile>> {
    walker::walk(root, chkptignore)
}
```

Implementation of walker.rs uses `std::fs` recursive walk (not async for v1 simplicity in scanner), checks matcher for each entry, collects metadata via `std::fs::metadata`.

**Step 4: Run tests**

Run: `cargo test -p chkpt-core --test scanner_test`
Expected: PASS

**Step 5: Commit**

```bash
git add -A
git commit -m "feat: add scanner with .chkptignore support and built-in exclusions"
```

---

## Task 9: Lock Module

**Files:**

- Create: `crates/chkpt-core/src/ops/mod.rs`
- Create: `crates/chkpt-core/src/ops/lock.rs`
- Create: `crates/chkpt-core/tests/lock_test.rs`

**Step 1: Write failing tests**

`crates/chkpt-core/tests/lock_test.rs`:

```rust
use chkpt_core::ops::lock::ProjectLock;
use tempfile::TempDir;

#[test]
fn test_lock_acquire_release() {
    let dir = TempDir::new().unwrap();
    let lock_dir = dir.path().join("locks");
    std::fs::create_dir_all(&lock_dir).unwrap();
    let lock = ProjectLock::acquire(&lock_dir).unwrap();
    drop(lock); // should release
}

#[test]
fn test_double_lock_fails() {
    let dir = TempDir::new().unwrap();
    let lock_dir = dir.path().join("locks");
    std::fs::create_dir_all(&lock_dir).unwrap();
    let _lock1 = ProjectLock::acquire(&lock_dir).unwrap();
    let result = ProjectLock::try_acquire(&lock_dir);
    assert!(result.is_err() || result.unwrap().is_none());
}
```

**Step 2: Implement lock.rs**

Uses `fs4::FileExt` for `try_lock_exclusive()` on `locks/project.lock`. Returns `ChkptError::LockHeld` on failure.

**Step 3: Run tests, commit**

```bash
git add -A
git commit -m "feat: add file-based project lock with fs4"
```

---

## Task 10: Save Operation

**Files:**

- Create: `crates/chkpt-core/src/ops/save.rs`
- Create: `crates/chkpt-core/tests/save_test.rs`
- Modify: `crates/chkpt-core/src/ops/mod.rs`

**Step 1: Write failing integration test**

`crates/chkpt-core/tests/save_test.rs`:

```rust
use chkpt_core::ops::save::{save, SaveOptions, SaveResult};
use tempfile::TempDir;
use std::fs;

#[test]
fn test_save_basic() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("hello.txt"), "hello").unwrap();
    fs::write(workspace.path().join("world.txt"), "world").unwrap();

    let result = save(workspace.path(), SaveOptions::default()).unwrap();
    assert!(!result.snapshot_id.is_empty());
    assert_eq!(result.stats.total_files, 2);
    assert_eq!(result.stats.new_objects, 2);
}

#[test]
fn test_save_incremental_dedup() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "content").unwrap();

    let r1 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r1.stats.new_objects, 1);

    // Second save with no changes: no new objects
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r2.stats.new_objects, 0);
}

#[test]
fn test_save_detects_changes() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "v1").unwrap();
    save(workspace.path(), SaveOptions::default()).unwrap();

    // Modify file
    fs::write(workspace.path().join("a.txt"), "v2").unwrap();
    let r2 = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(r2.stats.new_objects, 1);
}

#[test]
fn test_save_with_message() {
    let workspace = TempDir::new().unwrap();
    fs::write(workspace.path().join("a.txt"), "data").unwrap();

    let opts = SaveOptions { message: Some("my checkpoint".into()), ..Default::default() };
    let result = save(workspace.path(), opts).unwrap();
    assert!(!result.snapshot_id.is_empty());
}

#[test]
fn test_save_with_subdirectories() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src/utils")).unwrap();
    fs::write(workspace.path().join("src/main.rs"), "fn main(){}").unwrap();
    fs::write(workspace.path().join("src/utils/helper.rs"), "fn help(){}").unwrap();

    let result = save(workspace.path(), SaveOptions::default()).unwrap();
    assert_eq!(result.stats.total_files, 2);
}
```

**Step 2: Implement save.rs**

Orchestrates: lock → scan → index compare → blob store → tree build (bottom-up) → snapshot create → index update → auto-pack check → unlock.

**Step 3: Run tests, commit**

```bash
git add -A
git commit -m "feat: add save operation with incremental change detection and tree building"
```

---

## Task 11: List Operation

**Files:**

- Create: `crates/chkpt-core/src/ops/list.rs`
- Create: `crates/chkpt-core/tests/list_test.rs`

Thin wrapper around `SnapshotStore::list()`. Tested as part of save→list flow.

**Commit:**

```bash
git commit -m "feat: add list operation"
```

---

## Task 12: Restore Operation (hard + dry-run)

**Files:**

- Create: `crates/chkpt-core/src/ops/restore.rs`
- Create: `crates/chkpt-core/tests/restore_test.rs`

**Step 1: Write failing tests**

```rust
// Key tests:
// 1. save → modify → restore → verify original state
// 2. dry-run returns summary without modifying workspace
// 3. restore "latest" alias works
// 4. restore with added/deleted files
// 5. atomic: if restore partially fails, workspace is unchanged
```

**Step 2: Implement restore.rs**

Implements the atomic restore flow from the design doc:

1. Lock
2. Load snapshot, walk tree to get target state
3. Compare with current workspace
4. dry-run: return diff summary
5. actual: prepare in temp dir → atomic swap → cleanup
6. Reset index
7. Unlock

**Step 3: Run tests, commit**

```bash
git commit -m "feat: add atomic restore operation with dry-run support"
```

---

## Task 13: Delete + GC Operation

**Files:**

- Create: `crates/chkpt-core/src/ops/delete.rs`
- Create: `crates/chkpt-core/tests/delete_test.rs`

**Step 1: Write failing tests**

```rust
// Key tests:
// 1. save → delete → list is empty
// 2. save A → save B → delete A → B still restorable
// 3. GC removes unreferenced objects
// 4. GC repacks when pack contains unreferenced entries
// 5. shared objects between snapshots are not deleted prematurely
```

**Step 2: Implement delete.rs**

Mark & Sweep GC: collect all reachable hashes from remaining snapshots → delete unreachable loose objects → repack if needed → delete unreachable attachments.

**Step 3: Run tests, commit**

```bash
git commit -m "feat: add delete with mark-and-sweep GC"
```

---

## Task 14: Attachments — deps (node_modules archive)

**Files:**

- Create: `crates/chkpt-core/src/attachments/mod.rs`
- Create: `crates/chkpt-core/src/attachments/deps.rs`
- Create: `crates/chkpt-core/tests/deps_test.rs`

**Step 1: Write failing tests**

```rust
// Key tests:
// 1. compute deps_key from lockfile
// 2. create tar.zst archive of node_modules
// 3. restore archive to node_modules
// 4. same lockfile → same deps_key (reuse)
// 5. different lockfile → different deps_key
```

**Step 2: Implement deps.rs**

`deps_key = blake3(lockfile_contents + package_manager + node_version + platform + arch)`. Archive with `tar` + `zstd`. Store at `attachments/deps/<deps_key>.tar.zst`.

**Step 3: Run tests, commit**

```bash
git commit -m "feat: add deps attachment layer for node_modules archiving"
```

---

## Task 15: Attachments — git bundle

**Files:**

- Create: `crates/chkpt-core/src/attachments/git.rs`
- Create: `crates/chkpt-core/tests/git_attachment_test.rs`

**Step 1: Write failing tests**

```rust
// Key tests:
// 1. create git bundle from a repo
// 2. restore (unbundle) to recover refs
// 3. git_key is based on bundle content hash
```

**Step 2: Implement git.rs**

Shells out to `git bundle create <path> --all` and `git bundle unbundle <path>`. Stores at `attachments/git/<git_key>.bundle` where `git_key = blake3(bundle_content)` hex prefix.

**Step 3: Run tests, commit**

```bash
git commit -m "feat: add git bundle attachment layer"
```

---

## Task 16: CLI (clap)

**Files:**

- Modify: `crates/chkpt-cli/src/main.rs`
- Create: `crates/chkpt-cli/tests/cli_test.rs` (optional, integration)

**Step 1: Implement CLI commands**

```rust
// Commands: save, list, restore, delete, init
// Each command calls chkpt_core::ops functions
// Output formatting: human-readable tables and status messages
```

**Step 2: Manual testing**

```bash
cargo run -p chkpt-cli -- save -m "test"
cargo run -p chkpt-cli -- list
cargo run -p chkpt-cli -- restore latest --dry-run
cargo run -p chkpt-cli -- restore latest
cargo run -p chkpt-cli -- delete <id>
```

**Step 3: Commit**

```bash
git commit -m "feat: add chkpt CLI with save/list/restore/delete commands"
```

---

## Task 17: MCP Server (stdio)

**Files:**

- Modify: `crates/chkpt-mcp/src/main.rs`

**Step 1: Implement MCP server**

Using `rmcp` crate with `#[tool_router]` and `#[tool_handler]` macros. Four tools: `checkpoint_save`, `checkpoint_list`, `checkpoint_restore`, `checkpoint_delete`. Each tool delegates to `chkpt_core::ops`.

**Step 2: Test with MCP inspector or Claude Code**

Run: `cargo run -p chkpt-mcp` (connects via stdio)

**Step 3: Commit**

```bash
git commit -m "feat: add MCP stdio server with checkpoint tools"
```

---

## Task 18: End-to-End Integration Tests

**Files:**

- Create: `crates/chkpt-core/tests/e2e_test.rs`

**Step 1: Write comprehensive integration tests**

```rust
// Full flow tests:
// 1. save → list → restore → verify
// 2. save → modify → save → restore first → verify
// 3. save with deps → restore with deps
// 4. save with git → restore with git
// 5. save → delete → GC → verify objects cleaned
// 6. guardrail violation → error
// 7. concurrent lock detection
// 8. pack auto-creation after threshold
// 9. large file count scenario (1000+ files)
```

**Step 2: Run all tests**

Run: `cargo test --workspace`
Expected: ALL PASS

**Step 3: Commit**

```bash
git commit -m "test: add end-to-end integration tests for full checkpoint lifecycle"
```

---

## Task Summary

| #   | Task                  | Depends On             |
| --- | --------------------- | ---------------------- |
| 1   | Workspace scaffolding | —                      |
| 2   | Error types & config  | 1                      |
| 3   | Blob store            | 2                      |
| 4   | Tree store            | 2                      |
| 5   | Snapshot store        | 2                      |
| 6   | Packfile store        | 3                      |
| 7   | SQLite index          | 2                      |
| 8   | Scanner               | 2                      |
| 9   | Lock module           | 2                      |
| 10  | Save operation        | 3, 4, 5, 7, 8, 9       |
| 11  | List operation        | 5                      |
| 12  | Restore operation     | 3, 4, 5, 6, 7, 9       |
| 13  | Delete + GC           | 3, 4, 5, 6, 9          |
| 14  | Attachments: deps     | 2                      |
| 15  | Attachments: git      | 2                      |
| 16  | CLI                   | 10, 11, 12, 13, 14, 15 |
| 17  | MCP server            | 10, 11, 12, 13, 14, 15 |
| 18  | E2E tests             | 16, 17                 |

**Parallelizable groups:**

- Tasks 3, 4, 5, 7, 8, 9 can all proceed in parallel after Task 2
- Tasks 14, 15 can proceed in parallel with Tasks 10-13
- Tasks 16, 17 can proceed in parallel with each other
