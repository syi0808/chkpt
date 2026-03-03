# Automation Patterns

## When to Suggest `chkpt save`

### Before Risky Operations

Suggest saving a checkpoint before the user or you are about to:

- **Large-scale refactoring** — changing more than 5 files, renaming modules, moving directories
- **File or directory deletion** — `rm`, `git rm`, or bulk file removal
- **Dependency changes** — modifying `package.json`, `Cargo.toml`, `requirements.txt`, `go.mod`, etc.
- **Risky git operations** — `git rebase`, `git reset`, `git merge` with conflicts expected
- **Database migrations** — schema changes that affect application state
- **Configuration changes** — environment files, build configs, CI/CD pipelines

**Suggested message format:**

```bash
chkpt save -m "before: <brief description of upcoming operation>"
```

### After Milestones

Suggest saving after:

- **Major feature completion** — a logical unit of work is done and working
- **All tests passing** — a known-good state worth preserving
- **Successful build** — after resolving complex build issues

**Suggested message format:**

```bash
chkpt save -m "milestone: <what was achieved>"
```

## When to Suggest `chkpt restore`

- **Repeated build/test failures** — if changes introduced failures and reverting would be cleaner than debugging
- **User requests undo** — "undo", "go back", "revert to before", "roll back"
- **Workspace corruption** — missing files, broken state after failed operations

**Always use dry-run first:**

```bash
chkpt restore latest --dry-run
```

Then show the user what would change and ask for confirmation.

## When NOT to Auto-Suggest

Do not proactively suggest checkpoints when:

- **Minor edits** — changing 1-2 files with small modifications
- **Read-only operations** — browsing, searching, reading files
- **User declined recently** — if the user said "no" to a checkpoint suggestion in the current session, do not suggest again for similar operations
- **Rapid iteration** — user is in a tight edit-test loop; don't interrupt every cycle
