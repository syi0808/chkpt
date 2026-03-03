# chkpt-napi

Node.js N-API bindings for the filesystem checkpoint engine.

## Build

### Current platform only

```bash
pnpm build
```

Builds a `.node` binary for the host platform only.

### Cross-compile all targets

```bash
pnpm build:all
```

Builds N-API modules and CLI binaries for all 5 platforms.
To build a specific platform:

```bash
bash scripts/build-all.sh darwin-arm64
bash scripts/build-all.sh win32-x64-msvc
```

## Cross-compilation prerequisites

The following tools are required to run the full cross-build (`build:all`).

### 1. Rust targets

```bash
rustup target add aarch64-apple-darwin
rustup target add x86_64-apple-darwin
rustup target add aarch64-unknown-linux-gnu
rustup target add x86_64-unknown-linux-gnu
rustup target add x86_64-pc-windows-msvc
```

### 2. Linux — cross (Docker-based)

```bash
cargo install cross
```

Requires Docker to be installed and running.

### 3. Windows MSVC — cargo-xwin

```bash
# Install cargo-xwin
cargo install cargo-xwin

# Install LLVM (provides clang-cl)
# macOS
brew install llvm

# Ubuntu/Debian
sudo apt install llvm clang lld
```

On the first build, `cargo-xwin` automatically downloads the MSVC CRT headers and Windows SDK libraries.

### Summary

| Target | Build tool | Dependencies |
|--------|-----------|--------------|
| `aarch64-apple-darwin` | `cargo` | (native) |
| `x86_64-apple-darwin` | `cargo` | (native) |
| `aarch64-unknown-linux-gnu` | `cross` | Docker |
| `x86_64-unknown-linux-gnu` | `cross` | Docker |
| `x86_64-pc-windows-msvc` | `cargo xwin` | LLVM (clang-cl) |

## Test

```bash
pnpm test
```

## Publish

```bash
# dry-run
bash scripts/publish.sh --dry-run

# publish
bash scripts/publish.sh
```

Publishes platform-specific npm packages first, then the main package.
