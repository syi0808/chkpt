# chkpt

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)

> Save and restore your entire workspace without touching Git.

chkpt is a fast, content-addressable checkpoint system. One `chkpt save` before a big refactor, dependency update, or AI agent run, and you can roll back anytime. Unlike `git stash` or temporary branches, chkpt captures everything (including untracked files), deduplicates content with XXH3-128 hashing, and compresses with LZ4. Snapshots are fast and take up little disk space.

## Features

- **Content-addressed deduplication** via XXH3-128 hashing. Identical files are stored only once.
- **LZ4 compression** for every blob, keeping storage small.
- **Incremental saves** with a SQLite index that detects only changed files.
- **Catalog-backed metadata** for fast snapshot lookup, listing, and restore planning.
- **Atomic restore** that keeps your workspace intact if something fails midway.
- **Optional dependency scanning** when you want to include `node_modules`, `.venv`, and similar directories.
- **Optional pack chunking** when you need pack data stored as smaller part files.
- **Multiple interfaces**: CLI, Node.js API, MCP server, and Claude Code plugin.

## Performance

Benchmarked on MacBook Pro (Apple M2 Pro, 16 GB RAM, APFS SSD). Release build, median of 3 runs.

| Project | Files | Size | Cold Save | Incr. Save | Restore | Storage | Ratio | Incr. Storage (new data) |
|---------|------:|-----:|----------:|-----------:|--------:|--------:|------:|------------------------:|
| [React](https://github.com/facebook/react) | 6,879 | 34.4 MB | 0.26s | 0.05s | 0.03s | 19.5 MB | 1.7x | +2.5 MB (7.4 KB) |
| [Rust](https://github.com/rust-lang/rust) | 58,760 | 195.9 MB | 2.04s | 0.33s | 0.18s | 96.0 MB | 2.0x | +15.8 MB (2.2 KB) |
| [Linux kernel](https://github.com/torvalds/linux) | 92,923 | 1.4 GB | 2.95s | 0.47s | 0.24s | 503.3 MB | 2.9x | +22.9 MB (25 KB) |

> **Cold Save** = first checkpoint with empty store. **Incr. Save** = re-save after modifying 5 files.
> **Storage** = total `.chkpt` store size after cold save (LZ4-compressed, content-deduplicated). **Ratio** = original size / storage size.
> **Incr. Storage** = total store growth per incremental save (includes metadata). Parenthesized value = actual new file content stored.

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

### Optional Dependency Inclusion

```bash
# Include dependency directories (node_modules, .venv, etc.)
chkpt save --include-deps
```

### Optional Pack Chunking

```bash
# Split newly generated pack data into 48 MiB part files
chkpt save --pack-chunk-bytes 50331648
```

Pack chunking is opt-in. When enabled, chkpt writes `pack-<hash>.dat.parts.json` plus `pack-<hash>.dat.part-000000`, `pack-<hash>.dat.part-000001`, and so on. Restore reads those parts directly; no manual join step is required.

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
│  src/        │   save →     │  trees/           │
│  tests/      │              │  packs/           │
│  Cargo.toml  │   ← restore │  catalog.sqlite   │
└──────────────┘              │  index.bin        │
                              └──────────────────┘
```

1. **Scan**: walk files according to `.chkptignore` rules
2. **Hash**: generate an XXH3-128 content hash for each file
3. **Deduplicate**: skip content already in the store
4. **Compress & store**: write new content with LZ4 compression
5. **Record snapshot**: persist metadata and manifest in `catalog.sqlite`

By default, each pack's compressed blob data is stored in `packs/pack-<hash>.dat` with a sibling `.idx` file. With `--pack-chunk-bytes`, the `.dat` payload is stored as contiguous part files plus a small parts manifest instead.

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
chkpt deduplicates at the file level using XXH3-128 content hashing and compresses blobs with LZ4. If most files haven't changed between saves, the incremental cost is minimal.

**Does chkpt replace Git?**
No. chkpt is for quick, local snapshots, not version control. Think of it as a "save game" for your workspace. Use Git for collaboration and history; use chkpt for instant rollback points.

**What files does chkpt ignore?**
By default, chkpt skips `.git/`, `node_modules/`, and other common build artifacts. You can customize this with a `.chkptignore` file (same syntax as `.gitignore`).

**Does it work on Windows?**
Yes. Pre-built binaries are available for macOS (arm64, x64), Linux (arm64, x64), and Windows (x64).

## Contributing

Contributions are welcome. Please read the [Contributing Guide](CONTRIBUTING.md) before submitting a pull request.

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](LICENSE) file for details.

## Author

**Yein Sung** · [GitHub](https://github.com/syi0808)
