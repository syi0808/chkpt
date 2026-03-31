# chkpt Performance Hotpaths Design

**Goal:** Reduce `chkpt-core` save/restore latency on medium and large workspaces without changing checkpoint semantics.

## Scope

This design covers three hot-path improvements in `chkpt-core`:

1. Make workspace scans use the existing parallel walker by default.
2. Replace restore-time full-file reads for hashing with a streaming hash API that can be parallelized.
3. Replace restore-time index clear-and-rebuild with an atomic incremental index sync.

## Current Problem

Recent benchmark runs show two dominant costs:

- Total file count drives `warm save` and `restore --dry-run`, which points at workspace scanning and full index loading.
- Large modified-file restores drive `restore apply`, which points at restore-time hashing and index rebuild work.

The current code already contains a parallel scanner implementation, but the public `scan_workspace` entrypoint still calls the sequential walker. Restore also reads entire files into memory to hash them and then clears the full SQLite index before rebuilding it from scratch.

## Proposed Design

### 1. Parallel Scan as Default

Expose a public `scan_workspace_parallel` entrypoint in `scanner/mod.rs` and make `scan_workspace` dispatch to it. This keeps scanner behavior stable while moving callers onto the faster existing implementation.

The parallel walker already sorts the final file list, so save/restore ordering remains deterministic.

### 2. Streaming File Hash API

Add a streaming file-hash helper in `store/blob.rs` that hashes a file via buffered reads instead of `std::fs::read`. Restore will use this helper when cached metadata is stale.

Restore-time hashing will also be parallelized using the same pattern already used by save-time file preparation: chunk files across worker threads, hash independently, then merge results.

### 3. Incremental Index Sync

Add a `FileIndex::apply_changes` API that atomically removes stale paths and upserts changed entries in a single transaction. Restore will update only the changed portion of the index instead of deleting and rebuilding the entire table.

This keeps index contents equivalent to the prior implementation while cutting unnecessary SQLite work for unchanged files.

## Non-Goals

- No change to snapshot format.
- No change to tree encoding or pack layout.
- No attempt to optimize pack loading in this change set.

## Testing

- Scanner tests verify public sequential and parallel entrypoints remain equivalent.
- Blob tests verify streaming file hashing matches existing in-memory hashing.
- Index tests verify incremental sync semantics.
- Restore tests verify restore correctness and incremental-save behavior remain intact after refactoring.

