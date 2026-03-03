# chkpt Skill Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a Claude Code skill that provides chkpt checkpoint operations (save/list/restore/delete), proactive automation, and store inspection — without requiring MCP setup.

**Architecture:** Single `SKILL.md` with three reference files in `references/`. The skill uses Bash to invoke the `chkpt` CLI and Read/Glob/Grep for store inspection. All detailed content (CLI commands, store layout, automation patterns) lives in reference files to keep the main SKILL.md concise.

**Tech Stack:** Claude Code skill system (YAML frontmatter + Markdown + semantic HTML tags)

---

### Task 1: Create `references/cli-commands.md`

**Files:**

- Create: `crates/chkpt-cli/skills/chkpt/references/cli-commands.md`

**Step 1: Create the directory structure**

Run: `mkdir -p crates/chkpt-cli/skills/chkpt/references`

**Step 2: Write cli-commands.md**

````markdown
# CLI Command Reference

## Commands

### `chkpt save`

Save a checkpoint of the current workspace.

```bash
chkpt save [-m <message>]
```
````

| Argument          | Required | Description                             |
| ----------------- | -------- | --------------------------------------- |
| `-m`, `--message` | No       | Human-readable label for the checkpoint |

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
chkpt list [--limit <n>]
```

| Argument        | Required | Description                           |
| --------------- | -------- | ------------------------------------- |
| `-n`, `--limit` | No       | Maximum number of checkpoints to show |

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
chkpt restore <id|latest> [--dry-run]
```

| Argument    | Required | Description                                     |
| ----------- | -------- | ----------------------------------------------- |
| `id`        | Yes      | Snapshot ID (first 8 chars suffice) or `latest` |
| `--dry-run` | No       | Preview changes without modifying files         |

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
| `Guardrail exceeded: <detail>` | Workspace exceeds size/file limits     | Inform user of the limit hit            |
| `Store corrupted: <detail>`    | Integrity issue in store               | Suggest inspecting store with Read tool |
| `IO error: <detail>`           | File system error                      | Show raw error to user                  |

````

**Step 3: Commit**

```bash
git add crates/chkpt-cli/skills/chkpt/references/cli-commands.md
git commit -m "feat(skill): add CLI command reference for chkpt skill"
````

---

### Task 2: Create `references/store-layout.md`

**Files:**

- Create: `crates/chkpt-cli/skills/chkpt/references/store-layout.md`

**Step 1: Write store-layout.md**

````markdown
# Store Layout & Inspection Guide

## Locating the Store

chkpt stores data at `~/.chkpt/stores/<project_id>/` where `project_id` is the first 16 hex chars of the BLAKE3 hash of the canonical workspace path.

To find the store for the current workspace:

```bash
# The project ID is derived from the absolute workspace path
# Example: /Users/me/projects/myapp → project_id = a3f8c1e2b4d6...
```
````

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

````

**Step 2: Commit**

```bash
git add crates/chkpt-cli/skills/chkpt/references/store-layout.md
git commit -m "feat(skill): add store layout reference for chkpt skill"
````

---

### Task 3: Create `references/automation-patterns.md`

**Files:**

- Create: `crates/chkpt-cli/skills/chkpt/references/automation-patterns.md`

**Step 1: Write automation-patterns.md**

````markdown
# Automation Patterns

## When to Suggest `chkpt save`

### Before Risky Operations

Suggest saving a checkpoint before the user or you are about to:

- **Large-scale refactoring** — changing more than 5 files, renaming modules, moving directories
- **File or directory deletion** — `rm`, `git rm`, or bulk file removal
- **Dependency changes** — modifying `package.json`, `Cargo.toml`, `requirements.txt`, `go.mod`, etc.
- **Risky git operations** — `git rebase`, `git reset`, `git merge` with conflicts expected
- **Database migrations** — schema changes that affect application state
- **Configuration changes** — environment files, build configs, CI/CD pipelines

**Suggested message format:**

```bash
chkpt save -m "before: <brief description of upcoming operation>"
```
````

### After Milestones

Suggest saving after:

- **Major feature completion** — a logical unit of work is done and working
- **All tests passing** — a known-good state worth preserving
- **Successful build** — after resolving complex build issues

**Suggested message format:**

```bash
chkpt save -m "milestone: <what was achieved>"
```

## When to Suggest `chkpt restore`

- **Repeated build/test failures** — if changes introduced failures and reverting would be cleaner than debugging
- **User requests undo** — "undo", "go back", "revert to before", "roll back"
- **Workspace corruption** — missing files, broken state after failed operations

**Always use dry-run first:**

```bash
chkpt restore latest --dry-run
```

Then show the user what would change and ask for confirmation.

## When NOT to Auto-Suggest

Do not proactively suggest checkpoints when:

- **Minor edits** — changing 1-2 files with small modifications
- **Read-only operations** — browsing, searching, reading files
- **User declined recently** — if the user said "no" to a checkpoint suggestion in the current session, do not suggest again for similar operations
- **Rapid iteration** — user is in a tight edit-test loop; don't interrupt every cycle

````

**Step 2: Commit**

```bash
git add crates/chkpt-cli/skills/chkpt/references/automation-patterns.md
git commit -m "feat(skill): add automation patterns reference for chkpt skill"
````

---

### Task 4: Create `SKILL.md`

**Files:**

- Create: `crates/chkpt-cli/skills/chkpt/SKILL.md`

**Step 1: Write SKILL.md**

```markdown
---
name: chkpt
description: Filesystem checkpoint automation — save, restore, list, delete workspace snapshots and auto-protect work during risky operations.
user-invocable: true
allowed-tools:
  - Bash
  - Read
  - Glob
  - Grep
  - AskUserQuestion
---

<role>
You are a workspace checkpoint assistant powered by chkpt, a filesystem checkpoint engine. You proactively protect work by suggesting checkpoints before risky operations, execute checkpoint commands via CLI, and inspect store internals for debugging. You understand content-addressed storage, BLAKE3 hashing, and zstd compression but communicate in plain terms.
</role>

<context>
chkpt saves workspace state to `~/.chkpt/stores/<project_id>/` without polluting Git. It uses content-addressed deduplication (BLAKE3), zstd compression, and SQLite-based incremental change detection.

See `references/store-layout.md` for full store structure and snapshot schema.
See `references/cli-commands.md` for command details and error handling.
See `references/automation-patterns.md` for when to suggest save/restore.
</context>

<workflow>

## Mode 1: Proactive Automation

When you detect a risky operation is about to happen (see `references/automation-patterns.md`), suggest a checkpoint:

1. Inform the user why a checkpoint would be helpful
2. Propose: `chkpt save -m "before: <description>"`
3. If user agrees, execute via Bash
4. If user declines, proceed without saving

After milestones (feature complete, tests passing), suggest saving the known-good state.

If an operation fails and a recent checkpoint exists, suggest restore as a recovery option.

## Mode 2: Direct Operations

When the user requests a checkpoint operation, execute it:

1. **Save** — Run `chkpt save [-m <message>]`, report snapshot ID and stats
2. **List** — Run `chkpt list [--limit N]`, present the table
3. **Restore** — ALWAYS run `chkpt restore <id> --dry-run` first, show changes, ask for confirmation via AskUserQuestion, then run actual restore only after approval
4. **Delete** — Show snapshot info, ask for confirmation via AskUserQuestion, then run `chkpt delete <id>`

See `references/cli-commands.md` for argument details and output formats.

## Mode 3: Store Inspection

When the user wants to examine checkpoint internals:

1. Locate the store: find `~/.chkpt/stores/*/config.json` where `project_root` matches the workspace
2. Read snapshot JSONs directly with the Read tool
3. Check disk usage, object counts, and pack status via Bash
4. Compare snapshots by reading their tree hashes and diffing entries

See `references/store-layout.md` for directory structure and JSON schemas.

</workflow>

<constraints>
- Restore MUST use `--dry-run` first with user confirmation before actual restore.
- Delete MUST confirm with user before executing.
- Never modify store files directly. All mutations go through the `chkpt` CLI.
- On `Lock held` errors, inform the user another process is using chkpt and suggest waiting.
- On `Snapshot not found`, run `chkpt list` to show available snapshots.
- All CLI commands run from the workspace root directory.
</constraints>

<references>
- `references/cli-commands.md` — Complete CLI command reference with arguments, output formats, and error handling
- `references/store-layout.md` — Store directory structure, snapshot JSON schema, tree node format, and inspection recipes
- `references/automation-patterns.md` — Rules for when to suggest save/restore and when not to
</references>
```

**Step 2: Commit**

```bash
git add crates/chkpt-cli/skills/chkpt/SKILL.md
git commit -m "feat(skill): add main chkpt skill definition"
```

---

### Task 5: Verify the skill structure

**Step 1: Check all files exist with correct paths**

Run: `find crates/chkpt-cli/skills -type f | sort`

Expected:

```
crates/chkpt-cli/skills/chkpt/SKILL.md
crates/chkpt-cli/skills/chkpt/references/automation-patterns.md
crates/chkpt-cli/skills/chkpt/references/cli-commands.md
crates/chkpt-cli/skills/chkpt/references/store-layout.md
```

**Step 2: Verify SKILL.md frontmatter is valid YAML**

Run: `head -11 crates/chkpt-cli/skills/chkpt/SKILL.md`

Expected: Valid YAML between `---` delimiters with name, description, user-invocable, allowed-tools.

**Step 3: Verify references are correctly linked**

Run: `grep -n 'references/' crates/chkpt-cli/skills/chkpt/SKILL.md`

Expected: All three reference files are mentioned in both `<context>` and `<references>` sections.

**Step 4: Commit (if any fixes needed)**

Only commit if changes were made during verification.
