# Performance Storage Redesign Design

## Goal

Replace the save/restore metadata path with a manifest-driven catalog so `save`, `list`, `restore`, and `delete` no longer depend on tree reconstruction, snapshot JSON directory scans, or pack-file discovery scans.

## Constraints

- Backward compatibility is not required.
- Behavior must remain correct before performance work is considered successful.
- Risky changes must be protected by tests written first.
- End-to-end tests are the primary safety net.

## Current Problems

- `save` still scans the workspace, loads the entire file index, and rebuilds the full tree even for warm saves.
- `restore --dry-run` scans the workspace again and reads changed files fully into memory to hash them.
- `list`, `latest`, and prefix resolution scale with the number of snapshot JSON files.
- Pack lookup scales with the number of pack files because packs are opened eagerly.
- `restore` clears and rebuilds the entire index after apply.

## Proposed Design

### 1. Catalog-backed metadata

Add a new SQLite catalog at the store root containing:

- `snapshots`: snapshot metadata, parent pointer, stats, creation timestamp
- `snapshot_files`: flattened snapshot manifest rows (`snapshot_id`, `path`, `blob_hash`, `size`, `mode`)
- `blob_index`: where a blob lives (`pack_hash` or loose object metadata)

The existing `FileIndex` remains as the current-workspace cache for metadata equality checks, but snapshot and blob discovery move to the catalog.

### 2. Manifest-driven save

- Make workspace scanning parallel by default.
- Reuse cached blob hashes for unchanged files from `FileIndex`.
- Stream-hash and compress only changed files.
- Write new blobs straight to pack files and record them in `blob_index`.
- Persist the snapshot manifest directly into `snapshot_files`.
- Remove tree-building from the hot path.

### 3. Manifest-driven restore

- Resolve snapshots from the catalog (`latest`, exact ID, prefix).
- Load the target manifest directly from `snapshot_files`.
- Parallel-scan the workspace and stream-hash only cache misses.
- Compute diffs from the target manifest and current state.
- Update only changed/added/removed `FileIndex` rows after apply.

### 4. Delete and garbage collection

- Delete snapshot metadata and manifest rows from the catalog.
- Remove unreferenced loose blobs immediately.
- Remove pack files only when no remaining blobs reference their `pack_hash`.
- Do not rebuild tree reachability because trees are no longer part of the operational path.

## Testing Strategy

### Unit tests

- Catalog CRUD, latest lookup, prefix resolution, manifest round-trips, blob index lookups
- Save/restore helpers for incremental updates and partial index rewrites
- Pack resolution by `blob_index`

### End-to-end tests

- Multi-snapshot lifecycle: save -> list -> restore -> delete -> restore again
- Warm save after restore remains incremental
- Dry-run reports exact add/change/remove counts without modifying files
- Many snapshots with overlapping and diverging files remain restorable after deletions
- Prefix restore works, ambiguous prefix errors correctly, latest stays stable after deletes
- Packed blobs remain restorable after old snapshots are removed

## Non-goals

- Migrating old stores
- Changing public CLI syntax
- Rewriting independent `blob`, `tree`, or `snapshot` unit-test fixtures unless needed by the new operational path
