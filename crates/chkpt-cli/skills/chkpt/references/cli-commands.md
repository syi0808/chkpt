# CLI Command Reference

## Commands

### `chkpt save`

Save a checkpoint of the current workspace.

```bash
chkpt save [-m <message>] [--include-deps]
```

| Argument          | Required | Description                                                |
| ----------------- | -------- | ---------------------------------------------------------- |
| `-m`, `--message` | No       | Human-readable label for the checkpoint                    |
| `--include-deps`  | No       | Include dependency directories like `node_modules`, `.venv` |

**Output:**

```
Checkpoint saved: <uuid>
  Files: <n>, New objects: <n>, Total bytes: <n>
```

**Example:**

```bash
chkpt save -m "before refactoring auth module"
```

---

### `chkpt list`

List all checkpoints, newest first.

```bash
chkpt list [--limit <n>] [--full]
```

| Argument        | Required | Description                           |
| --------------- | -------- | ------------------------------------- |
| `-n`, `--limit` | No       | Maximum number of checkpoints to show |
| `--full`        | No       | Show full snapshot IDs                |

**Output:**

```
ID         Created                Files    Message
------------------------------------------------------------
a1b2c3d4   2026-03-04 14:30:00   142      before refactoring
e5f6a7b8   2026-03-04 13:00:00   140      initial state

2 checkpoint(s)
```

**Example:**

```bash
chkpt list -n 5
```

---

### `chkpt restore`

Restore workspace to a previous checkpoint.

```bash
chkpt restore [<id|latest>] [--dry-run]
```

| Argument    | Required | Description                                                 |
| ----------- | -------- | ----------------------------------------------------------- |
| `id`        | No       | Snapshot ID, prefix, or `latest`. If omitted, CLI prompts. |
| `--dry-run` | No       | Preview changes without modifying files                     |

**Dry-run output:**

```
Dry run -- no changes made:
  Added: <n>, Changed: <n>, Removed: <n>, Unchanged: <n>
```

**Restore output:**

```
Restored to checkpoint <id>:
  Added: <n>, Changed: <n>, Removed: <n>, Unchanged: <n>
```

**IMPORTANT:** Always run with `--dry-run` first and show results to the user. Only proceed with actual restore after explicit user confirmation.

**Example:**

```bash
chkpt restore latest --dry-run
chkpt restore a1b2c3d4
```

---

### `chkpt delete`

Delete a checkpoint and run garbage collection.

```bash
chkpt delete <id>
```

| Argument | Required | Description           |
| -------- | -------- | --------------------- |
| `id`     | Yes      | Snapshot ID to delete |

**Output:**

```
Checkpoint <id> deleted.
```

**IMPORTANT:** Always show snapshot details (from `chkpt list`) and ask for user confirmation before deleting.

---

## Error Handling

| Error                          | Meaning                                | Action                                  |
| ------------------------------ | -------------------------------------- | --------------------------------------- |
| `Lock held by another process` | Another chkpt operation is in progress | Wait and retry, or inform user          |
| `Snapshot not found: <id>`     | Invalid snapshot ID                    | Run `chkpt list` to show available IDs  |
| `Store corrupted: <detail>`    | Integrity issue in store               | Suggest inspecting store with Read tool |
| `IO error: <detail>`           | File system error                      | Show raw error to user                  |
