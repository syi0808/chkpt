# @chkpt/mcp npm Package + Interactive Restore

## Overview

Two features:
1. **`@chkpt/mcp` npm package** — run the MCP server via `npx @chkpt/mcp`
2. **Interactive restore** — `chkpt restore` (no ID) shows an interactive checkpoint selector

## 1. @chkpt/mcp npm Package

### Structure

New directory `crates/chkpt-mcp-npm/`:

```
crates/chkpt-mcp-npm/
├── package.json    # name: "@chkpt/mcp", bin: { "chkpt-mcp": "./cli.mjs" }
├── cli.mjs         # Wrapper script that resolves and executes chkpt-mcp binary
```

### How it works

- `cli.mjs` follows the same pattern as `chkpt-napi/cli.mjs`: resolves the native binary from `@chkpt/platform-*` packages by platform/arch
- The `chkpt-mcp` Rust binary must be bundled into each `@chkpt/platform-*` package alongside the existing `chkpt` binary
- `npx @chkpt/mcp` starts the MCP stdio server

### package.json

```json
{
  "name": "@chkpt/mcp",
  "version": "0.1.2",
  "description": "MCP server for chkpt checkpoint engine",
  "bin": { "chkpt-mcp": "./cli.mjs" },
  "files": ["cli.mjs"],
  "optionalDependencies": {
    "@chkpt/platform-darwin-arm64": "0.1.2",
    "@chkpt/platform-darwin-x64": "0.1.2",
    "@chkpt/platform-linux-arm64-gnu": "0.1.2",
    "@chkpt/platform-linux-x64-gnu": "0.1.2",
    "@chkpt/platform-win32-x64-msvc": "0.1.2"
  }
}
```

### Build changes

The cross-compilation build scripts must compile `chkpt-mcp` for each target and include the binary in each platform package.

## 2. Interactive Restore (dialoguer)

### CLI changes in chkpt-cli

- Change `Restore.id` from `String` to `Option<String>`
- Add `dialoguer` dependency
- When `id` is `None`, list checkpoints and show interactive select

### Interactive UI

```
? Select checkpoint to restore:
> 01935a2b  2026-03-04 15:30:12  42 files  "add auth feature"
  01935a1c  2026-03-04 14:20:00  38 files  "refactor db"
  01935a0f  2026-03-04 13:10:45  35 files  "initial setup"
```

Arrow keys to navigate, Enter to confirm. If no checkpoints exist, print a message and exit.

### Behavior

- `chkpt restore <id>` — existing behavior, restore immediately
- `chkpt restore <id> --dry-run` — existing behavior, dry run
- `chkpt restore` — interactive checkpoint selector, then restore
- `chkpt restore --dry-run` — interactive selector, then dry run
