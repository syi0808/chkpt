# Git State Snapshot Design

## Problem

`git bundle --all` only captures committed objects and refs. Critical git state is lost:

- HEAD position / detached HEAD
- Staged changes (index)
- Unstaged working tree diffs
- Untracked files (already covered by chkpt workspace scan)
- Stash entries
- Merge/rebase/cherry-pick in-progress state
- Submodule state
- Local git config (`.git/config`)
- Custom hooks (`.git/hooks/`)

The goal is **perfect restoration** of the entire `.git/` directory at a given point in time.

## Approach

Store `.git/` using chkpt's existing content-addressable pipeline (scan -> BLAKE3 hash -> zstd compress -> pack -> tree). This reuses proven infrastructure and gets dedup + incremental saves for free.

Replaces the existing `git bundle` approach in `attachments/git.rs`.

## Storage Structure

```
Snapshot
├── root_tree_hash      -> workspace file tree (existing)
└── attachments
    └── git_tree_hash   -> .git/ directory tree (new)
```

## Data Model Changes

### `SnapshotAttachments`

```rust
pub struct SnapshotAttachments {
    pub deps_key: Option<String>,
    pub git_key: Option<String>,           // deprecated, kept for backward compat
    pub git_tree_hash: Option<[u8; 32]>,   // NEW: .git/ tree hash
}
```

### `Snapshot`

Add `is_auto_backup: bool` field to distinguish auto-backups from user-created snapshots.

### `SaveOptions`

Add `include_git: bool` (default `false`).

## Save Flow

When `--git` flag is provided:

1. Workspace scan + processing (existing, unchanged)
2. `.git/` directory scan (no exclusions, all files included)
3. `.git/` files hashed + compressed + packed into same blob store
4. `.git/` tree structure built -> `git_tree_hash` obtained
5. `SnapshotAttachments { git_tree_hash: Some(hash) }` stored in snapshot

Steps 1 and 2-4 are independent and can run in **parallel**.

## Restore Flow

**Every** restore creates an auto-backup first:

1. **Auto-backup current state**
   - Save current workspace files as a snapshot
   - If `.git/` exists, include git tree in the backup
   - Mark snapshot as `is_auto_backup: true`
   - Print backup snapshot ID to stdout
2. **Restore workspace files** (existing logic)
3. **If snapshot has `git_tree_hash`**: delete current `.git/` and restore from tree

No `--git` flag on restore. If the snapshot has git state, it is always restored.

## CLI Interface

```bash
# Save
chkpt save -m "message"         # workspace only
chkpt save -m "message" --git   # workspace + git state

# Restore (restores everything saved, always auto-backs up first)
chkpt restore <snapshot-id>

# List (shows GIT column)
chkpt list
# ID       DATE        MESSAGE                          GIT
# abc123   2026-03-31  my checkpoint                    Y
# def456   2026-03-31  [auto-backup before abc123]      Y
```

## Scanner Changes

The existing scanner excludes `.git/` by default. For git state saving, the scanner is reused with exclusions bypassed — all files inside `.git/` are included.

`.chkptignore` rules are **not** applied to `.git/` scanning.

## Code Changes

### Modified

| File | Change |
|------|--------|
| `store/snapshot.rs` | Add `git_tree_hash` to `SnapshotAttachments`, add `is_auto_backup` to `Snapshot` |
| `ops/save.rs` | When `include_git`, scan `.git/` in parallel, build tree, set `git_tree_hash` |
| `ops/restore.rs` | Auto-backup before every restore. Restore `.git/` when `git_tree_hash` present |
| `scanner/` | Support scanning `.git/` without exclusions |
| `chkpt-cli` | `--git` flag on `save`, GIT column on `list` |

### Deprecated

| File | Change |
|------|--------|
| `attachments/git.rs` | `create_git_bundle` / `restore_git_bundle` deprecated, replaced by new approach |
| `SnapshotAttachments.git_key` | Field kept for backward compat, not used in new saves |

## Auto-Backup Retention

Auto-backup snapshots are stored as regular snapshots (with `is_auto_backup: true`). They follow the same lifecycle — visible in `chkpt list` and deletable via `chkpt delete`. No automatic cleanup or expiration.

### Unchanged

- Blob store, pack, tree build logic
- FileIndex (reusable for `.git/` files)
- `.chkptignore` processing
