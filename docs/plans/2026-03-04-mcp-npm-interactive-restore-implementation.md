# @chkpt/mcp npm Package + Interactive Restore — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create a `@chkpt/mcp` npm package that runs the MCP server via `npx @chkpt/mcp`, and add interactive checkpoint selection to `chkpt restore` (no ID) using `dialoguer`.

**Architecture:** Two independent workstreams. (1) New `crates/chkpt-mcp-npm/` directory with `package.json` and `cli.mjs` wrapper that resolves `chkpt-mcp` binary from `@chkpt/platform-*` packages; build scripts updated to compile and bundle `chkpt-mcp` binary. (2) `chkpt-cli` modified to accept optional restore ID, showing interactive `dialoguer::Select` when omitted.

**Tech Stack:** Node.js (ESM wrapper), Rust (dialoguer crate), existing build tooling (cargo/cross/cargo-xwin)

---

### Task 1: Create `@chkpt/mcp` package — `package.json`

**Files:**
- Create: `crates/chkpt-mcp-npm/package.json`

**Step 1: Create the package.json**

```json
{
  "name": "@chkpt/mcp",
  "version": "0.1.2",
  "description": "MCP server for chkpt – run via npx @chkpt/mcp",
  "bin": {
    "chkpt-mcp": "./cli.mjs"
  },
  "files": [
    "cli.mjs"
  ],
  "license": "Apache-2.0",
  "engines": {
    "node": ">= 18"
  },
  "optionalDependencies": {
    "@chkpt/platform-darwin-arm64": "0.1.2",
    "@chkpt/platform-darwin-x64": "0.1.2",
    "@chkpt/platform-linux-arm64-gnu": "0.1.2",
    "@chkpt/platform-linux-x64-gnu": "0.1.2",
    "@chkpt/platform-win32-x64-msvc": "0.1.2"
  }
}
```

**Step 2: Commit**

```bash
git add crates/chkpt-mcp-npm/package.json
git commit -m "feat(mcp-npm): add @chkpt/mcp package.json"
```

---

### Task 2: Create `@chkpt/mcp` package — `cli.mjs` wrapper

**Files:**
- Create: `crates/chkpt-mcp-npm/cli.mjs`
- Reference: `crates/chkpt-napi/cli.mjs` (same pattern, different binary name)

**Step 1: Create the cli.mjs wrapper**

The wrapper follows the exact same pattern as `crates/chkpt-napi/cli.mjs` but resolves `chkpt-mcp` / `chkpt-mcp.exe` instead of `chkpt` / `chkpt.exe`.

```javascript
#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync } from "node:fs";

function getBinaryName() {
  return process.platform === "win32" ? "chkpt-mcp.exe" : "chkpt-mcp";
}

function getBinaryPath() {
  const platform = process.platform;
  const arch = process.arch;
  const binaryName = getBinaryName();

  const triples = {
    "darwin-arm64": "darwin-arm64",
    "darwin-x64": "darwin-x64",
    "linux-arm64": "linux-arm64-gnu",
    "linux-x64": "linux-x64-gnu",
    "win32-x64": "win32-x64-msvc",
  };

  const key = `${platform}-${arch}`;
  const triple = triples[key];

  if (!triple) {
    throw new Error(
      `Unsupported platform: ${platform}-${arch}. ` +
        `Supported: ${Object.keys(triples).join(", ")}`,
    );
  }

  // Try platform-specific npm package
  const packageName = `@chkpt/platform-${triple}`;
  try {
    const pkgDir = dirname(
      fileURLToPath(import.meta.resolve(`${packageName}/package.json`)),
    );
    const binaryPath = join(pkgDir, binaryName);
    if (existsSync(binaryPath)) {
      return binaryPath;
    }
  } catch {
    // Package not installed, fall through
  }

  // Fallback: binary next to this script (local dev)
  const localBinary = join(dirname(fileURLToPath(import.meta.url)), binaryName);
  if (existsSync(localBinary)) {
    return localBinary;
  }

  throw new Error(
    `Could not find chkpt-mcp binary. Install the platform package: ${packageName}`,
  );
}

try {
  execFileSync(getBinaryPath(), process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });
} catch (error) {
  if (error.status != null) {
    process.exit(error.status);
  }
  console.error(error.message);
  process.exit(1);
}
```

**Step 2: Commit**

```bash
git add crates/chkpt-mcp-npm/cli.mjs
git commit -m "feat(mcp-npm): add cli.mjs binary wrapper"
```

---

### Task 3: Update platform packages to include `chkpt-mcp` binary

**Files:**
- Modify: `crates/chkpt-napi/npm/darwin-arm64/package.json`
- Modify: `crates/chkpt-napi/npm/darwin-x64/package.json`
- Modify: `crates/chkpt-napi/npm/linux-arm64-gnu/package.json`
- Modify: `crates/chkpt-napi/npm/linux-x64-gnu/package.json`
- Modify: `crates/chkpt-napi/npm/win32-x64-msvc/package.json`
- Modify: `crates/chkpt-napi/npm/.gitignore`

**Step 1: Add `chkpt-mcp` to each platform package's `files` array**

For unix platforms (darwin-arm64, darwin-x64, linux-arm64-gnu, linux-x64-gnu), add `"chkpt-mcp"` to the `files` array.

Example for `darwin-arm64/package.json`:
```json
{
  "name": "@chkpt/platform-darwin-arm64",
  "version": "0.1.2",
  "os": ["darwin"],
  "cpu": ["arm64"],
  "main": "chkpt.darwin-arm64.node",
  "files": [
    "chkpt.darwin-arm64.node",
    "chkpt",
    "chkpt-mcp"
  ],
  "description": "chkpt native bindings and CLI for darwin-arm64",
  "license": "MIT",
  "engines": { "node": ">= 18" }
}
```

For win32-x64-msvc, add `"chkpt-mcp.exe"`:
```json
{
  "files": [
    "chkpt.win32-x64-msvc.node",
    "chkpt.exe",
    "chkpt-mcp.exe"
  ]
}
```

**Step 2: Add `chkpt-mcp` and `chkpt-mcp.exe` to `crates/chkpt-napi/npm/.gitignore`**

```
*.node
chkpt
chkpt.exe
chkpt-mcp
chkpt-mcp.exe
```

**Step 3: Commit**

```bash
git add crates/chkpt-napi/npm/
git commit -m "feat(platform): add chkpt-mcp binary to platform packages"
```

---

### Task 4: Update build script to compile and copy `chkpt-mcp`

**Files:**
- Modify: `crates/chkpt-napi/scripts/build-all.sh`

**Step 1: Add chkpt-mcp build step**

In `build-all.sh`, after the existing `[2/2] Building chkpt-cli...` block, add a `[3/3] Building chkpt-mcp...` step. Update step labels from `[1/2]` → `[1/3]` and `[2/2]` → `[2/3]`.

After the CLI binary copy block, add a copy block for the MCP binary:

```bash
  # 1. Build N-API native module (cdylib)
  echo "  [1/3] Building chkpt-napi..."
  $tool build --release --target "$triple" -p chkpt-napi

  # 2. Build CLI binary
  echo "  [2/3] Building chkpt-cli..."
  $tool build --release --target "$triple" -p chkpt-cli

  # 3. Build MCP server binary
  echo "  [3/3] Building chkpt-mcp..."
  $tool build --release --target "$triple" -p chkpt-mcp

  # ... (existing artifact copy) ...

  # MCP server binary
  MCP_BIN_NAME="chkpt-mcp${bin_ext}"
  cp "target/${triple}/release/${MCP_BIN_NAME}" "${TARGET_DIR}/${MCP_BIN_NAME}"
  echo "  -> ${MCP_BIN_NAME}"
```

**Step 2: Commit**

```bash
git add crates/chkpt-napi/scripts/build-all.sh
git commit -m "feat(build): compile chkpt-mcp binary in build-all.sh"
```

---

### Task 5: Update publish script to publish `@chkpt/mcp`

**Files:**
- Modify: `crates/chkpt-napi/scripts/publish.sh`

**Step 1: Add `@chkpt/mcp` publish step after main package**

After the existing "Publish main package" block, add:

```bash
# 3. Publish @chkpt/mcp package
MCP_NPM_DIR="$(cd "$NAPI_DIR/../chkpt-mcp-npm" && pwd)"
echo "Publishing @chkpt/mcp..."
cd "$MCP_NPM_DIR"
npm publish $PUBLISH_FLAGS
echo ""
```

**Step 2: Commit**

```bash
git add crates/chkpt-napi/scripts/publish.sh
git commit -m "feat(publish): add @chkpt/mcp to publish script"
```

---

### Task 6: Add `dialoguer` dependency to `chkpt-cli`

**Files:**
- Modify: `crates/chkpt-cli/Cargo.toml`
- Modify: `Cargo.toml` (workspace)

**Step 1: Add dialoguer to workspace dependencies**

In root `Cargo.toml`, add to `[workspace.dependencies]`:

```toml
dialoguer = "0.11"
```

**Step 2: Add dialoguer to chkpt-cli dependencies**

In `crates/chkpt-cli/Cargo.toml`, add to `[dependencies]`:

```toml
dialoguer = { workspace = true }
chrono = { workspace = true }
```

(`chrono` is needed to format the `created_at` timestamp in the selector.)

**Step 3: Verify it compiles**

Run: `cargo check -p chkpt-cli`
Expected: Compiles without errors.

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/chkpt-cli/Cargo.toml
git commit -m "feat(cli): add dialoguer and chrono dependencies"
```

---

### Task 7: Implement interactive restore in `chkpt-cli`

**Files:**
- Modify: `crates/chkpt-cli/src/main.rs`

**Step 1: Modify the Restore command to accept optional ID**

Change the `Restore` variant's `id` field from `String` to `Option<String>`:

```rust
use clap::{Parser, Subcommand};
use anyhow::{Result, bail};
use dialoguer::{Select, theme::ColorfulTheme};

#[derive(Parser)]
#[command(name = "chkpt", about = "Filesystem checkpoint engine")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Save a checkpoint of the current workspace
    Save {
        /// Optional message for the checkpoint
        #[arg(short, long)]
        message: Option<String>,
    },
    /// List all checkpoints
    List {
        /// Maximum number of checkpoints to show
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Restore workspace to a checkpoint
    Restore {
        /// Snapshot ID or "latest" (interactive selection if omitted)
        id: Option<String>,
        /// Show what would change without modifying files
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete a checkpoint and run garbage collection
    Delete {
        /// Snapshot ID to delete
        id: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let workspace = std::env::current_dir()?;

    match cli.command {
        Commands::Save { message } => {
            let opts = chkpt_core::ops::save::SaveOptions { message };
            let result = chkpt_core::ops::save::save(&workspace, opts)?;
            println!("Checkpoint saved: {}", result.snapshot_id);
            println!(
                "  Files: {}, New objects: {}, Total bytes: {}",
                result.stats.total_files, result.stats.new_objects, result.stats.total_bytes
            );
        }
        Commands::List { limit } => {
            let snapshots = chkpt_core::ops::list::list(&workspace, limit)?;
            if snapshots.is_empty() {
                println!("No checkpoints found.");
            } else {
                println!(
                    "{:<10} {:<22} {:<8} {}",
                    "ID", "Created", "Files", "Message"
                );
                println!("{}", "-".repeat(60));
                for snap in &snapshots {
                    let short_id = &snap.id[..8.min(snap.id.len())];
                    let msg = snap.message.as_deref().unwrap_or("");
                    println!(
                        "{:<10} {:<22} {:<8} {}",
                        short_id,
                        snap.created_at.format("%Y-%m-%d %H:%M:%S"),
                        snap.stats.total_files,
                        msg
                    );
                }
                println!("\n{} checkpoint(s)", snapshots.len());
            }
        }
        Commands::Restore { id, dry_run } => {
            let snapshot_id = match id {
                Some(id) => id,
                None => {
                    let snapshots = chkpt_core::ops::list::list(&workspace, None)?;
                    if snapshots.is_empty() {
                        bail!("No checkpoints found.");
                    }

                    let items: Vec<String> = snapshots
                        .iter()
                        .map(|s| {
                            let short_id = &s.id[..8.min(s.id.len())];
                            let msg = s.message.as_deref().unwrap_or("");
                            format!(
                                "{}  {}  {} files  {}",
                                short_id,
                                s.created_at.format("%Y-%m-%d %H:%M:%S"),
                                s.stats.total_files,
                                if msg.is_empty() {
                                    String::new()
                                } else {
                                    format!("\"{}\"", msg)
                                }
                            )
                        })
                        .collect();

                    let selection = Select::with_theme(&ColorfulTheme::default())
                        .with_prompt("Select checkpoint to restore")
                        .items(&items)
                        .default(0)
                        .interact()?;

                    snapshots[selection].id.clone()
                }
            };

            let opts = chkpt_core::ops::restore::RestoreOptions { dry_run };
            let result = chkpt_core::ops::restore::restore(&workspace, &snapshot_id, opts)?;
            if dry_run {
                println!("Dry run -- no changes made:");
            } else {
                println!("Restored to checkpoint {}:", result.snapshot_id);
            }
            println!(
                "  Added: {}, Changed: {}, Removed: {}, Unchanged: {}",
                result.files_added,
                result.files_changed,
                result.files_removed,
                result.files_unchanged
            );
        }
        Commands::Delete { id } => {
            chkpt_core::ops::delete::delete(&workspace, &id)?;
            println!("Checkpoint {} deleted.", id);
        }
    }

    Ok(())
}
```

**Step 2: Verify it compiles**

Run: `cargo check -p chkpt-cli`
Expected: Compiles without errors.

**Step 3: Build and test locally**

Run: `cargo build -p chkpt-cli`

Then test:
- `./target/debug/chkpt restore some-id` — should behave as before (fail with "snapshot not found" if ID doesn't exist)
- `./target/debug/chkpt restore` — should show interactive selector (or "No checkpoints found" if empty)

**Step 4: Commit**

```bash
git add crates/chkpt-cli/src/main.rs
git commit -m "feat(cli): interactive checkpoint selector for 'chkpt restore'"
```

---

### Task 8: Local build verification

**Files:** None (verification only)

**Step 1: Build for local platform**

Run: `cargo build --release -p chkpt-cli -p chkpt-mcp`
Expected: Both binaries compile successfully.

**Step 2: Verify chkpt restore interactive mode**

```bash
cd /tmp && mkdir test-chkpt && cd test-chkpt
echo "hello" > test.txt
/path/to/target/release/chkpt save -m "test checkpoint"
/path/to/target/release/chkpt restore
```

Expected: Interactive selector appears with the "test checkpoint" entry.

**Step 3: Verify chkpt restore <id> still works**

```bash
/path/to/target/release/chkpt list
# Copy the snapshot ID from output
/path/to/target/release/chkpt restore <copied-id>
```

Expected: Restores without showing interactive selector.

**Step 4: Verify chkpt-mcp starts**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"test","version":"0.1"},"protocolVersion":"2025-03-26"}}' | /path/to/target/release/chkpt-mcp
```

Expected: Responds with JSON-RPC initialization response.

**Step 5: Commit build artifacts (if needed)**

No code changes expected. If build errors are found, fix and commit.
