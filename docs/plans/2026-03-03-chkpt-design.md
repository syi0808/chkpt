# chkpt v1 Design Document

## Overview

Filesystem checkpoint engine that lets humans and AI agents save, restore, and delete workspace state without polluting Git. Provides CLI, MCP server, and Rust SDK.

## Decisions

| Item               | Decision                                                         |
| ------------------ | ---------------------------------------------------------------- |
| MVP scope          | Core + CLI + MCP + Attachments                                   |
| Index format       | SQLite (rusqlite, bundled)                                       |
| Restore mode       | hard only (merge deferred to v1.1)                               |
| .git layer         | git bundle                                                       |
| MCP protocol       | stdio                                                            |
| Project ID         | BLAKE3 hash of canonicalized workspace path (first 16 hex chars) |
| Ignore strategy    | Independent .chkptignore only (no .gitignore parsing)            |
| Async runtime      | tokio                                                            |
| Tree serialization | bincode                                                          |
| Object paths       | 2-char prefix directories (objects/ab/cdef...)                   |
| Packfile           | Included in v1                                                   |

---

## 1. Crate Structure

```
chkpt/                          (workspace root)
├── Cargo.toml                 (workspace manifest)
├── crates/
│   ├── chkpt-core/             (library crate)
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── config.rs      — project settings, guardrails, project_id
│   │   │   ├── scanner/       — filesystem walking + .chkptignore matching
│   │   │   │   ├── mod.rs
│   │   │   │   ├── walker.rs  — async directory traversal (ignore crate)
│   │   │   │   └── matcher.rs — .chkptignore parser/matcher
│   │   │   ├── store/         — content-addressed storage
│   │   │   │   ├── mod.rs
│   │   │   │   ├── blob.rs    — BLAKE3 hashing + zstd compress/decompress
│   │   │   │   ├── tree.rs    — Tree bincode ser/de
│   │   │   │   ├── snapshot.rs — Snapshot meta CRUD
│   │   │   │   └── pack.rs   — Packfile creation, idx lookup, gc repack
│   │   │   ├── index/         — SQLite change detection cache
│   │   │   │   ├── mod.rs
│   │   │   │   └── schema.rs  — table definitions, migrations
│   │   │   ├── ops/           — high-level operations
│   │   │   │   ├── mod.rs
│   │   │   │   ├── save.rs
│   │   │   │   ├── restore.rs — atomic restore + dry-run
│   │   │   │   ├── delete.rs  — delete + GC
│   │   │   │   ├── list.rs
│   │   │   │   └── lock.rs    — file-based project lock
│   │   │   └── attachments/   — optional layers
│   │   │       ├── mod.rs
│   │   │       ├── deps.rs    — node_modules tar.zst archive
│   │   │       └── git.rs     — git bundle create/restore
│   │   ├── tests/             — integration tests
│   │   └── benches/           — benchmarks
│   ├── chkpt-cli/              (binary crate)
│   │   └── src/main.rs        — clap-based CLI
│   └── chkpt-mcp/              (binary crate)
│       └── src/main.rs        — MCP stdio server
```

### Dependencies

| Crate              | Purpose                                            |
| ------------------ | -------------------------------------------------- |
| blake3             | Content hashing                                    |
| zstd               | Blob compression                                   |
| rusqlite (bundled) | Index storage                                      |
| ignore             | .chkptignore parsing (gitignore-compatible syntax) |
| tokio              | Async runtime                                      |
| clap               | CLI parsing                                        |
| serde + serde_json | Config/snapshot serialization                      |
| bincode            | Tree serialization                                 |
| chrono             | Timestamps                                         |
| uuid               | Snapshot ID generation (v7)                        |
| tar + zstd         | Deps archive                                       |
| rmcp or custom     | MCP stdio protocol                                 |
| fs4                | File locking                                       |
| tempfile           | Atomic restore temp directories                    |

---

## 2. Store Layout

```
~/.chkpt/stores/<project_id>/
├── config.json          — { project_root, created_at }
├── snapshots/
│   └── <snapshot_id>.json
├── trees/
│   └── <prefix>/<rest>  — bincode-serialized tree entries
├── objects/
│   └── <prefix>/<rest>  — zstd-compressed file content
├── packs/
│   ├── pack-<hash>.dat  — concatenated compressed entries
│   └── pack-<hash>.idx  — sorted hash→offset index
├── index.sqlite         — change detection cache
├── locks/
│   └── project.lock     — mutual exclusion lock
└── attachments/
    ├── deps/
    │   └── <deps_key>.tar.zst
    └── git/
        └── <git_key>.bundle
```

`project_id` = first 16 hex chars of `blake3(canonicalize(workspace_path))`

---

## 3. Object Model

### Blob

- Key: `blake3(content)` hex
- Storage: `objects/<first 2 chars>/<remaining chars>` containing `zstd(content)`

### Tree

```rust
#[derive(Serialize, Deserialize)]
struct TreeEntry {
    name: String,
    entry_type: EntryType,  // File | Dir | Symlink
    hash: [u8; 32],         // blob_hash (File) or tree_hash (Dir)
    size: u64,              // original size (File only)
    mode: u32,              // Unix permissions
}

enum EntryType { File, Dir, Symlink }
```

- Key: `blake3(bincode::serialize(sorted_entries))` hex
- Storage: `trees/<first 2 chars>/<remaining chars>`
- Entries sorted by `name` (lexicographic)
- Unchanged subdirectory trees reuse existing hash

### Snapshot

```rust
struct Snapshot {
    id: String,                       // UUID v7
    created_at: DateTime<Utc>,
    message: Option<String>,
    root_tree_hash: [u8; 32],
    parent_snapshot_id: Option<String>,
    attachments: SnapshotAttachments,
    stats: SnapshotStats,
}

struct SnapshotAttachments {
    deps_key: Option<String>,
    git_key: Option<String>,
}

struct SnapshotStats {
    total_files: u64,
    total_bytes: u64,
    new_objects: u64,
}
```

---

## 4. Index (Change Detection Cache)

### SQLite Schema

```sql
CREATE TABLE file_index (
    path        TEXT PRIMARY KEY,
    blob_hash   BLOB NOT NULL,      -- BLAKE3 (32 bytes)
    size        INTEGER NOT NULL,
    mtime_secs  INTEGER NOT NULL,
    mtime_nanos INTEGER NOT NULL,
    inode       INTEGER,
    mode        INTEGER NOT NULL
);

CREATE TABLE metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

### Change Detection Flow

1. Walk workspace files (scanner)
2. For each file:
   - Lookup path in index
   - Compare (size, mtime, inode)
   - Mismatch → recompute BLAKE3 hash
   - Hash differs → changed (store new blob)
   - Hash same → update mtime only (false positive)
   - Not in index → new file
3. Paths in index but not in workspace → deleted files

### Optimizations

- Skip hash when mtime + size match (same strategy as Git)
- Bulk INSERT/UPDATE within SQLite transactions
- WAL mode for concurrent read/write improvement

---

## 5. Packfile

### Format

```
pack-<hash>.dat:
┌──────────────────────────────────────────────┐
│ Header: magic(4) + version(4) + entry_count(4) │
├──────────────────────────────────────────────┤
│ Entry: hash(32) + compressed_size(8) + data    │
│ ...                                            │
└──────────────────────────────────────────────┘

pack-<hash>.idx:
┌──────────────────────────────────────────────┐
│ Sorted entries: hash(32) + offset(8) + size(8) │
└──────────────────────────────────────────────┘
```

### Object Lookup Order

1. Check loose objects (`objects/<prefix>/<rest>`)
2. Binary search through pack index files
3. Read from pack .dat at offset

### Packing Strategy

- After save: if loose objects > threshold (1000), auto-pack
- Pack: all loose objects → new pack → delete loose files
- After delete/GC: repack excluding unreferenced entries
- No delta compression in v1 (each entry independently zstd-compressed)
- Both blobs and trees stored in same packs

---

## 6. Operations

### Lock

- File-based advisory lock (`locks/project.lock`) via fs4 crate
- save, restore, delete are mutually exclusive
- Lock acquisition failure → immediate error (no wait)

### Save

1. Acquire exclusive lock
2. Validate guardrails (max_total_bytes, max_files, max_file_size)
3. Scanner: walk workspace + apply .chkptignore
4. Compare with index: identify changed/new/deleted files
5. For each changed file: compute BLAKE3, store as loose blob if new
6. Build trees bottom-up (reuse unchanged subtree hashes)
7. Create snapshot: root_tree_hash + metadata → `snapshots/<uuid>.json`
8. Update index to reflect current state
9. Process attachments (optional):
   - `--with-deps`: compute deps_key, create node_modules tar.zst if changed
   - `--with-git`: run `git bundle create`
10. Check loose object count → auto-pack if threshold exceeded
11. Release lock

### Restore (hard)

**dry-run:**

1. Acquire lock
2. Load snapshot → extract full file list from root tree
3. Compare with current workspace
4. Return change summary: `{ added[], modified[], deleted[], stats }`
5. Release lock

**actual restore:**

1. Acquire exclusive lock
2. Load snapshot
3. Create temp directory (same filesystem as workspace)
4. Phase 1 — Prepare:
   - Walk snapshot tree
   - Compare with current workspace
   - Read blobs from store, reconstruct in temp directory
5. Phase 2 — Commit (atomic):
   - Remove files/directories not in snapshot
   - Rename/move from temp → workspace
   - On failure → clean up temp, workspace remains intact
6. Reset index to snapshot state
7. Restore attachments (optional):
   - deps: extract tar.zst → node_modules
   - git: `git bundle unbundle`
8. Release lock

### Delete + GC

1. Acquire lock
2. Delete `snapshots/<id>.json`
3. Mark & Sweep GC:
   - Collect all reachable hashes from remaining snapshots' root trees
   - Delete unreachable loose objects
   - Repack if pack files contain unreferenced entries
   - Delete unreferenced attachments
4. Release lock

### List

1. Load all JSON files from `snapshots/`
2. Sort by `created_at` (newest first)
3. Apply limit
4. Return `{ id, created_at, message, stats }` list

---

## 7. CLI Interface

```
chkpt save [-m <message>] [--with-deps] [--with-git]
chkpt list [--limit <n>]
chkpt restore <id|latest> [--dry-run]
chkpt delete <id>
chkpt init  (optional: explicit store init + .chkptignore creation)
```

---

## 8. MCP Interface (stdio)

### Tools

**checkpoint_save**

- Input: `{ message?, with_deps?, with_git? }`
- Output: `{ id, stats: { files, bytes, new_objects } }`

**checkpoint_list**

- Input: `{ limit? }`
- Output: `{ items: [{ id, created_at, message, stats }] }`

**checkpoint_restore**

- Input: `{ id, dry_run? }`
- Output (dry_run): `{ ok, summary: { added, modified, deleted }, changed_paths[] }`
- Output (actual): `{ ok, summary }`

**checkpoint_delete**

- Input: `{ id }`
- Output: `{ ok, freed_bytes }`

### Error Format

```json
{ "ok": false, "error": "ERROR_CODE", "message": "Human-readable description" }
```

Error codes: `LOCK_HELD`, `SNAPSHOT_NOT_FOUND`, `GUARDRAIL_EXCEEDED`, `STORE_CORRUPTED`, `IO_ERROR`

---

## 9. Test Strategy

### Unit Tests (chkpt-core)

| Module          | Coverage                                                           |
| --------------- | ------------------------------------------------------------------ |
| store::blob     | BLAKE3 accuracy, zstd round-trip                                   |
| store::tree     | bincode ser/de round-trip, tree hash determinism                   |
| store::snapshot | JSON serialization, UUID generation                                |
| store::pack     | pack creation, idx binary search, loose→pack transition            |
| index           | SQLite CRUD, mtime-based change detection, false positive handling |
| scanner         | .chkptignore matching, symlink handling, empty directories         |
| ops::lock       | lock acquire/release, double-lock prevention                       |

### Integration Tests (crates/chkpt-core/tests/)

- save → list → restore full flow
- save → modify files → save → restore previous (incremental)
- save → delete → GC (unreferenced object cleanup)
- dry-run does not modify workspace
- atomic restore failure preserves workspace integrity
- guardrail exceeded → error
- large file count (1000+) scenarios
- attachments: deps archive round-trip
- attachments: git bundle round-trip
- pack auto-creation + correct object lookup

### Benchmarks (crates/chkpt-core/benches/)

- save: 5K/10K/20K file workspaces
- restore: varying changed file counts (10/100/1000)
- index change detection: skip efficiency when mostly unchanged
- pack lookup vs loose lookup comparison
