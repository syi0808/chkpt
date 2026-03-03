# chkpt — Claude Code Plugin

Fast, content-addressable workspace checkpoints for Claude Code.

## Installation

```shell
# Add the marketplace
/plugin marketplace add syi0808/chkpt

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
