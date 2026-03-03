#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
NAPI_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT_DIR="$(cd "$NAPI_DIR/../.." && pwd)"
NPM_DIR="$NAPI_DIR/npm"

cd "$ROOT_DIR"

# Target format: "rust_triple:npm_platform:build_tool:lib_ext:bin_ext"
#   build_tool: "cargo" for macOS (native), "cross" for Linux (Docker),
#               "cargo xwin" for Windows MSVC (cross-compile via cargo-xwin)
#   lib_ext: shared library extension produced by rustc
#   bin_ext: binary extension ("" for unix, ".exe" for windows)
TARGETS=(
  "aarch64-apple-darwin:darwin-arm64:cargo:.dylib:"
  "x86_64-apple-darwin:darwin-x64:cargo:.dylib:"
  "aarch64-unknown-linux-gnu:linux-arm64-gnu:cross:.so:"
  "x86_64-unknown-linux-gnu:linux-x64-gnu:cross:.so:"
  "x86_64-pc-windows-msvc:win32-x64-msvc:cargo xwin:.dll:.exe"
)

# Allow building a single target: ./build-all.sh darwin-arm64
FILTER="${1:-}"

echo "=== chkpt cross-compilation build ==="
echo ""

for entry in "${TARGETS[@]}"; do
  IFS=: read -r triple platform tool lib_ext bin_ext <<< "$entry"

  if [[ -n "$FILTER" && "$platform" != "$FILTER" ]]; then
    continue
  fi

  echo "--- Building $platform ($triple) via $tool ---"

  # 1. Build N-API native module (cdylib)
  echo "  [1/3] Building chkpt-napi..."
  $tool build --release --target "$triple" -p chkpt-napi

  # 2. Build CLI binary
  echo "  [2/3] Building chkpt-cli..."
  $tool build --release --target "$triple" -p chkpt-cli

  # 3. Build MCP server binary
  echo "  [3/3] Building chkpt-mcp..."
  $tool build --release --target "$triple" -p chkpt-mcp

  # 3. Copy artifacts to npm/{platform}/
  TARGET_DIR="$NPM_DIR/$platform"
  mkdir -p "$TARGET_DIR"

  # .node file: rename from libchkpt_napi{.dylib|.so|.dll} → chkpt.{platform}.node
  LIB_NAME="libchkpt_napi${lib_ext}"
  # Windows uses chkpt_napi.dll (no "lib" prefix)
  if [[ "$lib_ext" == ".dll" ]]; then
    LIB_NAME="chkpt_napi${lib_ext}"
  fi
  NODE_FILE="chkpt.${platform}.node"

  cp "target/${triple}/release/${LIB_NAME}" "${TARGET_DIR}/${NODE_FILE}"
  echo "  -> ${NODE_FILE}"

  # CLI binary
  BIN_NAME="chkpt${bin_ext}"
  cp "target/${triple}/release/${BIN_NAME}" "${TARGET_DIR}/${BIN_NAME}"
  echo "  -> ${BIN_NAME}"

  # MCP server binary
  MCP_BIN_NAME="chkpt-mcp${bin_ext}"
  cp "target/${triple}/release/${MCP_BIN_NAME}" "${TARGET_DIR}/${MCP_BIN_NAME}"
  echo "  -> ${MCP_BIN_NAME}"

  echo ""
done

# 4. Generate index.js and index.d.ts via napi build (local platform only)
echo "--- Generating index.js and index.d.ts ---"
cd "$NAPI_DIR"
pnpm run build
echo ""

echo "=== Build complete ==="
echo ""
echo "Artifacts:"
for dir in "$NPM_DIR"/*/; do
  platform=$(basename "$dir")
  echo "  $platform/: $(ls "$dir" 2>/dev/null | tr '\n' ' ')"
done
