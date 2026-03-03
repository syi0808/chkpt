#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
NAPI_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
NPM_DIR="$NAPI_DIR/npm"

DRY_RUN="${1:-}"

if [[ "$DRY_RUN" == "--dry-run" ]]; then
  PUBLISH_FLAGS="--dry-run --access public"
  echo "=== DRY RUN: npm publish ==="
else
  PUBLISH_FLAGS="--access public"
  echo "=== Publishing to npm ==="
fi

echo ""

# 1. Publish platform packages first (they must exist before main package)
PLATFORMS=(
  darwin-arm64
  darwin-x64
  linux-arm64-gnu
  linux-x64-gnu
  win32-x64-msvc
)

for platform in "${PLATFORMS[@]}"; do
  PKG_DIR="$NPM_DIR/$platform"

  # Verify artifacts exist
  if ! ls "$PKG_DIR"/*.node 1>/dev/null 2>&1; then
    echo "ERROR: No .node file found in $PKG_DIR"
    echo "       Run scripts/build-all.sh first."
    exit 1
  fi

  echo "Publishing @chkpt/platform-${platform}..."
  cd "$PKG_DIR"
  npm publish $PUBLISH_FLAGS
  echo ""
done

# 2. Publish main package
echo "Publishing chkpt (main package)..."
cd "$NAPI_DIR"
npm publish $PUBLISH_FLAGS
echo ""

# 3. Publish @chkpt/mcp package
MCP_NPM_DIR="$(cd "$NAPI_DIR/../chkpt-mcp-npm" && pwd)"
echo "Publishing @chkpt/mcp..."
cd "$MCP_NPM_DIR"
npm publish $PUBLISH_FLAGS
echo ""

echo "=== Done ==="
