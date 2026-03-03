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
