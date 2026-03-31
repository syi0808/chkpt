# Performance Hotpaths Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce scanner and restore hot-path overhead in `chkpt-core` while preserving save/restore behavior through tests.

**Architecture:** Reuse the existing parallel scanner implementation, add a streaming file-hash helper for restore, and replace restore's full index rebuild with a single atomic sync operation. The behavior of snapshots, trees, and restored files must remain unchanged.

**Tech Stack:** Rust 2021, rusqlite, ignore, blake3, zstd, tempfile

---

### Task 1: Lock Scanner Behavior Before Refactor

**Files:**
- Modify: `crates/chkpt-core/tests/scanner_test.rs`
- Modify: `crates/chkpt-core/src/scanner/mod.rs`
- Modify: `crates/chkpt-core/src/ops/save.rs`

- [ ] **Step 1: Write the failing test**

Add a public parallel scan entrypoint test in `crates/chkpt-core/tests/scanner_test.rs` that calls `chkpt_core::scanner::scan_workspace_parallel(...)` and compares it with `scan_workspace(...)`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p chkpt-core --test scanner_test`
Expected: FAIL to compile because `scan_workspace_parallel` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Expose `scan_workspace_parallel` in `crates/chkpt-core/src/scanner/mod.rs` by delegating to `walker::walk_parallel`, then make `scan_workspace` delegate to the new parallel entrypoint. Keep the existing sequential walker available for internal use.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p chkpt-core --test scanner_test`
Expected: PASS

### Task 2: Add Streaming File Hashing for Restore

**Files:**
- Modify: `crates/chkpt-core/tests/blob_test.rs`
- Modify: `crates/chkpt-core/src/store/blob.rs`
- Modify: `crates/chkpt-core/src/ops/restore.rs`

- [ ] **Step 1: Write the failing test**

Add a test in `crates/chkpt-core/tests/blob_test.rs` asserting `hash_file(path)` matches `hash_content(bytes)` for the same file contents.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p chkpt-core --test blob_test`
Expected: FAIL to compile because `hash_file` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Implement `hash_file` in `crates/chkpt-core/src/store/blob.rs` using buffered reads and BLAKE3 incremental hashing. Update restore-time current-state hashing in `crates/chkpt-core/src/ops/restore.rs` to use the helper and parallelize stale-file hashing work.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p chkpt-core --test blob_test --test restore_test`
Expected: PASS

### Task 3: Make Restore Index Updates Incremental

**Files:**
- Modify: `crates/chkpt-core/tests/index_test.rs`
- Modify: `crates/chkpt-core/tests/restore_test.rs`
- Modify: `crates/chkpt-core/src/index/mod.rs`
- Modify: `crates/chkpt-core/src/ops/restore.rs`

- [ ] **Step 1: Write the failing test**

Add an index test that calls `FileIndex::apply_changes(remove_paths, upsert_entries)` and verifies removed paths disappear while updated paths remain. Add a restore test that restores after file add/remove/change, then verifies a subsequent save produces `new_objects == 0`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p chkpt-core --test index_test --test restore_test`
Expected: FAIL to compile because `apply_changes` does not exist yet.

- [ ] **Step 3: Write minimal implementation**

Implement `FileIndex::apply_changes` as one transaction that performs removals and upserts. Update restore to compute removed and updated entries and apply only those changes instead of clearing and rebuilding the entire index.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p chkpt-core --test index_test --test restore_test`
Expected: PASS

### Task 4: Verify Workspace

**Files:**
- Modify: `crates/chkpt-core/examples/bench_ops.rs` (only if benchmark compatibility changes are required)

- [ ] **Step 1: Run focused verification**

Run: `cargo test -p chkpt-core --test scanner_test --test blob_test --test index_test --test restore_test --test save_test`
Expected: PASS

- [ ] **Step 2: Run full core verification**

Run: `cargo test -p chkpt-core`
Expected: PASS

- [ ] **Step 3: Re-run performance benchmark**

Run: `cargo run -p chkpt-core --example bench_ops --release -- --iterations 3`
Expected: save/restore timings improve or remain neutral without behavior regressions.
