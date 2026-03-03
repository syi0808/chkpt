# chkpt Plugin & Marketplace Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Package chkpt as a Claude Code plugin (MCP server + Skill) and create a GitHub-hosted marketplace for distribution.

**Architecture:** Mono plugin at `crates/chkpt-plugin/` containing MCP server config (via `npx @chkpt/mcp`) and adapted Skill. Marketplace catalog at project root `.claude-plugin/marketplace.json`.

**Tech Stack:** Claude Code plugin system (plugin.json, .mcp.json, SKILL.md, marketplace.json)

---

### Task 1: Create plugin directory structure and manifest

**Files:**
- Create: `crates/chkpt-plugin/.claude-plugin/plugin.json`

**Step 1: Create directories**

Run:
```bash
mkdir -p crates/chkpt-plugin/.claude-plugin
mkdir -p crates/chkpt-plugin/skills/chkpt/references
```

**Step 2: Create plugin.json**

Create `crates/chkpt-plugin/.claude-plugin/plugin.json`:

```json
{
  "name": "chkpt",
  "description": "Fast, content-addressable workspace checkpoints. Save and restore workspace snapshots without Git pollution.",
  "version": "0.1.2",
  "author": {
    "name": "chkpt"
  },
  "license": "MIT",
  "keywords": ["checkpoint", "snapshot", "backup", "workspace"]
}
```

**Step 3: Commit**

```bash
git add crates/chkpt-plugin/.claude-plugin/plugin.json
git commit -m "feat(plugin): add plugin manifest"
```

---

### Task 2: Create MCP server configuration

**Files:**
- Create: `crates/chkpt-plugin/.mcp.json`

**Step 1: Create .mcp.json**

Create `crates/chkpt-plugin/.mcp.json`:

```json
{
  "mcpServers": {
    "chkpt": {
      "command": "npx",
      "args": ["-y", "@chkpt/mcp"]
    }
  }
}
```

This references the existing `@chkpt/mcp` npm package (v0.1.2) which automatically resolves the correct platform-specific binary from `@chkpt/platform-*` packages. The MCP server exposes 4 tools:
- `checkpoint_save` — Save a workspace checkpoint
- `checkpoint_list` — List checkpoints (newest first)
- `checkpoint_restore` — Restore workspace (supports dry-run)
- `checkpoint_delete` — Delete checkpoint + garbage collect

**Step 2: Commit**

```bash
git add crates/chkpt-plugin/.mcp.json
git commit -m "feat(plugin): add MCP server configuration"
```

---

### Task 3: Create plugin Skill (SKILL.md)

**Files:**
- Create: `crates/chkpt-plugin/skills/chkpt/SKILL.md`

**Step 1: Create SKILL.md**

Create `crates/chkpt-plugin/skills/chkpt/SKILL.md` — adapted from `crates/chkpt-cli/skills/chkpt/SKILL.md` with MCP-first approach:

```markdown
---
name: chkpt
description: Filesystem checkpoint automation — save, restore, list, delete workspace snapshots and auto-protect work during risky operations.
user-invocable: true
---

<role>
You are a workspace checkpoint assistant powered by chkpt. You proactively protect work by suggesting checkpoints before risky operations, execute checkpoint operations via MCP tools, and inspect store internals for debugging.
</role>

<context>
chkpt saves workspace state to `~/.chkpt/stores/<project_id>/` without polluting Git. It uses content-addressed deduplication (BLAKE3), zstd compression, and SQLite-based incremental change detection.

This plugin provides 4 MCP tools: `checkpoint_save`, `checkpoint_list`, `checkpoint_restore`, `checkpoint_delete`. All tools require a `workspace_path` parameter.

See `references/store-layout.md` for full store structure and snapshot schema.
See `references/cli-commands.md` for CLI fallback reference.
See `references/automation-patterns.md` for when to suggest save/restore.
</context>

<workflow>

## Mode 1: Proactive Automation

When you detect a risky operation is about to happen (see `references/automation-patterns.md`), suggest a checkpoint:

1. Inform the user why a checkpoint would be helpful
2. Propose saving with a descriptive message
3. If user agrees, use the `checkpoint_save` MCP tool with the workspace path and message
4. If user declines, proceed without saving

After milestones (feature complete, tests passing), suggest saving the known-good state.

If an operation fails and a recent checkpoint exists, suggest restore as a recovery option.

## Mode 2: Direct Operations

When the user requests a checkpoint operation:

1. **Save** — Use `checkpoint_save` MCP tool with `workspace_path` and optional `message`. Report snapshot ID and stats from the response.
2. **List** — Use `checkpoint_list` MCP tool with `workspace_path` and optional `limit`. Present the results.
3. **Restore** — ALWAYS use `checkpoint_restore` with `dry_run: true` first, show changes, ask for confirmation via AskUserQuestion, then call again with `dry_run: false` only after approval.
4. **Delete** — List checkpoints first to show details, ask for confirmation via AskUserQuestion, then use `checkpoint_delete` MCP tool.

### CLI Fallback

If MCP tools are not available, fall back to CLI commands:
- `chkpt save [-m <message>]`
- `chkpt list [--limit N]`
- `chkpt restore <id> [--dry-run]`
- `chkpt delete <id>`

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
- Restore MUST use dry_run first with user confirmation before actual restore.
- Delete MUST confirm with user before executing.
- Never modify store files directly. All mutations go through MCP tools or CLI.
- On lock errors, inform the user another process is using chkpt and suggest waiting.
- On snapshot not found errors, list available snapshots to help the user.
</constraints>

<references>
- `references/cli-commands.md` — Complete CLI command reference with arguments, output formats, and error handling
- `references/store-layout.md` — Store directory structure, snapshot JSON schema, tree node format, and inspection recipes
- `references/automation-patterns.md` — Rules for when to suggest save/restore and when not to
</references>
```

**Step 2: Commit**

```bash
git add crates/chkpt-plugin/skills/chkpt/SKILL.md
git commit -m "feat(plugin): add chkpt skill with MCP-first approach"
```

---

### Task 4: Copy reference files

**Files:**
- Create: `crates/chkpt-plugin/skills/chkpt/references/cli-commands.md`
- Create: `crates/chkpt-plugin/skills/chkpt/references/store-layout.md`
- Create: `crates/chkpt-plugin/skills/chkpt/references/automation-patterns.md`

**Step 1: Copy references from existing skill**

```bash
cp crates/chkpt-cli/skills/chkpt/references/cli-commands.md crates/chkpt-plugin/skills/chkpt/references/
cp crates/chkpt-cli/skills/chkpt/references/store-layout.md crates/chkpt-plugin/skills/chkpt/references/
cp crates/chkpt-cli/skills/chkpt/references/automation-patterns.md crates/chkpt-plugin/skills/chkpt/references/
```

These files are used as-is — no modifications needed.

**Step 2: Commit**

```bash
git add crates/chkpt-plugin/skills/chkpt/references/
git commit -m "feat(plugin): add skill reference files"
```

---

### Task 5: Create marketplace catalog

**Files:**
- Create: `.claude-plugin/marketplace.json`

**Step 1: Create marketplace directory**

```bash
mkdir -p .claude-plugin
```

**Step 2: Create marketplace.json**

Create `.claude-plugin/marketplace.json`:

```json
{
  "name": "chkpt-marketplace",
  "owner": {
    "name": "chkpt"
  },
  "metadata": {
    "description": "Workspace checkpoint tools for Claude Code"
  },
  "plugins": [
    {
      "name": "chkpt",
      "source": "./crates/chkpt-plugin",
      "description": "Fast workspace checkpoints — save and restore snapshots without Git pollution",
      "version": "0.1.2",
      "category": "developer-tools",
      "tags": ["checkpoint", "snapshot", "backup", "workspace"],
      "keywords": ["checkpoint", "snapshot", "backup"]
    }
  ]
}
```

**Step 3: Commit**

```bash
git add .claude-plugin/marketplace.json
git commit -m "feat: add marketplace catalog for plugin distribution"
```

---

### Task 6: Create plugin README

**Files:**
- Create: `crates/chkpt-plugin/README.md`

**Step 1: Create README.md**

Create `crates/chkpt-plugin/README.md`:

```markdown
# chkpt — Claude Code Plugin

Fast, content-addressable workspace checkpoints for Claude Code.

## Installation

```shell
# Add the marketplace
/plugin marketplace add <owner>/chkpt

# Install the plugin
/plugin install chkpt@chkpt-marketplace
```

## What's Included

### MCP Server (4 tools)

- `checkpoint_save` — Save a workspace snapshot
- `checkpoint_list` — List available checkpoints
- `checkpoint_restore` — Restore to a checkpoint (with dry-run)
- `checkpoint_delete` — Delete a checkpoint

### Skill (`/chkpt:chkpt`)

- **Proactive automation** — Suggests checkpoints before risky operations (large refactors, file deletion, dependency changes)
- **Direct operations** — Save, list, restore, delete via MCP tools
- **Store inspection** — Examine checkpoint internals and compare snapshots

## Requirements

- Node.js (for `npx` to run the MCP server)
- The MCP server binary is automatically downloaded via npm

## How It Works

chkpt saves workspace state to `~/.chkpt/stores/` without polluting Git. It uses:
- Content-addressed deduplication (BLAKE3 hashing)
- zstd compression for storage efficiency
- SQLite-based incremental change detection
```

**Step 2: Commit**

```bash
git add crates/chkpt-plugin/README.md
git commit -m "docs(plugin): add plugin README"
```

---

### Task 7: Local testing

**Step 1: Test plugin loading**

Run:
```bash
claude --plugin-dir ./crates/chkpt-plugin
```

Expected: Claude Code starts and loads the plugin. Check `/help` to see `/chkpt:chkpt` listed.

**Step 2: Test skill invocation**

In the Claude Code session:
```
/chkpt:chkpt
```

Expected: Skill loads and Claude responds as checkpoint assistant.

**Step 3: Test MCP tools**

In the Claude Code session, ask Claude to save a checkpoint. Expected: Claude uses `checkpoint_save` MCP tool.

**Step 4: Validate plugin structure**

Run:
```bash
claude plugin validate ./crates/chkpt-plugin
```

Expected: No errors.

**Step 5: Validate marketplace**

Run:
```bash
claude plugin validate .
```

Expected: Marketplace validates with chkpt plugin listed.

---

### Task 8: Final commit

**Step 1: Verify all files are committed**

Run `git status` — should be clean.

**Step 2: Create a summary commit if needed**

If any uncommitted changes remain:
```bash
git add -A
git commit -m "feat: complete chkpt plugin and marketplace setup"
```
