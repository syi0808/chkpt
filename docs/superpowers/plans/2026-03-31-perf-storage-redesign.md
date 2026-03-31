# Performance Storage Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the operational snapshot/tree path with a manifest-driven SQLite catalog while preserving end-to-end checkpoint behavior and improving large-workspace performance.

**Architecture:** Keep `FileIndex` as the current-workspace cache, add a new catalog for snapshots/manifests/blob locations, switch `save`/`restore`/`list`/`delete` to the catalog, and leave legacy tree/snapshot modules only for isolated tests not on the hot path.

**Tech Stack:** Rust, rusqlite, tempfile, existing pack/blob stores, cargo test

---

### Task 1: Lock in failing tests for the new storage path

**Files:**
- Create: `crates/chkpt-core/tests/catalog_test.rs`
- Modify: `crates/chkpt-core/tests/e2e_test.rs`

- [ ] **Step 1: Write the failing catalog tests**

Add tests for snapshot manifest round-trip, latest lookup, prefix lookup, and blob index lookup against a not-yet-existing catalog API.

- [ ] **Step 2: Run test to verify it fails**

Run: `env HOME=/tmp/chkpt-worktree-baseline RUSTUP_HOME=/Users/classting/.rustup CARGO_HOME=/Users/classting/.cargo /Users/classting/.cargo/bin/cargo test -p chkpt-core --test catalog_test`
Expected: FAIL because the catalog module and APIs do not exist yet.

- [ ] **Step 3: Add dense E2E cases before implementation**

Extend `e2e_test.rs` with multi-snapshot prefix, delete-after-packed-save, and repeated restore/save lifecycle tests.

- [ ] **Step 4: Run focused E2E tests to verify new coverage**

Run: `env HOME=/tmp/chkpt-worktree-baseline RUSTUP_HOME=/Users/classting/.rustup CARGO_HOME=/Users/classting/.cargo /Users/classting/.cargo/bin/cargo test -p chkpt-core --test e2e_test`
Expected: FAIL for cases that depend on the new catalog-backed behavior.

### Task 2: Implement the catalog layer

**Files:**
- Create: `crates/chkpt-core/src/store/catalog.rs`
- Modify: `crates/chkpt-core/src/store/mod.rs`
- Modify: `crates/chkpt-core/src/config.rs`

- [ ] **Step 1: Write the minimal catalog schema and operations**

Implement SQLite tables and methods for snapshots, snapshot manifests, and blob locations.

- [ ] **Step 2: Run catalog tests**

Run: `env HOME=/tmp/chkpt-worktree-baseline RUSTUP_HOME=/Users/classting/.rustup CARGO_HOME=/Users/classting/.cargo /Users/classting/.cargo/bin/cargo test -p chkpt-core --test catalog_test`
Expected: PASS

### Task 3: Switch save/list to the catalog path

**Files:**
- Modify: `crates/chkpt-core/src/scanner/mod.rs`
- Modify: `crates/chkpt-core/src/ops/save.rs`
- Modify: `crates/chkpt-core/src/ops/list.rs`

- [ ] **Step 1: Make workspace scanning parallel by default**

Update `scan_workspace` to use the existing parallel walker.

- [ ] **Step 2: Replace tree/snapshot writes with catalog manifest writes**

Save manifests directly and record new pack/blob locations in the catalog.

- [ ] **Step 3: Run save and E2E tests**

Run: `env HOME=/tmp/chkpt-worktree-baseline RUSTUP_HOME=/Users/classting/.rustup CARGO_HOME=/Users/classting/.cargo /Users/classting/.cargo/bin/cargo test -p chkpt-core --test save_test --test e2e_test`
Expected: PASS

### Task 4: Switch restore/delete to the catalog path

**Files:**
- Modify: `crates/chkpt-core/src/ops/restore.rs`
- Modify: `crates/chkpt-core/src/ops/delete.rs`
- Modify: `crates/chkpt-core/src/store/pack.rs`

- [ ] **Step 1: Replace restore snapshot resolution and manifest load with catalog lookups**

Use manifest rows rather than tree reconstruction.

- [ ] **Step 2: Replace full index rebuild with partial index updates**

Keep unchanged paths intact and rewrite only changed/added/removed rows.

- [ ] **Step 3: Update delete GC to use manifest/blob references**

Delete unreachable loose blobs and fully-unreferenced packs.

- [ ] **Step 4: Run restore/delete/E2E tests**

Run: `env HOME=/tmp/chkpt-worktree-baseline RUSTUP_HOME=/Users/classting/.rustup CARGO_HOME=/Users/classting/.cargo /Users/classting/.cargo/bin/cargo test -p chkpt-core --test restore_test --test delete_test --test e2e_test`
Expected: PASS

### Task 5: Re-measure and verify the redesign

**Files:**
- Modify: `crates/chkpt-core/examples/bench_ops.rs` (only if needed for new store assumptions)

- [ ] **Step 1: Run the full suite**

Run: `env HOME=/tmp/chkpt-worktree-baseline RUSTUP_HOME=/Users/classting/.rustup CARGO_HOME=/Users/classting/.cargo /Users/classting/.cargo/bin/cargo test`
Expected: PASS

- [ ] **Step 2: Run the benchmark example**

Run: `env HOME=/Users/classting/Workspace/temp/chkpt/.bench-home target/release/examples/bench_ops --files 10000 --modified-files 1000 --dirs 200 --iterations 1`
Expected: Lower warm-save and restore overhead than the pre-redesign baseline.
