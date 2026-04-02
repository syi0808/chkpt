# Store Layout & Inspection Guide

## Current Store Layout

chkpt stores data under:

```text
~/.chkpt/stores/<project_id>/
├── catalog.sqlite          snapshot metadata, manifests, blob locations
├── index.bin               current-workspace metadata cache
├── locks/
│   └── project.lock
├── objects/                loose compressed blobs
├── trees/                  content-addressed tree nodes
├── packs/                  packed blobs and indexes
├── snapshots/              reserved compatibility directory
└── attachments/            reserved compatibility directory
```

The live save/restore/delete/list path is driven by `catalog.sqlite`. chkpt no longer stores snapshot JSON files or `config.json`.

## How To Find The Right Store

There is no `config.json` to map workspace path to store directory anymore.

Use this workflow instead:

1. Call `checkpoint_list` for the current workspace.
2. Search candidate stores under `~/.chkpt/stores/*/catalog.sqlite`.
3. Query each catalog until you find snapshot IDs or timestamps that match the `checkpoint_list` output.

Example:

```bash
for db in ~/.chkpt/stores/*/catalog.sqlite; do
  echo "== $db =="
  sqlite3 "$db" "select id, created_at, message from snapshots order by created_at desc limit 5;"
done
```

## Catalog Schema

### `snapshots`

Stores snapshot metadata:

- `id`
- `created_at`
- `message`
- `parent_snapshot_id`
- `manifest_snapshot_id`
- `root_tree_hash`
- `total_files`
- `total_bytes`
- `new_objects`

### `snapshot_files`

Stores the flattened manifest for a snapshot:

- `snapshot_id`
- `path`
- `blob_hash`
- `size`
- `mode`

### `blob_index`

Stores where each blob lives:

- `blob_hash`
- `pack_hash`
- `size`

## Inspection Recipes

### List snapshots from a catalog

```bash
sqlite3 ~/.chkpt/stores/<project_id>/catalog.sqlite \
  "select id, created_at, message, total_files, new_objects from snapshots order by created_at desc;"
```

### Inspect a snapshot manifest

```bash
sqlite3 ~/.chkpt/stores/<project_id>/catalog.sqlite \
  "select path, hex(blob_hash), size, mode from snapshot_files where snapshot_id = '<snapshot-id>' order by path;"
```

If the snapshot reuses another manifest, first resolve the manifest owner:

```bash
sqlite3 ~/.chkpt/stores/<project_id>/catalog.sqlite \
  "select id, coalesce(manifest_snapshot_id, id), hex(root_tree_hash) from snapshots where id = '<snapshot-id>';"
```

### Inspect blob locations

```bash
sqlite3 ~/.chkpt/stores/<project_id>/catalog.sqlite \
  "select hex(blob_hash), pack_hash, size from blob_index limit 20;"
```

### Check store disk usage

```bash
du -sh ~/.chkpt/stores/<project_id>/
du -sh ~/.chkpt/stores/<project_id>/objects/
du -sh ~/.chkpt/stores/<project_id>/packs/
du -sh ~/.chkpt/stores/<project_id>/trees/
```

### Count loose objects

```bash
find ~/.chkpt/stores/<project_id>/objects -type f | wc -l
```

## Tree Nodes

Tree nodes are still used for reconstruction and fallback traversal.

Each `TreeEntry` stores:

- `name`
- `entry_type`
- `hash`
- `size`
- `mode`

They are content-addressed and stored under `trees/<prefix>/<rest>`.
