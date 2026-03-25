# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

chkpt is a fast, content-addressable checkpoint system for saving and restoring workspace snapshots without touching Git. It uses BLAKE3 hashing for deduplication and zstd compression, with an SQLite index for incremental saves.

This repository is a Cargo workspace.

## Repository Layout

```
crates/
  chkpt-core/        — Core library: scanner, store (blob/tree/pack/snapshot), index, ops, attachments
  chkpt-cli/         — CLI binary (clap + dialoguer)
  chkpt-mcp/         — MCP server (rmcp, stdio transport)
  chkpt-napi/        — Node.js native bindings (NAPI)
  chkpt-plugin/      — Claude Code plugin (MCP tools + /chkpt skill)
```

## Commands

```bash
cargo build                          # Build all crates
cargo test --workspace               # Run all Rust tests
cargo fmt --all                      # Format code
cargo clippy --workspace             # Lint
cargo run -p chkpt-cli -- <args>     # Run CLI (e.g., save -m "msg", list, restore, delete)
cargo run -p chkpt-mcp               # Run MCP server
```

Node.js bindings (in `crates/chkpt-napi`):

```bash
pnpm install && pnpm build && pnpm test
```

Run a single test:

```bash
cargo test -p chkpt-core --test save_test
```

## Architecture

> See [ARCHITECTURE.md](ARCHITECTURE.md) for detailed architecture documentation including diagrams, data flows, storage layout, and module organization.
>
> ARCHITECTURE.md is large. Do NOT read the entire file. Instead, use a **subagent(haiku)** to read and summarize only the relevant section. Example:
> ```
> Agent(model: "haiku", prompt: "Read ARCHITECTURE.md and summarize the Store Modules section. Focus on ...")
> ```

## Code Style

- **Rust edition**: 2021
- **Error handling**: `thiserror` for libraries, `anyhow` for binaries
- **Async**: tokio
- **Serialization**: serde + bincode (trees), serde_json (snapshots, config)
- **Formatting**: `cargo fmt --all` (default rustfmt settings)
- **Commit convention**: Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`, `refactor:`)

## Pre-commit Checklist

Before committing, run these checks and fix any failures:

1. `cargo fmt --all` — format code
2. `cargo clippy --workspace` — no warnings
3. `cargo test --workspace` — all tests pass
