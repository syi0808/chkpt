# Contributing to chkpt

Thank you for your interest in contributing to chkpt. This guide explains how to report issues, suggest improvements, and submit code changes.

## Code of Conduct

Please be respectful and constructive in all interactions. We are committed to providing a welcoming and inclusive experience for everyone.

## How to Contribute

### Reporting Bugs

1. Search [existing issues](../../issues) to check if the bug has already been reported
2. If not, open a new issue with:
   - Steps to reproduce the bug
   - Expected behavior vs. actual behavior
   - Your environment (OS, Rust version, Node.js version)
   - Relevant error messages or logs

### Suggesting Enhancements

1. Search [existing issues](../../issues) for similar suggestions
2. Open a new issue describing:
   - The problem or use case
   - Your proposed solution
   - Alternatives you considered

### Pull Requests

1. Fork the repository
2. Create a feature branch from `main` (`git checkout -b feature/your-feature`)
3. Make your changes
4. Ensure the project builds and tests pass
5. Write clear commit messages (see Style Guide below)
6. Push to your fork and open a pull request
7. Fill in the PR description explaining what changed and why

## Development Setup

### Rust

```bash
git clone https://github.com/syi0808/chkpt.git
cd chkpt
cargo build
cargo test --workspace
```

### Node.js Bindings

```bash
cd crates/chkpt-napi
pnpm install
pnpm build
pnpm test
```

### Running a Specific Crate

```bash
# Run the CLI
cargo run -p chkpt-cli -- save -m "test"

# Run the MCP server
cargo run -p chkpt-mcp
```

## Style Guide

### Code Style

This project uses Rust 2021 edition. Follow the existing patterns in the codebase. Key conventions:

- Use `thiserror` for error definitions in library crates
- Use `anyhow` for error handling in binary crates
- Async code uses `tokio` runtime
- Serialization uses `serde` with `bincode` for binary formats and `serde_json` for JSON

Format Rust code before submitting:

```bash
cargo fmt --all
```

### Commit Messages

This project uses [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add dry-run support to restore command
fix: handle symlinks during workspace scan
docs: update installation instructions
test: add index concurrency tests
```

- Use the imperative mood: "add feature" not "added feature"
- Keep the first line under 72 characters
- Reference issue numbers when applicable: `fix: handle empty dirs (#42)`

## Testing

Run the full test suite before submitting a pull request:

```bash
# Rust tests
cargo test --workspace

# Node.js binding tests
cd crates/chkpt-napi
pnpm test
```

Rust tests are located in `crates/chkpt-core/tests/` and cover all major components including blob storage, indexing, scanning, save/restore operations, and end-to-end workflows.

Node.js tests are in `crates/chkpt-napi/__test__/` and use Vitest.

To run a specific Rust test:

```bash
cargo test -p chkpt-core --test blob_test
```

To run a specific Node.js test:

```bash
cd crates/chkpt-napi
pnpm vitest run __test__/ops.spec.ts
```
