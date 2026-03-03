# chkpt

Save and restore your entire workspace without touching Git.

One `chkpt save` before a big refactor, dependency update, or AI agent run — and you can roll back anytime.

## Why chkpt?

| Current approach | Problem |
|-----------------|---------|
| `git stash` | Tangles with staged state, misses untracked files |
| Temporary branches | Pollutes branch list, needs cleanup |
| Manual copies | Slow, error-prone, wastes disk space |

**chkpt takes a different approach:**

- **BLAKE3 hashing** — Content-addressed deduplication, identical files stored once
- **zstd compression** — Minimizes storage footprint
- **SQLite index** — Detects only changed files for incremental saves
- **Atomic restore** — Workspace stays intact even if restore fails midway

## Installation

### Claude Code Plugin (recommended)

```shell
# Add the marketplace
/plugin marketplace add syi0808/chkpt

# Install the plugin
/plugin install chkpt@chkpt-marketplace
```

This activates 4 MCP tools and the automation skill (`/chkpt:chkpt`).

### CLI

```shell
cargo install chkpt-cli
```

### MCP Server (standalone)

```shell
npx @chkpt/mcp
```

## Usage

### CLI

```shell
# Save current workspace
chkpt save -m "before refactor"

# List checkpoints
chkpt list

# Restore to latest checkpoint
chkpt restore latest

# Preview restore without applying
chkpt restore <id> --dry-run

# Delete a checkpoint
chkpt delete <id>
```

### Claude Code MCP Tools

| Tool | Description |
|------|-------------|
| `checkpoint_save` | Save a workspace snapshot |
| `checkpoint_list` | List available checkpoints |
| `checkpoint_restore` | Restore to a checkpoint (dry-run supported) |
| `checkpoint_delete` | Delete a checkpoint |

### Automation Skill (`/chkpt:chkpt`)

Invoke `/chkpt:chkpt` in Claude Code to:

- Get automatic checkpoint suggestions before risky operations (large refactors, file deletion, dependency changes)
- Save, restore, and delete checkpoints conversationally
- Compare differences between snapshots

## How It Works

```
Workspace                      ~/.chkpt/stores/
┌──────────────┐              ┌──────────────────┐
│  src/        │   save →     │  blobs/  (zstd)   │
│  tests/      │              │  trees/  (bincode) │
│  Cargo.toml  │   ← restore │  snapshots/ (meta) │
└──────────────┘              │  index.db (SQLite) │
                              └──────────────────┘
```

1. **Scan** — Walk files according to `.chkptignore` rules
2. **Hash** — Generate BLAKE3 content hash for each file
3. **Deduplicate** — Skip content already in the store
4. **Compress & store** — Write new content with zstd compression
5. **Record snapshot** — Save tree structure and metadata to SQLite

## Optional Attachments

```shell
# Include dependencies (node_modules, etc.)
chkpt save --with-deps

# Include Git history
chkpt save --with-git
```

## Project Structure

```
crates/
├── chkpt-core/      # Core library (scanner, store, index, ops)
├── chkpt-cli/       # CLI binary
├── chkpt-mcp/       # MCP stdio server
├── chkpt-napi/      # Node.js native bindings (NAPI)
├── chkpt-mcp-npm/   # @chkpt/mcp npm package
└── chkpt-plugin/    # Claude Code plugin
```

## Requirements

- **CLI**: Rust toolchain
- **Plugin / MCP**: Node.js (to run via npx)

## License

Apache-2.0
