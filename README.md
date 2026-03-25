# chkpt

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

> Save and restore your entire workspace without touching Git.

chkpt is a fast, content-addressable checkpoint system that saves and restores entire workspace snapshots without touching Git.

One `chkpt save` before a big refactor, dependency update, or AI agent run — and you can roll back anytime. Unlike `git stash` or temporary branches, chkpt captures everything (including untracked files), deduplicates content with BLAKE3 hashing, and compresses with zstd — so snapshots are fast and storage-efficient.

## Features

- **Content-Addressed Deduplication** — BLAKE3 hashing ensures identical files are stored only once
- **zstd Compression** — Minimizes storage footprint for every blob
- **Incremental Saves** — SQLite index detects only changed files, skipping unchanged content
- **Atomic Restore** — Workspace stays intact even if a restore fails midway
- **Dependency Attachments** — Optionally include `node_modules` or `.git` history in checkpoints
- **Multiple Interfaces** — CLI, Node.js API, MCP server, and Claude Code plugin

## Getting Started

### Requirements

- **CLI (Rust)**: Rust toolchain (for `cargo install`)
- **CLI / MCP (Node.js)**: Node.js 18 or later (for `npx`)

### Install

#### CLI via Cargo

```bash
cargo install chkpt-cli
```

#### CLI via npx

```bash
npx chkpt
```

#### MCP Server

```bash
npx @chkpt/mcp
```

#### Claude Code Plugin (recommended)

```bash
# Add the marketplace
/plugin marketplace add syi0808/chkpt

# Install the plugin
/plugin install chkpt@chkpt-marketplace
```

This activates 4 MCP tools and the automation skill (`/chkpt:chkpt`).

## Usage

### CLI

```bash
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

### Optional Attachments

```bash
# Include dependencies (node_modules, etc.)
chkpt save --with-deps

# Include Git history
chkpt save --with-git
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
4. **Compress & Store** — Write new content with zstd compression
5. **Record Snapshot** — Save tree structure and metadata

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

## FAQ

**How much disk space do checkpoints use?**
chkpt deduplicates at the file level using BLAKE3 content hashing and compresses blobs with zstd. If most files haven't changed between saves, the incremental cost is minimal.

**Does chkpt replace Git?**
No. chkpt is designed for quick, local snapshots — not version control. Think of it as a "save game" for your workspace. Use Git for collaboration and history; use chkpt for instant rollback points.

**What files does chkpt ignore?**
By default, chkpt skips `.git/`, `node_modules/`, and other common build artifacts. You can customize this with a `.chkptignore` file (same syntax as `.gitignore`).

**Are there size limits?**
Default guardrails allow up to 2 GB total, 100,000 files, and 100 MB per file. These are configurable.

**Does it work on Windows?**
Yes. Pre-built binaries are available for macOS (arm64, x64), Linux (arm64, x64), and Windows (x64).

## Contributing

Contributions are welcome. Please read the [Contributing Guide](CONTRIBUTING.md) before submitting a pull request.

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

## Author

**Yein Sung** — [GitHub](https://github.com/syi0808)
