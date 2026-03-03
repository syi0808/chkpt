# N-API JS SDK + npm CLI Package Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a Node.js SDK (via napi-rs) exposing all chkpt-core APIs and distribute it with the Rust CLI binary as a single npm package `chkpt`.

**Architecture:** A new `chkpt-napi` Rust crate uses `#[napi]` macros to wrap chkpt-core functions as async JS functions. Platform-specific npm packages contain both the `.node` N-API module and the Rust CLI binary. The main `chkpt` npm package auto-loads the correct platform package and provides a thin `cli.mjs` launcher that exec's the Rust binary directly.

**Tech Stack:** Rust, napi-rs (v2), chkpt-core, Node.js >= 18, vitest (testing)

---

### Task 1: Scaffold chkpt-napi crate and npm project

**Files:**

- Create: `crates/chkpt-napi/Cargo.toml`
- Create: `crates/chkpt-napi/build.rs`
- Create: `crates/chkpt-napi/src/lib.rs`
- Create: `crates/chkpt-napi/package.json`
- Modify: `Cargo.toml:3` (add `chkpt-napi` to workspace members)

**Step 1: Add chkpt-napi to workspace members**

In `Cargo.toml` line 3, change:

```toml
members = ["crates/chkpt-core", "crates/chkpt-cli", "crates/chkpt-mcp"]
```

to:

```toml
members = ["crates/chkpt-core", "crates/chkpt-cli", "crates/chkpt-mcp", "crates/chkpt-napi"]
```

**Step 2: Create Cargo.toml**

Create `crates/chkpt-napi/Cargo.toml`:

```toml
[package]
name = "chkpt-napi"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
chkpt-core = { path = "../chkpt-core" }
napi = { version = "2", default-features = false, features = ["async", "serde-json", "napi9"] }
napi-derive = "2"
serde_json = { workspace = true }

[build-dependencies]
napi-build = "2"
```

**Step 3: Create build.rs**

Create `crates/chkpt-napi/build.rs`:

```rust
extern crate napi_build;

fn main() {
    napi_build::setup();
}
```

**Step 4: Create minimal lib.rs**

Create `crates/chkpt-napi/src/lib.rs`:

```rust
#[macro_use]
extern crate napi_derive;

mod error;

#[napi]
pub fn ping() -> String {
    "pong".to_string()
}
```

**Step 5: Create error.rs**

Create `crates/chkpt-napi/src/error.rs`:

```rust
use chkpt_core::error::ChkptError;

pub fn to_napi_error(err: ChkptError) -> napi::Error {
    napi::Error::new(napi::Status::GenericFailure, err.to_string())
}
```

**Step 6: Create package.json**

Create `crates/chkpt-napi/package.json`:

```json
{
  "name": "chkpt",
  "version": "0.1.0",
  "description": "Filesystem checkpoint engine - save, restore, and manage workspace snapshots",
  "main": "index.js",
  "types": "index.d.ts",
  "bin": {
    "chkpt": "./cli.mjs"
  },
  "napi": {
    "name": "chkpt",
    "triples": {
      "defaults": false,
      "additional": [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-musl",
        "x86_64-unknown-linux-musl",
        "x86_64-pc-windows-msvc"
      ]
    }
  },
  "scripts": {
    "artifacts": "napi artifacts",
    "build": "napi build --platform --release",
    "build:debug": "napi build --platform",
    "prepublishOnly": "napi prepublish -t npm",
    "test": "vitest run",
    "version": "napi version"
  },
  "engines": {
    "node": ">= 18"
  },
  "license": "MIT",
  "devDependencies": {
    "@napi-rs/cli": "^3",
    "vitest": "^3"
  },
  "files": ["index.js", "index.d.ts", "cli.mjs"]
}
```

**Step 7: Verify build compiles**

Run: `cd /Users/classting/Workspace/temp/chkpt && cargo build -p chkpt-napi`
Expected: Successful compilation (may show warnings)

**Step 8: Initialize npm and build native module**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm install && npm run build`
Expected: Generates `index.js`, `index.d.ts`, and `chkpt.*.node` file

**Step 9: Verify ping works from Node.js**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && node -e "const m = require('./index.js'); console.log(m.ping())"`
Expected: Prints `pong`

**Step 10: Commit**

```bash
git add crates/chkpt-napi/Cargo.toml crates/chkpt-napi/build.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/src/error.rs crates/chkpt-napi/package.json Cargo.toml Cargo.lock
git commit -m "feat(napi): scaffold chkpt-napi crate with napi-rs"
```

---

### Task 2: Implement config bindings (getProjectId, getStoreLayout)

**Files:**

- Create: `crates/chkpt-napi/src/config.rs`
- Modify: `crates/chkpt-napi/src/lib.rs`

**Step 1: Write JS test for config bindings**

Create `crates/chkpt-napi/__test__/config.spec.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { getProjectId, getStoreLayout } from "../index.js";
import { mkdtempSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("config", () => {
  it("getProjectId returns a 16-char hex string", () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-test-"));
    const id = getProjectId(dir);
    expect(id).toMatch(/^[0-9a-f]{16}$/);
  });

  it("getProjectId is deterministic for same path", () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-test-"));
    expect(getProjectId(dir)).toBe(getProjectId(dir));
  });

  it("getStoreLayout returns all required paths", () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-test-"));
    const layout = getStoreLayout(dir);
    expect(layout.root).toContain(".chkpt/stores/");
    expect(layout.objectsDir).toContain("objects");
    expect(layout.treesDir).toContain("trees");
    expect(layout.snapshotsDir).toContain("snapshots");
    expect(layout.indexPath).toContain("index.sqlite");
    expect(layout.locksDir).toContain("locks");
    expect(layout.attachmentsDepsDir).toContain("deps");
    expect(layout.attachmentsGitDir).toContain("git");
    expect(layout.packsDir).toContain("packs");
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/config.spec.ts`
Expected: FAIL — `getProjectId` and `getStoreLayout` not found

**Step 3: Create config.rs bindings**

Create `crates/chkpt-napi/src/config.rs`:

```rust
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
```

**Step 4: Register module in lib.rs**

Update `crates/chkpt-napi/src/lib.rs`:

```rust
#[macro_use]
extern crate napi_derive;

mod error;
mod config;

#[napi]
pub fn ping() -> String {
    "pong".to_string()
}
```

**Step 5: Build and run tests**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/config.spec.ts`
Expected: All 3 tests PASS

**Step 6: Commit**

```bash
git add crates/chkpt-napi/src/config.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/__test__/config.spec.ts
git commit -m "feat(napi): add config bindings (getProjectId, getStoreLayout)"
```

---

### Task 3: Implement store bindings (blob, tree, snapshot)

**Files:**

- Create: `crates/chkpt-napi/src/store.rs`
- Modify: `crates/chkpt-napi/src/lib.rs`

**Step 1: Write JS test for blob operations**

Create `crates/chkpt-napi/__test__/store.spec.ts`:

```typescript
import { describe, it, expect, beforeEach } from "vitest";
import {
  blobHash,
  blobStore,
  blobLoad,
  blobExists,
  treeBuild,
  treeLoad,
  snapshotSave,
  snapshotLoad,
  snapshotList,
} from "../index.js";
import { mkdtempSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("blob store", () => {
  let storeDir: string;

  beforeEach(() => {
    storeDir = mkdtempSync(join(tmpdir(), "chkpt-blob-"));
    mkdirSync(join(storeDir, "objects"), { recursive: true });
  });

  it("blobHash returns 64-char hex", () => {
    const hash = blobHash(Buffer.from("hello world"));
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });

  it("blobHash is deterministic", () => {
    const buf = Buffer.from("test content");
    expect(blobHash(buf)).toBe(blobHash(buf));
  });

  it("blobStore + blobLoad roundtrip", async () => {
    const content = Buffer.from("hello world");
    const hash = blobHash(content);
    const objectsDir = join(storeDir, "objects");
    await blobStore(objectsDir, hash, content);
    expect(blobExists(objectsDir, hash)).toBe(true);
    const loaded = await blobLoad(objectsDir, hash);
    expect(Buffer.from(loaded)).toEqual(content);
  });

  it("blobExists returns false for missing hash", () => {
    const objectsDir = join(storeDir, "objects");
    expect(blobExists(objectsDir, "a".repeat(64))).toBe(false);
  });
});

describe("tree store", () => {
  let storeDir: string;

  beforeEach(() => {
    storeDir = mkdtempSync(join(tmpdir(), "chkpt-tree-"));
    mkdirSync(join(storeDir, "trees"), { recursive: true });
  });

  it("treeBuild + treeLoad roundtrip", async () => {
    const treesDir = join(storeDir, "trees");
    const entries = [
      {
        name: "hello.txt",
        entryType: "file",
        hash: "a".repeat(64),
        size: 11,
        mode: 0o100644,
      },
    ];
    const result = await treeBuild(treesDir, entries);
    expect(result.hash).toMatch(/^[0-9a-f]{64}$/);

    const loaded = await treeLoad(treesDir, result.hash);
    expect(loaded).toHaveLength(1);
    expect(loaded[0].name).toBe("hello.txt");
    expect(loaded[0].entryType).toBe("file");
  });
});

describe("snapshot store", () => {
  let storeDir: string;

  beforeEach(() => {
    storeDir = mkdtempSync(join(tmpdir(), "chkpt-snap-"));
    mkdirSync(join(storeDir, "snapshots"), { recursive: true });
  });

  it("snapshotSave + snapshotLoad roundtrip", async () => {
    const snapshotsDir = join(storeDir, "snapshots");
    const snap = {
      id: "test-snap-001",
      createdAt: new Date().toISOString(),
      message: "test snapshot",
      rootTreeHash: "b".repeat(64),
      parentSnapshotId: null,
      attachments: { depsKey: null, gitKey: null },
      stats: { totalFiles: 5, totalBytes: 1024, newObjects: 3 },
    };
    await snapshotSave(snapshotsDir, snap);
    const loaded = await snapshotLoad(snapshotsDir, "test-snap-001");
    expect(loaded.id).toBe("test-snap-001");
    expect(loaded.message).toBe("test snapshot");
    expect(loaded.stats.totalFiles).toBe(5);
  });

  it("snapshotList returns all snapshots sorted", async () => {
    const snapshotsDir = join(storeDir, "snapshots");
    await snapshotSave(snapshotsDir, {
      id: "snap-a",
      createdAt: "2026-01-01T00:00:00Z",
      message: "first",
      rootTreeHash: "a".repeat(64),
      parentSnapshotId: null,
      attachments: { depsKey: null, gitKey: null },
      stats: { totalFiles: 1, totalBytes: 10, newObjects: 1 },
    });
    await snapshotSave(snapshotsDir, {
      id: "snap-b",
      createdAt: "2026-02-01T00:00:00Z",
      message: "second",
      rootTreeHash: "b".repeat(64),
      parentSnapshotId: "snap-a",
      attachments: { depsKey: null, gitKey: null },
      stats: { totalFiles: 2, totalBytes: 20, newObjects: 1 },
    });
    const list = await snapshotList(snapshotsDir);
    expect(list).toHaveLength(2);
    // Newest first
    expect(list[0].id).toBe("snap-b");
    expect(list[1].id).toBe("snap-a");
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/store.spec.ts`
Expected: FAIL — functions not exported

**Step 3: Create store.rs bindings**

Create `crates/chkpt-napi/src/store.rs`:

```rust
use crate::error::to_napi_error;
use chkpt_core::store::blob::{hash_content, BlobStore};
use chkpt_core::store::snapshot::{Snapshot, SnapshotAttachments, SnapshotStats, SnapshotStore};
use chkpt_core::store::tree::{EntryType, TreeEntry, TreeStore};
use napi::bindgen_prelude::*;
use std::path::PathBuf;

// ── Blob ──

#[napi]
pub fn blob_hash(content: Buffer) -> String {
    hash_content(content.as_ref())
}

#[napi]
pub async fn blob_store(objects_dir: String, hash: String, content: Buffer) -> napi::Result<()> {
    let store = BlobStore::new(PathBuf::from(&objects_dir));
    // BlobStore::write hashes internally; we write raw content and it deduplicates.
    // But the design calls for explicit hash + content, so we store using the provided hash.
    // Since BlobStore::write computes hash internally, just call write with the content.
    store
        .write(content.as_ref())
        .map_err(to_napi_error)?;
    Ok(())
}

#[napi]
pub async fn blob_load(objects_dir: String, hash: String) -> napi::Result<Buffer> {
    let store = BlobStore::new(PathBuf::from(&objects_dir));
    let data = store.read(&hash).map_err(to_napi_error)?;
    Ok(Buffer::from(data))
}

#[napi]
pub fn blob_exists(objects_dir: String, hash: String) -> bool {
    let store = BlobStore::new(PathBuf::from(&objects_dir));
    store.exists(&hash)
}

// ── Tree ──

#[napi(object)]
pub struct JsTreeEntry {
    pub name: String,
    pub entry_type: String,
    pub hash: String,
    pub size: i64,
    pub mode: u32,
}

#[napi(object)]
pub struct JsTreeBuildResult {
    pub hash: String,
}

fn hex_to_bytes32(hex: &str) -> napi::Result<[u8; 32]> {
    let mut bytes = [0u8; 32];
    if hex.len() != 64 {
        return Err(napi::Error::new(
            napi::Status::InvalidArg,
            format!("Expected 64-char hex string, got {}", hex.len()),
        ));
    }
    for i in 0..32 {
        bytes[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| {
            napi::Error::new(napi::Status::InvalidArg, "Invalid hex character")
        })?;
    }
    Ok(bytes)
}

fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn parse_entry_type(s: &str) -> napi::Result<EntryType> {
    match s {
        "file" => Ok(EntryType::File),
        "directory" | "dir" => Ok(EntryType::Dir),
        "symlink" => Ok(EntryType::Symlink),
        _ => Err(napi::Error::new(
            napi::Status::InvalidArg,
            format!("Invalid entry type: {}", s),
        )),
    }
}

fn entry_type_to_string(et: &EntryType) -> String {
    match et {
        EntryType::File => "file".to_string(),
        EntryType::Dir => "directory".to_string(),
        EntryType::Symlink => "symlink".to_string(),
    }
}

#[napi]
pub async fn tree_build(
    trees_dir: String,
    entries: Vec<JsTreeEntry>,
) -> napi::Result<JsTreeBuildResult> {
    let store = TreeStore::new(PathBuf::from(&trees_dir));
    let tree_entries: Vec<TreeEntry> = entries
        .iter()
        .map(|e| {
            Ok(TreeEntry {
                name: e.name.clone(),
                entry_type: parse_entry_type(&e.entry_type)?,
                hash: hex_to_bytes32(&e.hash)?,
                size: e.size as u64,
                mode: e.mode,
            })
        })
        .collect::<napi::Result<Vec<_>>>()?;
    let hash = store.write(&tree_entries).map_err(to_napi_error)?;
    Ok(JsTreeBuildResult { hash })
}

#[napi]
pub async fn tree_load(trees_dir: String, hash: String) -> napi::Result<Vec<JsTreeEntry>> {
    let store = TreeStore::new(PathBuf::from(&trees_dir));
    let entries = store.read(&hash).map_err(to_napi_error)?;
    Ok(entries
        .iter()
        .map(|e| JsTreeEntry {
            name: e.name.clone(),
            entry_type: entry_type_to_string(&e.entry_type),
            hash: bytes32_to_hex(&e.hash),
            size: e.size as i64,
            mode: e.mode,
        })
        .collect())
}

// ── Snapshot ──

#[napi(object)]
pub struct JsSnapshotAttachments {
    pub deps_key: Option<String>,
    pub git_key: Option<String>,
}

#[napi(object)]
pub struct JsSnapshotStats {
    pub total_files: i64,
    pub total_bytes: i64,
    pub new_objects: i64,
}

#[napi(object)]
pub struct JsSnapshot {
    pub id: String,
    pub created_at: String,
    pub message: Option<String>,
    pub root_tree_hash: String,
    pub parent_snapshot_id: Option<String>,
    pub attachments: JsSnapshotAttachments,
    pub stats: JsSnapshotStats,
}

impl From<&Snapshot> for JsSnapshot {
    fn from(s: &Snapshot) -> Self {
        JsSnapshot {
            id: s.id.clone(),
            created_at: s.created_at.to_rfc3339(),
            message: s.message.clone(),
            root_tree_hash: bytes32_to_hex(&s.root_tree_hash),
            parent_snapshot_id: s.parent_snapshot_id.clone(),
            attachments: JsSnapshotAttachments {
                deps_key: s.attachments.deps_key.clone(),
                git_key: s.attachments.git_key.clone(),
            },
            stats: JsSnapshotStats {
                total_files: s.stats.total_files as i64,
                total_bytes: s.stats.total_bytes as i64,
                new_objects: s.stats.new_objects as i64,
            },
        }
    }
}

#[napi]
pub async fn snapshot_save(snapshots_dir: String, snapshot: JsSnapshot) -> napi::Result<()> {
    let store = SnapshotStore::new(PathBuf::from(&snapshots_dir));
    let root_tree_hash = hex_to_bytes32(&snapshot.root_tree_hash)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&snapshot.created_at)
        .map_err(|e| napi::Error::new(napi::Status::InvalidArg, e.to_string()))?
        .with_timezone(&chrono::Utc);
    let snap = Snapshot {
        id: snapshot.id,
        created_at,
        message: snapshot.message,
        root_tree_hash,
        parent_snapshot_id: snapshot.parent_snapshot_id,
        attachments: SnapshotAttachments {
            deps_key: snapshot.attachments.deps_key,
            git_key: snapshot.attachments.git_key,
        },
        stats: SnapshotStats {
            total_files: snapshot.stats.total_files as u64,
            total_bytes: snapshot.stats.total_bytes as u64,
            new_objects: snapshot.stats.new_objects as u64,
        },
    };
    store.save(&snap).map_err(to_napi_error)
}

#[napi]
pub async fn snapshot_load(snapshots_dir: String, snapshot_id: String) -> napi::Result<JsSnapshot> {
    let store = SnapshotStore::new(PathBuf::from(&snapshots_dir));
    let snap = store.load(&snapshot_id).map_err(to_napi_error)?;
    Ok(JsSnapshot::from(&snap))
}

#[napi]
pub async fn snapshot_list(snapshots_dir: String) -> napi::Result<Vec<JsSnapshot>> {
    let store = SnapshotStore::new(PathBuf::from(&snapshots_dir));
    let snaps = store.list(None).map_err(to_napi_error)?;
    Ok(snaps.iter().map(JsSnapshot::from).collect())
}
```

Note: Add `chrono` dependency to `crates/chkpt-napi/Cargo.toml`:

```toml
chrono = { workspace = true }
```

**Step 4: Register module in lib.rs**

Update `crates/chkpt-napi/src/lib.rs`:

```rust
#[macro_use]
extern crate napi_derive;

mod error;
mod config;
mod store;

#[napi]
pub fn ping() -> String {
    "pong".to_string()
}
```

**Step 5: Build and run tests**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/store.spec.ts`
Expected: All tests PASS

**Step 6: Commit**

```bash
git add crates/chkpt-napi/src/store.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/Cargo.toml crates/chkpt-napi/__test__/store.spec.ts Cargo.lock
git commit -m "feat(napi): add store bindings (blob, tree, snapshot)"
```

---

### Task 4: Implement scanner bindings

**Files:**

- Create: `crates/chkpt-napi/src/scanner.rs`
- Modify: `crates/chkpt-napi/src/lib.rs`

**Step 1: Write JS test for scanner**

Create `crates/chkpt-napi/__test__/scanner.spec.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { scanWorkspace } from "../index.js";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("scanner", () => {
  it("scans files in workspace", async () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-scan-"));
    writeFileSync(join(dir, "hello.txt"), "hello");
    mkdirSync(join(dir, "sub"));
    writeFileSync(join(dir, "sub", "world.txt"), "world");

    const files = await scanWorkspace(dir);
    expect(files).toHaveLength(2);

    const paths = files.map((f: any) => f.relativePath).sort();
    expect(paths).toEqual(["hello.txt", "sub/world.txt"]);

    const hello = files.find((f: any) => f.relativePath === "hello.txt");
    expect(hello.size).toBe(5);
    expect(hello.absolutePath).toContain("hello.txt");
    expect(hello.mode).toBeGreaterThan(0);
  });

  it("returns empty array for empty workspace", async () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-scan-"));
    const files = await scanWorkspace(dir);
    expect(files).toHaveLength(0);
  });

  it("excludes .git directory", async () => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-scan-"));
    mkdirSync(join(dir, ".git"));
    writeFileSync(join(dir, ".git", "config"), "x");
    writeFileSync(join(dir, "file.txt"), "y");

    const files = await scanWorkspace(dir);
    expect(files).toHaveLength(1);
    expect(files[0].relativePath).toBe("file.txt");
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/scanner.spec.ts`
Expected: FAIL — `scanWorkspace` not found

**Step 3: Create scanner.rs**

Create `crates/chkpt-napi/src/scanner.rs`:

```rust
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
pub async fn scan_workspace(workspace_path: String) -> napi::Result<Vec<JsScannedFile>> {
    let path = Path::new(&workspace_path);
    let files = scanner::scan_workspace(path, None).map_err(to_napi_error)?;
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
```

**Step 4: Register in lib.rs**

Update `crates/chkpt-napi/src/lib.rs` — add `mod scanner;`

**Step 5: Build and run tests**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/scanner.spec.ts`
Expected: All 3 tests PASS

**Step 6: Commit**

```bash
git add crates/chkpt-napi/src/scanner.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/__test__/scanner.spec.ts
git commit -m "feat(napi): add scanner binding (scanWorkspace)"
```

---

### Task 5: Implement index bindings (FileIndex)

**Files:**

- Create: `crates/chkpt-napi/src/index.rs`
- Modify: `crates/chkpt-napi/src/lib.rs`

**Step 1: Write JS test for index**

Create `crates/chkpt-napi/__test__/index.spec.ts`:

```typescript
import { describe, it, expect, beforeEach } from "vitest";
import {
  indexOpen,
  indexLookup,
  indexUpsert,
  indexAllEntries,
  indexClear,
} from "../index.js";
import { mkdtempSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("index", () => {
  let dbPath: string;

  beforeEach(() => {
    const dir = mkdtempSync(join(tmpdir(), "chkpt-idx-"));
    dbPath = join(dir, "index.sqlite");
  });

  it("upsert and lookup roundtrip", async () => {
    await indexOpen(dbPath);
    const entries = [
      {
        path: "hello.txt",
        blobHash: "a".repeat(64),
        size: 11,
        mtimeSecs: 1000,
        mtimeNanos: 0,
        inode: 12345,
        mode: 0o100644,
      },
    ];
    await indexUpsert(dbPath, entries);
    const result = await indexLookup(dbPath, "hello.txt");
    expect(result).not.toBeNull();
    expect(result!.path).toBe("hello.txt");
    expect(result!.blobHash).toBe("a".repeat(64));
    expect(result!.size).toBe(11);
  });

  it("lookup returns null for missing path", async () => {
    await indexOpen(dbPath);
    const result = await indexLookup(dbPath, "missing.txt");
    expect(result).toBeNull();
  });

  it("allEntries returns all entries", async () => {
    await indexOpen(dbPath);
    await indexUpsert(dbPath, [
      {
        path: "a.txt",
        blobHash: "a".repeat(64),
        size: 1,
        mtimeSecs: 0,
        mtimeNanos: 0,
        inode: null,
        mode: 0o100644,
      },
      {
        path: "b.txt",
        blobHash: "b".repeat(64),
        size: 2,
        mtimeSecs: 0,
        mtimeNanos: 0,
        inode: null,
        mode: 0o100644,
      },
    ]);
    const all = await indexAllEntries(dbPath);
    expect(all).toHaveLength(2);
  });

  it("clear removes all entries", async () => {
    await indexOpen(dbPath);
    await indexUpsert(dbPath, [
      {
        path: "x.txt",
        blobHash: "c".repeat(64),
        size: 3,
        mtimeSecs: 0,
        mtimeNanos: 0,
        inode: null,
        mode: 0o100644,
      },
    ]);
    await indexClear(dbPath);
    const all = await indexAllEntries(dbPath);
    expect(all).toHaveLength(0);
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/index.spec.ts`
Expected: FAIL

**Step 3: Create index.rs bindings**

Create `crates/chkpt-napi/src/index.rs`:

```rust
use crate::error::to_napi_error;
use crate::store::hex_to_bytes32;
use chkpt_core::index::{FileEntry, FileIndex};

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

fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn file_entry_to_js(entry: &FileEntry) -> JsFileEntry {
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

fn js_to_file_entry(entry: &JsFileEntry) -> napi::Result<FileEntry> {
    let blob_hash = hex_to_bytes32(&entry.blob_hash)?;
    Ok(FileEntry {
        path: entry.path.clone(),
        blob_hash,
        size: entry.size as u64,
        mtime_secs: entry.mtime_secs,
        mtime_nanos: entry.mtime_nanos,
        inode: entry.inode.map(|i| i as u64),
        mode: entry.mode,
    })
}

#[napi]
pub async fn index_open(db_path: String) -> napi::Result<()> {
    FileIndex::open(&db_path).map_err(to_napi_error)?;
    Ok(())
}

#[napi]
pub async fn index_lookup(db_path: String, path: String) -> napi::Result<Option<JsFileEntry>> {
    let index = FileIndex::open(&db_path).map_err(to_napi_error)?;
    let entry = index.get(&path).map_err(to_napi_error)?;
    Ok(entry.as_ref().map(file_entry_to_js))
}

#[napi]
pub async fn index_upsert(db_path: String, entries: Vec<JsFileEntry>) -> napi::Result<()> {
    let index = FileIndex::open(&db_path).map_err(to_napi_error)?;
    let file_entries: Vec<FileEntry> = entries
        .iter()
        .map(js_to_file_entry)
        .collect::<napi::Result<Vec<_>>>()?;
    index.bulk_upsert(&file_entries).map_err(to_napi_error)
}

#[napi]
pub async fn index_all_entries(db_path: String) -> napi::Result<Vec<JsFileEntry>> {
    let index = FileIndex::open(&db_path).map_err(to_napi_error)?;
    let entries = index.all_entries().map_err(to_napi_error)?;
    Ok(entries.iter().map(file_entry_to_js).collect())
}

#[napi]
pub async fn index_clear(db_path: String) -> napi::Result<()> {
    let index = FileIndex::open(&db_path).map_err(to_napi_error)?;
    index.clear().map_err(to_napi_error)
}
```

Note: `hex_to_bytes32` in `store.rs` must be `pub(crate)` so `index.rs` can use it. Update its visibility:

```rust
pub(crate) fn hex_to_bytes32(hex: &str) -> napi::Result<[u8; 32]> {
```

**Step 4: Register in lib.rs — add `mod index;`**

**Step 5: Build and run tests**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/index.spec.ts`
Expected: All 4 tests PASS

**Step 6: Commit**

```bash
git add crates/chkpt-napi/src/index.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/src/store.rs crates/chkpt-napi/__test__/index.spec.ts
git commit -m "feat(napi): add index bindings (open, lookup, upsert, clear)"
```

---

### Task 6: Implement high-level operations bindings (save, list, restore, delete)

**Files:**

- Create: `crates/chkpt-napi/src/ops.rs`
- Modify: `crates/chkpt-napi/src/lib.rs`

**Step 1: Write JS test for operations**

Create `crates/chkpt-napi/__test__/ops.spec.ts`:

```typescript
import { describe, it, expect, beforeEach } from "vitest";
import { save, list, restore, deleteSnapshot } from "../index.js";
import {
  mkdtempSync,
  writeFileSync,
  mkdirSync,
  readFileSync,
  existsSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("operations", () => {
  let workspace: string;

  beforeEach(() => {
    workspace = mkdtempSync(join(tmpdir(), "chkpt-ops-"));
    writeFileSync(join(workspace, "hello.txt"), "hello world");
    mkdirSync(join(workspace, "src"));
    writeFileSync(join(workspace, "src", "main.rs"), "fn main() {}");
  });

  it("save creates a checkpoint", async () => {
    const result = await save(workspace, "test save");
    expect(result.snapshotId).toBeTruthy();
    expect(result.totalFiles).toBe(2);
    expect(result.totalBytes).toBeGreaterThan(0);
    expect(result.newObjects).toBe(2);
  });

  it("list returns saved checkpoints", async () => {
    await save(workspace, "first");
    await save(workspace, "second");
    const snapshots = await list(workspace);
    expect(snapshots).toHaveLength(2);
    // Newest first
    expect(snapshots[0].message).toBe("second");
    expect(snapshots[1].message).toBe("first");
  });

  it("list with limit", async () => {
    await save(workspace, "a");
    await save(workspace, "b");
    await save(workspace, "c");
    const snapshots = await list(workspace, 2);
    expect(snapshots).toHaveLength(2);
  });

  it("restore with dry-run shows changes", async () => {
    const { snapshotId } = await save(workspace, "before");
    writeFileSync(join(workspace, "hello.txt"), "modified");
    writeFileSync(join(workspace, "new.txt"), "new file");

    const result = await restore(workspace, snapshotId, true);
    expect(result.filesChanged).toBe(1);
    expect(result.filesAdded).toBe(0); // new.txt not in snapshot
    expect(result.filesRemoved).toBe(1); // new.txt to be removed

    // Verify workspace unchanged (dry-run)
    expect(readFileSync(join(workspace, "hello.txt"), "utf-8")).toBe(
      "modified",
    );
  });

  it("restore actually restores files", async () => {
    const { snapshotId } = await save(workspace, "original");
    writeFileSync(join(workspace, "hello.txt"), "changed");
    writeFileSync(join(workspace, "extra.txt"), "extra");

    const result = await restore(workspace, snapshotId, false);
    expect(result.filesChanged).toBe(1);
    expect(result.filesRemoved).toBe(1);

    expect(readFileSync(join(workspace, "hello.txt"), "utf-8")).toBe(
      "hello world",
    );
    expect(existsSync(join(workspace, "extra.txt"))).toBe(false);
  });

  it('restore "latest" works', async () => {
    await save(workspace, "snap1");
    writeFileSync(join(workspace, "hello.txt"), "v2");
    await save(workspace, "snap2");
    writeFileSync(join(workspace, "hello.txt"), "v3");

    await restore(workspace, "latest", false);
    expect(readFileSync(join(workspace, "hello.txt"), "utf-8")).toBe("v2");
  });

  it("deleteSnapshot removes a checkpoint", async () => {
    const { snapshotId } = await save(workspace, "to-delete");
    let snapshots = await list(workspace);
    expect(snapshots).toHaveLength(1);

    await deleteSnapshot(workspace, snapshotId);
    snapshots = await list(workspace);
    expect(snapshots).toHaveLength(0);
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/ops.spec.ts`
Expected: FAIL

**Step 3: Create ops.rs bindings**

Create `crates/chkpt-napi/src/ops.rs`:

```rust
use crate::error::to_napi_error;
use crate::store::JsSnapshot;
use chkpt_core::ops;
use std::path::Path;

#[napi(object)]
pub struct JsSaveResult {
    pub snapshot_id: String,
    pub total_files: i64,
    pub new_objects: i64,
    pub total_bytes: i64,
}

#[napi(object)]
pub struct JsRestoreResult {
    pub snapshot_id: String,
    pub files_added: i64,
    pub files_changed: i64,
    pub files_removed: i64,
    pub files_unchanged: i64,
}

#[napi]
pub async fn save(workspace_path: String, message: Option<String>) -> napi::Result<JsSaveResult> {
    let path = Path::new(&workspace_path);
    let options = ops::save::SaveOptions { message };
    let result = ops::save::save(path, options).map_err(to_napi_error)?;
    Ok(JsSaveResult {
        snapshot_id: result.snapshot_id,
        total_files: result.stats.total_files as i64,
        new_objects: result.stats.new_objects as i64,
        total_bytes: result.stats.total_bytes as i64,
    })
}

#[napi]
pub async fn list(
    workspace_path: String,
    limit: Option<u32>,
) -> napi::Result<Vec<JsSnapshot>> {
    let path = Path::new(&workspace_path);
    let snapshots = ops::list::list(path, limit.map(|l| l as usize)).map_err(to_napi_error)?;
    Ok(snapshots.iter().map(JsSnapshot::from).collect())
}

#[napi]
pub async fn restore(
    workspace_path: String,
    snapshot_id: String,
    dry_run: Option<bool>,
) -> napi::Result<JsRestoreResult> {
    let path = Path::new(&workspace_path);
    let options = ops::restore::RestoreOptions {
        dry_run: dry_run.unwrap_or(false),
    };
    let result = ops::restore::restore(path, &snapshot_id, options).map_err(to_napi_error)?;
    Ok(JsRestoreResult {
        snapshot_id: result.snapshot_id,
        files_added: result.files_added as i64,
        files_changed: result.files_changed as i64,
        files_removed: result.files_removed as i64,
        files_unchanged: result.files_unchanged as i64,
    })
}

#[napi]
pub async fn delete_snapshot(
    workspace_path: String,
    snapshot_id: String,
) -> napi::Result<()> {
    let path = Path::new(&workspace_path);
    ops::delete::delete(path, &snapshot_id).map_err(to_napi_error)
}
```

**Step 4: Register in lib.rs — add `mod ops;`**

**Step 5: Build and run tests**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/ops.spec.ts`
Expected: All 7 tests PASS

**Step 6: Commit**

```bash
git add crates/chkpt-napi/src/ops.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/__test__/ops.spec.ts
git commit -m "feat(napi): add operations bindings (save, list, restore, delete)"
```

---

### Task 7: Implement attachment bindings (deps, git)

**Files:**

- Create: `crates/chkpt-napi/src/attachments.rs`
- Modify: `crates/chkpt-napi/src/lib.rs`

**Step 1: Write JS test for attachments**

Create `crates/chkpt-napi/__test__/attachments.spec.ts`:

```typescript
import { describe, it, expect, beforeEach } from "vitest";
import { depsArchive, depsRestore, computeDepsKey } from "../index.js";
import {
  mkdtempSync,
  writeFileSync,
  mkdirSync,
  readFileSync,
  existsSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("deps attachment", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "chkpt-deps-"));
  });

  it("computeDepsKey returns 16-char hex", () => {
    const lockfile = join(tmpDir, "package-lock.json");
    writeFileSync(lockfile, '{"lockfileVersion":3}');
    const key = computeDepsKey(lockfile);
    expect(key).toMatch(/^[0-9a-f]{16}$/);
  });

  it("computeDepsKey is deterministic", () => {
    const lockfile = join(tmpDir, "package-lock.json");
    writeFileSync(lockfile, '{"lockfileVersion":3}');
    expect(computeDepsKey(lockfile)).toBe(computeDepsKey(lockfile));
  });

  it("depsArchive + depsRestore roundtrip", async () => {
    // Create fake node_modules
    const depsDir = join(tmpDir, "node_modules");
    mkdirSync(join(depsDir, "lodash"), { recursive: true });
    writeFileSync(join(depsDir, "lodash", "index.js"), "module.exports = {}");

    const lockfile = join(tmpDir, "package-lock.json");
    writeFileSync(lockfile, '{"lockfileVersion":3}');

    const archiveDir = join(tmpDir, "archive");
    mkdirSync(archiveDir);

    const key = computeDepsKey(lockfile);
    await depsArchive(depsDir, archiveDir, key);
    expect(existsSync(join(archiveDir, `${key}.tar.zst`))).toBe(true);

    // Restore to new location
    const restoreDir = join(tmpDir, "restored_modules");
    await depsRestore(restoreDir, archiveDir, key);
    expect(readFileSync(join(restoreDir, "lodash", "index.js"), "utf-8")).toBe(
      "module.exports = {}",
    );
  });
});
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/attachments.spec.ts`
Expected: FAIL

**Step 3: Create attachments.rs**

Create `crates/chkpt-napi/src/attachments.rs`:

```rust
use crate::error::to_napi_error;
use chkpt_core::attachments::{deps, git};
use std::path::Path;

// ── Deps ──

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
    deps::archive_deps(
        Path::new(&deps_dir),
        Path::new(&archive_dir),
        &deps_key,
    )
    .map_err(to_napi_error)
}

#[napi]
pub async fn deps_restore(
    deps_dir: String,
    archive_dir: String,
    deps_key: String,
) -> napi::Result<()> {
    deps::restore_deps(
        Path::new(&deps_dir),
        Path::new(&archive_dir),
        &deps_key,
    )
    .map_err(to_napi_error)
}

// ── Git ──

#[napi]
pub async fn git_bundle_create(
    repo_path: String,
    archive_dir: String,
) -> napi::Result<String> {
    git::create_git_bundle(
        Path::new(&repo_path),
        Path::new(&archive_dir),
    )
    .map_err(to_napi_error)
}

#[napi]
pub async fn git_bundle_restore(
    repo_path: String,
    archive_dir: String,
    git_key: String,
) -> napi::Result<()> {
    git::restore_git_bundle(
        Path::new(&repo_path),
        Path::new(&archive_dir),
        &git_key,
    )
    .map_err(to_napi_error)
}
```

**Step 4: Register in lib.rs — add `mod attachments;`**

**Step 5: Build and run tests**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/attachments.spec.ts`
Expected: All 3 tests PASS

**Step 6: Commit**

```bash
git add crates/chkpt-napi/src/attachments.rs crates/chkpt-napi/src/lib.rs crates/chkpt-napi/__test__/attachments.spec.ts
git commit -m "feat(napi): add attachment bindings (deps, git)"
```

---

### Task 8: Create CLI launcher (cli.mjs)

**Files:**

- Create: `crates/chkpt-napi/cli.mjs`

**Step 1: Write JS test for CLI launcher**

Create `crates/chkpt-napi/__test__/cli.spec.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { execSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("CLI via Rust binary", () => {
  it("chkpt-cli binary runs and shows help", () => {
    // Verify the Rust CLI binary exists and runs
    const output = execSync("cargo run -p chkpt-cli -- --help", {
      cwd: "/Users/classting/Workspace/temp/chkpt",
      encoding: "utf-8",
    });
    expect(output).toContain("Filesystem checkpoint engine");
  });

  it("chkpt save + list via Rust CLI", () => {
    const workspace = mkdtempSync(join(tmpdir(), "chkpt-cli-"));
    writeFileSync(join(workspace, "test.txt"), "cli test");

    const saveOutput = execSync(
      `cargo run -p chkpt-cli -- save -m "cli test"`,
      {
        cwd: workspace,
        encoding: "utf-8",
        env: {
          ...process.env,
          CARGO_MANIFEST_DIR: "/Users/classting/Workspace/temp/chkpt",
        },
      },
    );
    expect(saveOutput).toContain("Checkpoint saved");

    const listOutput = execSync(`cargo run -p chkpt-cli -- list`, {
      cwd: workspace,
      encoding: "utf-8",
      env: {
        ...process.env,
        CARGO_MANIFEST_DIR: "/Users/classting/Workspace/temp/chkpt",
      },
    });
    expect(listOutput).toContain("cli test");
  });
});
```

**Step 2: Run test to verify it passes (Rust CLI already exists)**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run __test__/cli.spec.ts`
Expected: PASS (relies on existing `chkpt-cli` crate)

**Step 3: Create cli.mjs**

Create `crates/chkpt-napi/cli.mjs`:

```js
#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";

function getBinaryName() {
  return process.platform === "win32" ? "chkpt.exe" : "chkpt";
}

function getBinaryPath() {
  const platform = process.platform;
  const arch = process.arch;
  const binaryName = getBinaryName();

  // Platform-specific package mapping
  const triples = {
    "darwin-arm64": "darwin-arm64",
    "darwin-x64": "darwin-x64",
    "linux-arm64": "linux-arm64-gnu",
    "linux-x64": "linux-x64-gnu",
    "win32-x64": "win32-x64-msvc",
  };

  const key = `${platform}-${arch}`;
  const triple = triples[key];

  if (!triple) {
    throw new Error(
      `Unsupported platform: ${platform}-${arch}. ` +
        `Supported: ${Object.keys(triples).join(", ")}`,
    );
  }

  // Try to find the binary in the platform-specific npm package
  const packageName = `@chkpt/cli-${triple}`;
  try {
    const pkgDir = dirname(
      fileURLToPath(import.meta.resolve(`${packageName}/package.json`)),
    );
    const binaryPath = join(pkgDir, binaryName);
    if (existsSync(binaryPath)) {
      return binaryPath;
    }
  } catch {
    // Package not installed, fall through
  }

  // Fallback: try to find binary next to this script (local dev)
  const localBinary = join(dirname(fileURLToPath(import.meta.url)), binaryName);
  if (existsSync(localBinary)) {
    return localBinary;
  }

  throw new Error(
    `Could not find chkpt binary. Install the platform package: npm install ${packageName}`,
  );
}

try {
  const binary = getBinaryPath();
  const result = execFileSync(binary, process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });
} catch (error) {
  if (error.status != null) {
    process.exit(error.status);
  }
  console.error(error.message);
  process.exit(1);
}
```

**Step 4: Commit**

```bash
git add crates/chkpt-napi/cli.mjs crates/chkpt-napi/__test__/cli.spec.ts
git commit -m "feat(napi): add CLI launcher (cli.mjs) for Rust binary"
```

---

### Task 9: Generate platform npm packages scaffold

**Files:**

- Create: `crates/chkpt-napi/npm/darwin-arm64/package.json`
- Create: `crates/chkpt-napi/npm/darwin-x64/package.json`
- Create: `crates/chkpt-napi/npm/linux-arm64-gnu/package.json`
- Create: `crates/chkpt-napi/npm/linux-x64-gnu/package.json`
- Create: `crates/chkpt-napi/npm/linux-arm64-musl/package.json`
- Create: `crates/chkpt-napi/npm/linux-x64-musl/package.json`
- Create: `crates/chkpt-napi/npm/win32-x64-msvc/package.json`
- Modify: `crates/chkpt-napi/package.json` (add optionalDependencies)

**Step 1: Generate platform packages with napi-rs**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx napi create-npm-dirs`
Expected: Creates `npm/` directory with platform package.json files

If `napi create-npm-dirs` doesn't exist in your version, create them manually.

For each platform, the package.json follows this template (example for darwin-arm64):

```json
{
  "name": "@chkpt/cli-darwin-arm64",
  "version": "0.1.0",
  "os": ["darwin"],
  "cpu": ["arm64"],
  "main": "chkpt.darwin-arm64.node",
  "files": ["chkpt.darwin-arm64.node", "chkpt"],
  "description": "chkpt native bindings for darwin-arm64",
  "license": "MIT",
  "engines": {
    "node": ">= 18"
  }
}
```

**Step 2: Add optionalDependencies to main package.json**

Update `crates/chkpt-napi/package.json` to add:

```json
{
  "optionalDependencies": {
    "@chkpt/cli-darwin-arm64": "0.1.0",
    "@chkpt/cli-darwin-x64": "0.1.0",
    "@chkpt/cli-linux-arm64-gnu": "0.1.0",
    "@chkpt/cli-linux-x64-gnu": "0.1.0",
    "@chkpt/cli-linux-arm64-musl": "0.1.0",
    "@chkpt/cli-linux-x64-musl": "0.1.0",
    "@chkpt/cli-win32-x64-msvc": "0.1.0"
  }
}
```

**Step 3: Commit**

```bash
git add crates/chkpt-napi/npm/ crates/chkpt-napi/package.json
git commit -m "feat(napi): add platform-specific npm package scaffolds"
```

---

### Task 10: Add vitest config and run full test suite

**Files:**

- Create: `crates/chkpt-napi/vitest.config.ts`

**Step 1: Create vitest config**

Create `crates/chkpt-napi/vitest.config.ts`:

```typescript
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    testTimeout: 30000,
    include: ["__test__/**/*.spec.ts"],
  },
});
```

**Step 2: Build the native module**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build`
Expected: Build succeeds, generates `.node` file

**Step 3: Run full test suite**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run`
Expected: All tests across all spec files PASS

**Step 4: Run existing Rust tests to verify no regressions**

Run: `cd /Users/classting/Workspace/temp/chkpt && cargo test`
Expected: All existing tests PASS

**Step 5: Add .gitignore for generated files**

Create `crates/chkpt-napi/.gitignore`:

```
*.node
target/
node_modules/
index.js
index.d.ts
```

**Step 6: Commit**

```bash
git add crates/chkpt-napi/vitest.config.ts crates/chkpt-napi/.gitignore
git commit -m "feat(napi): add vitest config and verify full test suite"
```

---

### Task 11: End-to-end integration test

**Files:**

- Create: `crates/chkpt-napi/__test__/e2e.spec.ts`

**Step 1: Write e2e test**

Create `crates/chkpt-napi/__test__/e2e.spec.ts`:

```typescript
import { describe, it, expect } from "vitest";
import {
  save,
  list,
  restore,
  deleteSnapshot,
  getProjectId,
  scanWorkspace,
  blobHash,
} from "../index.js";
import {
  mkdtempSync,
  writeFileSync,
  readFileSync,
  existsSync,
  mkdirSync,
} from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

describe("end-to-end", () => {
  it("full lifecycle: save → modify → save → list → restore → delete", async () => {
    const workspace = mkdtempSync(join(tmpdir(), "chkpt-e2e-"));

    // Create initial files
    writeFileSync(join(workspace, "README.md"), "# Hello");
    mkdirSync(join(workspace, "src"));
    writeFileSync(join(workspace, "src", "index.ts"), 'console.log("v1")');

    // Verify project ID is deterministic
    const id1 = getProjectId(workspace);
    const id2 = getProjectId(workspace);
    expect(id1).toBe(id2);

    // Save checkpoint 1
    const save1 = await save(workspace, "initial version");
    expect(save1.snapshotId).toBeTruthy();
    expect(save1.totalFiles).toBe(2);

    // Modify files
    writeFileSync(join(workspace, "src", "index.ts"), 'console.log("v2")');
    writeFileSync(join(workspace, "src", "utils.ts"), "export const x = 1");

    // Save checkpoint 2
    const save2 = await save(workspace, "added utils");
    expect(save2.totalFiles).toBe(3);

    // List should show 2 snapshots (newest first)
    const snapshots = await list(workspace);
    expect(snapshots).toHaveLength(2);
    expect(snapshots[0].message).toBe("added utils");
    expect(snapshots[1].message).toBe("initial version");

    // Dry-run restore to checkpoint 1
    const dryRun = await restore(workspace, save1.snapshotId, true);
    expect(dryRun.filesChanged).toBe(1); // index.ts changed
    expect(dryRun.filesRemoved).toBe(1); // utils.ts to be removed
    // Verify no actual changes
    expect(readFileSync(join(workspace, "src", "index.ts"), "utf-8")).toBe(
      'console.log("v2")',
    );

    // Actual restore to checkpoint 1
    const restoreResult = await restore(workspace, save1.snapshotId, false);
    expect(restoreResult.filesChanged).toBe(1);
    expect(restoreResult.filesRemoved).toBe(1);
    expect(readFileSync(join(workspace, "src", "index.ts"), "utf-8")).toBe(
      'console.log("v1")',
    );
    expect(existsSync(join(workspace, "src", "utils.ts"))).toBe(false);

    // Delete checkpoint 2
    await deleteSnapshot(workspace, save2.snapshotId);
    const remaining = await list(workspace);
    expect(remaining).toHaveLength(1);
    expect(remaining[0].id).toBe(save1.snapshotId);

    // Scanner and blobHash work as low-level APIs
    const files = await scanWorkspace(workspace);
    expect(files).toHaveLength(2);
    const hash = blobHash(Buffer.from("# Hello"));
    expect(hash).toMatch(/^[0-9a-f]{64}$/);
  });
});
```

**Step 2: Build and run e2e test**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run __test__/e2e.spec.ts`
Expected: PASS

**Step 3: Run all tests one final time**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npx vitest run`
Expected: All tests PASS across all spec files

**Step 4: Commit**

```bash
git add crates/chkpt-napi/__test__/e2e.spec.ts
git commit -m "test(napi): add end-to-end integration test for JS SDK"
```

---

### Task 12: Clean up and remove ping, final verification

**Files:**

- Modify: `crates/chkpt-napi/src/lib.rs` (remove ping)

**Step 1: Remove the ping function from lib.rs**

Final `crates/chkpt-napi/src/lib.rs`:

```rust
#[macro_use]
extern crate napi_derive;

mod error;
mod config;
mod store;
mod scanner;
mod index;
mod ops;
mod attachments;
```

**Step 2: Build and verify**

Run: `cd /Users/classting/Workspace/temp/chkpt/crates/chkpt-napi && npm run build && npx vitest run`
Expected: All tests PASS

**Step 3: Run full Rust test suite**

Run: `cd /Users/classting/Workspace/temp/chkpt && cargo test`
Expected: All tests PASS (including chkpt-core, chkpt-cli)

**Step 4: Commit**

```bash
git add crates/chkpt-napi/src/lib.rs
git commit -m "refactor(napi): remove scaffold ping, finalize module exports"
```
