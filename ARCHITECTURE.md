# chkpt Architecture

> Current architecture for the code that ships today.

## Overview

chkpt is a local filesystem checkpoint system. All interfaces call the same core library:

- `chkpt-cli` for terminal usage
- `chkpt-mcp` for MCP tool access
- `chkpt-napi` for Node.js bindings
- `chkpt-plugin` for the Claude Code skill and plugin surface

The runtime storage model is:

- `catalog.sqlite` for snapshot metadata, manifests, and blob locations
- `packs/` for packed blobs plus `.idx` indexes
- `trees/` for tree nodes used to reconstruct directory structure
- `index.bin` for current-workspace file metadata cache
- `locks/project.lock` for process-level exclusion

`snapshots/` and `attachments/` directories may still exist in the store layout, but the current save/restore/delete/list path does not use snapshot JSON files or attachment archives.

## Monorepo Layout

```text
crates/
├── chkpt-core/      core logic
├── chkpt-cli/       CLI
├── chkpt-mcp/       MCP server
├── chkpt-napi/      Node.js bindings
├── chkpt-mcp-npm/   npm wrapper for the MCP server
└── chkpt-plugin/    plugin + skill content
```

## Core Modules

`chkpt-core/src/` is organized into:

- `config.rs`
  - `project_id_from_path()`
  - `StoreLayout`
- `error.rs`
  - shared error type
- `scanner/`
  - workspace walking and ignore matching
- `store/blob.rs`
  - BLAKE3 hashing and path/content helpers
- `store/tree.rs`
  - content-addressed tree nodes
- `store/pack.rs`
  - packed object IO and indexes
- `store/catalog.rs`
  - SQLite-backed snapshot metadata and manifest catalog
- `store/snapshot.rs`
  - public `Snapshot` and `SnapshotStats` data types
- `index/`
  - binary metadata cache for warm save/restore scans
- `ops/save.rs`
  - save pipeline
- `ops/restore.rs`
  - restore and dry-run pipeline
- `ops/delete.rs`
  - snapshot deletion and garbage collection
- `ops/list.rs`
  - snapshot listing
- `ops/lock.rs`
  - RAII lock guard around `project.lock`

## Storage Model

### Catalog

`catalog.sqlite` is the metadata source of truth.

Relevant tables:

- `snapshots`
  - `id`
  - `created_at`
  - `message`
  - `parent_snapshot_id`
  - `manifest_snapshot_id`
  - `root_tree_hash`
  - `total_files`
  - `total_bytes`
  - `new_objects`
- `snapshot_files`
  - flattened manifest rows: `snapshot_id`, `path`, `blob_hash`, `size`, `mode`
- `blob_index`
  - blob location metadata: `blob_hash`, `pack_hash`, `size`

`manifest_snapshot_id` lets metadata-only snapshots reuse another snapshot's manifest when nothing changed.

### Pack Storage

When new blobs are produced during save, they can be written into pack files:

```text
packs/pack-<hash>.dat
packs/pack-<hash>.idx
```

`blob_index.pack_hash` tells restore and delete where a blob lives.

### Tree Storage

Tree nodes still exist because restore and delete sometimes need to traverse a saved tree from `root_tree_hash`.

Each `TreeEntry` stores:

- `name`
- `entry_type` (`File`, `Dir`, `Symlink`)
- `hash`
- `size`
- `mode`

Trees are content-addressed and stored under `trees/`.

### Workspace Index

`index.bin` caches file metadata for the current workspace:

- relative path
- blob hash
- size
- mtime
- inode/device where available
- mode

This keeps warm saves incremental and lets restore avoid unnecessary re-hashing when metadata still matches.

## Save Flow

`save()` does the following:

1. Compute the project id from the workspace path.
2. Build the store layout and acquire `project.lock`.
3. Scan the workspace, excluding dependency directories unless `include_deps` is set.
4. Compare scanned files against `index.bin`.
5. Reuse cached hashes for unchanged files.
6. Hash and compress changed files.
7. Write new blobs and record blob locations in the catalog.
8. Build the root tree when needed, or reuse the previous snapshot's tree hash on no-op saves.
9. Insert snapshot metadata into `catalog.sqlite`.
10. Insert the manifest into `snapshot_files`, or point `manifest_snapshot_id` at the reused manifest owner when nothing changed.
11. Apply incremental index updates.

### Save Outputs

The public snapshot shape still includes:

- `id`
- `created_at`
- `message`
- `root_tree_hash`
- `parent_snapshot_id`
- `stats`

That shape is now backed by catalog rows, not snapshot JSON files.

## Restore Flow

`restore()` is catalog-first:

1. Acquire `project.lock`.
2. Resolve the snapshot from the catalog by exact id, prefix, or `latest`.
3. Load the target manifest from `snapshot_files`.
4. If the manifest is unavailable but `root_tree_hash` exists, reconstruct target state by traversing tree nodes.
5. Scan the current workspace state.
6. Compute add/change/remove/unchanged sets.
7. If `dry_run`, return stats only.
8. Otherwise restore content from packs using `blob_index`.
9. Remove stale files and clean empty directories.
10. Apply incremental index updates for changed paths only.

## Delete Flow

`delete()`:

1. acquires `project.lock`
2. removes the snapshot row from the catalog
3. re-computes reachable blobs from remaining manifests
4. falls back to `root_tree_hash` traversal when needed
5. deletes unreferenced blob rows
6. removes pack files whose `pack_hash` is no longer referenced

The current delete path does not delete snapshot JSON files because they are no longer part of the live storage model.

## Scanner Behavior

Built-in exclusions include:

- `.git`
- `.chkpt`
- `target`
- `node_modules`
- `.venv`
- `venv`
- `__pypackages__`
- `.tox`
- `.nox`
- `.gradle`
- `.m2`

`include_deps` disables exclusion for the dependency-directory set, not for `.git`.

## Store Layout Helper

`StoreLayout` currently exposes:

- `base_dir()`
- `snapshots_dir()`
- `catalog_path()`
- `trees_dir()`
- `packs_dir()`
- `index_path()`
- `locks_dir()`
- `attachments_deps_dir()`
- `attachments_git_dir()`
- `tree_path()`

`snapshots_dir()` and the attachment paths remain for compatibility with existing layout expectations, but the core save/restore/delete/list path does not rely on them.

## Error Model

The main core errors are:

- `Io`
- `Sqlite`
- `Bitcode`
- `SnapshotNotFound`
- `LockHeld`
- `GuardrailExceeded`
- `StoreCorrupted`
- `ObjectNotFound`
- `RestoreFailed`
- `Other`

There is no JSON serialization error path in `chkpt-core` anymore.

## Interface Layers

### CLI

Current commands:

- `chkpt save [-m MESSAGE] [--include-deps]`
- `chkpt list [-n LIMIT] [--full]`
- `chkpt restore [ID] [--dry-run]`
- `chkpt delete ID`

If restore id is omitted, the CLI offers an interactive selector.

### MCP

Current MCP operations mirror save/list/restore/delete and pass through to `chkpt-core`.

### Node.js

`chkpt-napi` exposes:

- high-level save/list/restore/delete
- scanner bindings
- low-level blob/tree/index/config helpers

It does not expose snapshot JSON read/write helpers anymore.

## Current Limitations

- `StoreLayout` still creates compatibility directories that are not part of the live data path.
- Tree building still exists in the save path because `root_tree_hash` remains part of the snapshot model.
- `ARCHITECTURE.md` intentionally describes the current implementation, not older design iterations preserved under `docs/plans/`.

## Test Coverage

Important active integration coverage includes:

- `catalog_test.rs`
- `blob_test.rs`
- `tree_test.rs`
- `pack_test.rs`
- `save_test.rs`
- `restore_test.rs`
- `delete_test.rs`
- `list_test.rs`
- `lock_test.rs`
- `config_test.rs`
- `e2e_test.rs`

There is no longer a `snapshot_test.rs` because `SnapshotStore` persistence was removed from the live system.
