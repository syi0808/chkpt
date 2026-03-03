# chkpt Plugin & Marketplace Design

## Overview

Package chkpt as a Claude Code plugin with MCP server + Skill, and host a GitHub-based marketplace for distribution.

## Architecture: Mono Plugin

Single plugin containing all components:
- **MCP Server**: 4 tools (checkpoint_save/list/restore/delete) via `npx @chkpt/mcp`
- **Skill**: Proactive automation + MCP tool guidance
- No Hooks (Skill's proactive automation covers this)

## Plugin Directory Structure

```
crates/chkpt-plugin/
├── .claude-plugin/
│   └── plugin.json           # Plugin manifest
├── skills/
│   └── chkpt/
│       ├── SKILL.md           # Adapted from chkpt-cli skill
│       └── references/
│           ├── cli-commands.md
│           ├── store-layout.md
│           └── automation-patterns.md
├── .mcp.json                  # MCP server configuration
└── README.md
```

## Plugin Manifest (plugin.json)

```json
{
  "name": "chkpt",
  "description": "Fast, content-addressable workspace checkpoints. Save and restore workspace snapshots without Git pollution.",
  "version": "0.1.2",
  "author": { "name": "chkpt" },
  "homepage": "https://github.com/<owner>/chkpt",
  "repository": "https://github.com/<owner>/chkpt",
  "license": "MIT",
  "keywords": ["checkpoint", "snapshot", "backup", "workspace"]
}
```

## MCP Configuration (.mcp.json)

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

Uses existing `@chkpt/mcp` npm package which resolves platform-specific binaries from `@chkpt/platform-*` packages.

## Skill Design

Based on existing `crates/chkpt-cli/skills/chkpt/SKILL.md` with these changes:

1. **MCP tools first**: Use `checkpoint_save`, `checkpoint_list`, `checkpoint_restore`, `checkpoint_delete` MCP tools instead of CLI
2. **CLI as fallback**: Keep CLI documentation for cases where MCP tools aren't available
3. **Proactive automation preserved**: Continue detecting risky operations (refactoring, file deletion, dependency changes) and suggesting checkpoints
4. **references/ copied as-is**: cli-commands.md, store-layout.md, automation-patterns.md

Plugin namespace: `/chkpt:chkpt`

## Marketplace Design

Marketplace hosted in the chkpt repository itself.

### Repository additions

```
chkpt/                          (project root)
├── .claude-plugin/
│   └── marketplace.json        # Marketplace catalog
├── crates/
│   └── chkpt-plugin/           # Plugin directory
│       └── ...
```

### marketplace.json

```json
{
  "name": "chkpt-marketplace",
  "owner": { "name": "chkpt" },
  "metadata": {
    "description": "Workspace checkpoint tools for Claude Code"
  },
  "plugins": [
    {
      "name": "chkpt",
      "source": "./crates/chkpt-plugin",
      "description": "Fast workspace checkpoints - save/restore snapshots without Git pollution",
      "version": "0.1.2",
      "category": "developer-tools",
      "tags": ["checkpoint", "snapshot", "backup", "workspace"],
      "keywords": ["checkpoint", "snapshot", "backup"]
    }
  ]
}
```

### User Installation

```shell
# Add marketplace
/plugin marketplace add <owner>/chkpt

# Install plugin
/plugin install chkpt@chkpt-marketplace

# Use
/chkpt:chkpt                    # Invoke skill
# MCP tools available automatically (checkpoint_save, etc.)
```

## Dependencies

- Existing `@chkpt/mcp` npm package (v0.1.2) for MCP server binary
- Existing `@chkpt/platform-*` packages for cross-platform binaries
- No new build infrastructure needed

## Version Strategy

Plugin version tracks the main `@chkpt/mcp` package version. When updating:
1. Update `plugin.json` version
2. Update `marketplace.json` version
3. Users refresh with `/plugin marketplace update`
