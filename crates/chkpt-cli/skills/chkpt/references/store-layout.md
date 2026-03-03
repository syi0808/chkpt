# Store Layout & Inspection Guide

## Locating the Store

chkpt stores data at `~/.chkpt/stores/<project_id>/` where `project_id` is the first 16 hex chars of the BLAKE3 hash of the canonical workspace path.

To find the store for the current workspace:

```bash
# The project ID is derived from the absolute workspace path
# Example: /Users/me/projects/myapp → project_id = a3f8c1e2b4d6...
```

You can confirm by reading `~/.chkpt/stores/<project_id>/config.json` which contains `project_root`.

## Directory Structure

```
~/.chkpt/stores/<project_id>/
├── config.json              # Project metadata
├── snapshots/               # Snapshot JSON files
│   └── <uuid>.json
├── objects/                 # Content-addressed blobs (zstd compressed)
│   └── <2-char-prefix>/
│       └── <remaining-hash>
├── trees/                   # Directory tree nodes (bincode serialized)
│   └── <2-char-prefix>/
│       └── <remaining-hash>
├── packs/                   # Packfiles for compacted objects
│   ├── pack-<hash>.dat
│   └── pack-<hash>.idx
├── index.sqlite             # Change detection cache
├── locks/
│   └── project.lock         # Advisory file lock
└── attachments/
    ├── deps/                # node_modules archives (tar.zst)
    └── git/                 # git bundle files
```

## Snapshot JSON Schema

Each file in `snapshots/` is a JSON document:

```json
{
  "id": "019579a2-...",
  "created_at": "2026-03-04T14:30:00Z",
  "message": "before refactoring",
  "root_tree_hash": [163, 248, ...],
  "parent_snapshot_id": null,
  "attachments": {
    "deps_key": null,
    "git_key": null
  },
  "stats": {
    "total_files": 142,
    "total_bytes": 8392048,
    "new_objects": 5
  }
}
```

| Field                | Type           | Description                             |
| -------------------- | -------------- | --------------------------------------- |
| `id`                 | UUID v7 string | Unique snapshot identifier              |
| `created_at`         | ISO 8601       | Creation timestamp                      |
| `message`            | string or null | User-provided label                     |
| `root_tree_hash`     | [u8; 32]       | BLAKE3 hash of the root tree node       |
| `parent_snapshot_id` | string or null | Previous snapshot in chain              |
| `stats.total_files`  | u64            | Number of files in snapshot             |
| `stats.total_bytes`  | u64            | Total uncompressed size                 |
| `stats.new_objects`  | u64            | Newly stored objects (not deduplicated) |

## Tree Node Structure

Tree nodes are bincode-serialized arrays of `TreeEntry`:

```rust
struct TreeEntry {
    name: String,       // File or directory name
    entry_type: EntryType, // File, Dir, or Symlink
    hash: [u8; 32],     // BLAKE3 hash (blob hash for files, tree hash for dirs)
    size: u64,          // File size in bytes
    mode: u32,          // Unix file mode
}
```

Trees are stored at `trees/<prefix>/<rest>` using the first 2 chars of the hex hash as prefix.

## config.json Schema

```json
{
  "project_root": "/Users/me/projects/myapp",
  "created_at": "2026-03-04T12:00:00Z",
  "guardrails": {
    "max_total_bytes": 2147483648,
    "max_files": 100000,
    "max_file_size": 104857600
  }
}
```

## Inspection Recipes

### List all snapshots with full details

```bash
ls ~/.chkpt/stores/*/snapshots/*.json
```

Then use the Read tool on individual snapshot JSON files.

### Check store disk usage

```bash
du -sh ~/.chkpt/stores/<project_id>/
du -sh ~/.chkpt/stores/<project_id>/objects/
du -sh ~/.chkpt/stores/<project_id>/packs/
```

### Count loose objects

```bash
find ~/.chkpt/stores/<project_id>/objects -type f | wc -l
```

### Verify config matches workspace

Use the Read tool on `~/.chkpt/stores/<project_id>/config.json` and check `project_root`.
