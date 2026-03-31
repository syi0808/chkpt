# chkpt Default Dependency Directory Exclusions Design

**Goal:** Exclude package-manager-managed dependency directories from workspace scans by default so `chkpt save` does not traverse large installed dependency trees unless the user opts in.

## Scope

This design changes the scanner's built-in exclusion behavior in `chkpt-core`:

1. Expand the built-in exclusion list to cover common dependency installation directories across JavaScript, Python, Rust, Ruby, and JVM tooling.
2. Apply those exclusions to matching path segments anywhere in the workspace, not only at the repository root.
3. Lock the behavior with scanner tests.

## Current Problem

The built-in exclusion logic only matches exact root-level directory names such as `node_modules/` and `target/`. Nested dependency trees such as `crates/chkpt-napi/node_modules/` are still scanned, which makes `save` much slower on multi-language workspaces.

## Proposed Design

### 1. Match Excluded Directories Anywhere In The Relative Path

Treat the built-in exclusions as directory names, not root-relative prefixes. A path should be ignored when any path segment matches an excluded directory name.

Examples:

- `node_modules/pkg/index.js` -> ignored
- `crates/chkpt-napi/node_modules/pkg/index.js` -> ignored
- `packages/app/.venv/lib/python3.12/site-packages/a.py` -> ignored
- `src/targeting.rs` -> not ignored

### 2. Use A Conservative Default List

Default exclusions should focus on installed dependency or package-manager state directories that are usually reproducible from a lockfile or package manifest:

- Existing: `.git`, `.chkpt`, `target`, `node_modules`
- Add: `.venv`, `venv`, `__pypackages__`, `.tox`, `.nox`, `.gradle`, `.m2`

Do not add broad generic names that may reasonably contain user-owned source content.

## Non-Goals

- No change to `.chkptignore` semantics.
- No attempt to exclude every cache directory.
- No change to snapshot format or restore behavior.

## Testing

- Scanner tests should verify nested `node_modules` directories are excluded.
- Scanner tests should verify nested Python virtualenv directories are excluded.
- Scanner tests should verify ordinary source paths with similar names are still included.
