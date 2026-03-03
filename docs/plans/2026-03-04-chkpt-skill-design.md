# chkpt Skill Design Document

## Overview

Claude Code skill that enables chkpt checkpoint operations without MCP setup. Combines three roles in a single skill: proactive automation (auto-save/restore suggestions), CLI mirroring (direct checkpoint operations via Bash), and store inspection/debugging.

## Decisions

| Item              | Decision                                                          |
| ----------------- | ----------------------------------------------------------------- |
| Skill count       | Single unified skill                                              |
| Location          | `crates/chkpt-cli/skills/chkpt/SKILL.md`                          |
| Detail separation | `references/` for CLI commands, store layout, automation patterns |
| CLI access        | Assumes `chkpt` binary installed in PATH                          |
| Allowed tools     | Bash, Read, Glob, Grep, AskUserQuestion                           |
| User-invocable    | Yes (`/chkpt`)                                                    |

---

## File Structure

```
crates/chkpt-cli/skills/chkpt/
├── SKILL.md
└── references/
    ├── cli-commands.md
    ├── store-layout.md
    └── automation-patterns.md
```

## SKILL.md Structure

### Frontmatter

```yaml
name: chkpt
description: Filesystem checkpoint automation — save, restore, list, delete workspace snapshots and auto-protect work during risky operations.
user-invocable: true
allowed-tools:
  - Bash
  - Read
  - Glob
  - Grep
  - AskUserQuestion
```

### Sections

**`<role>`** — Workspace checkpoint assistant that proactively protects work and provides direct checkpoint operations.

**`<context>`** — Brief description of chkpt: content-addressed dedup, zstd compression, BLAKE3 hashing, no Git pollution. Points to references for details.

**`<workflow>`** — Three-mode workflow:

1. **Proactive automation** — Detect risky operations (large refactors, file deletions, dependency changes), suggest `chkpt save` before and `chkpt restore` on failure. See `references/automation-patterns.md`.
2. **Direct operations** — Execute save/list/restore/delete via CLI. See `references/cli-commands.md`. Restore always does dry-run first.
3. **Store inspection** — Read snapshot JSONs, diagnose store health, compare snapshots. See `references/store-layout.md`.

**`<constraints>`** — Restore requires dry-run + user confirmation. Delete requires user confirmation. Never modify store files directly. All operations through `chkpt` CLI only.

**`<references>`** — Links to the three reference files.

---

## References Content

### `references/cli-commands.md`

Complete CLI command reference:

| Command                                  | Arguments                     | Output                                  |
| ---------------------------------------- | ----------------------------- | --------------------------------------- |
| `chkpt save [-m <msg>]`                  | Optional message              | Snapshot ID, file count                 |
| `chkpt list [--limit N]`                 | Optional limit (default: all) | Table: ID, date, file count, message    |
| `chkpt restore <id\|latest> [--dry-run]` | Snapshot ID or "latest"       | Added/modified/removed/unchanged counts |
| `chkpt delete <id>`                      | Snapshot ID                   | Confirmation, GC stats                  |

Error codes and their meanings: `LOCK_HELD`, `SNAPSHOT_NOT_FOUND`, `GUARDRAIL_EXCEEDED`, `STORE_CORRUPTED`, `IO_ERROR`.

Output formatting guidelines for each command.

### `references/store-layout.md`

Store structure at `~/.chkpt/stores/<project_id>/`:

```
├── config.json          — { project_root, created_at }
├── snapshots/<id>.json  — { id, created_at, tree_hash, file_count, message? }
├── objects/<prefix>/    — zstd-compressed blobs (BLAKE3 hash)
├── trees/<prefix>/      — bincode-serialized tree nodes
├── packs/               — packfiles (.dat + .idx)
├── index.sqlite         — change detection cache
├── locks/project.lock   — advisory lock
└── attachments/         — deps/ and git/ archives
```

Snapshot JSON schema for direct reading. Tree node structure for comparison operations. Project ID derivation (BLAKE3 of canonical workspace path, first 16 hex chars).

### `references/automation-patterns.md`

When to suggest `chkpt save`:

- Before large-scale refactoring (>5 files changing)
- Before file/directory deletion operations
- Before dependency updates (package.json, Cargo.toml, etc.)
- After completing a major feature or milestone
- Before risky git operations (rebase, reset)

When to suggest `chkpt restore`:

- Build/test failures after changes, when reverting would help
- User explicitly wants to undo recent changes
- Workspace state appears corrupted

When NOT to auto-suggest:

- Minor edits (1-2 files, small changes)
- Read-only operations
- When user has explicitly declined suggestions recently

---

## Trigger Conditions

The skill activates when:

1. User invokes `/chkpt` directly
2. User mentions "checkpoint", "snapshot", "save state", "restore state"
3. Claude detects a risky operation pattern (proactive mode — referenced from automation-patterns.md)
