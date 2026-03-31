# chkpt-core Benchmark Before/After Report (2026-03-31)

## Scope

- Change under test: inode-locality sorting before file hashing in save/restore hot paths.
- Baseline (Before): `HEAD` commit in detached worktree `/tmp/chkpt-baseline-20260331` (`4a5c755`).
- After: current workspace changes in `/Users/sung-yein/Workspace/chkpt`.

## Commands

### Scenario A (default bench_ops)

```bash
cargo run --release -q -p chkpt-core --example bench_ops -- --iterations 5
```

- Ran twice for Before and twice for After (total 10 iterations each side).

### Scenario B (large workspace stress)

```bash
cargo run --release -q -p chkpt-core --example bench_ops -- --files 20000 --modified-files 5000 --dirs 400 --iterations 3
```

- Ran once for Before and once for After (3 iterations each side).

## Results

### Scenario A: default bench_ops (10-iteration aggregate)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 42.00 | 42.76 | +1.80% |
| warm_save | 9.65 | 10.19 | +5.60% |
| incremental_save | 14.02 | 13.94 | -0.57% |
| restore_dry_run | 21.53 | 20.95 | -2.67% |
| restore_apply | 43.06 | 39.35 | -8.61% |

Notes:
- Outliers were observed in some iterations (filesystem/cache noise), so 10-iteration aggregate was used.
- Largest consistent gain in this scenario is `restore_apply`.

### Scenario B: large stress (files=20000, modified_files=5000, dirs=400)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 296.45 | 296.86 | +0.14% |
| warm_save | 44.82 | 45.68 | +1.92% |
| incremental_save | 106.76 | 106.99 | +0.22% |
| restore_dry_run | 154.27 | 154.54 | +0.18% |
| restore_apply | 559.12 | 541.52 | -3.15% |

Notes:
- Under larger modified-file workload, `restore_apply` still improved.
- Save path metrics were mostly neutral within noise range.

## Hotspot Snapshot (save_pipeline, fixed fixture path)

Command:

```bash
cargo bench -p chkpt-core --bench save_pipeline -- /tmp/chkpt-save-pipeline-fixture
```

Representative phase costs (same fixture):
- `read+hash+compress`: ~2.0s
- `pack_write`: ~0.85s to ~1.06s
- `scan`, `build_tree`, `index_flush`: comparatively small

Interpretation:
- The dominant bottleneck remains compression + pack writing.
- This aligns with where additional optimization effort should focus next.

---

## Incremental Step Log

### Step 1 (Rejected): Remove save hot-path hex conversion / decode

Change attempted:
- Add byte-based precompressed pack write API.
- Remove eager hash-hex conversion in save prepare loop.

Benchmark policy for this step:
- `bench_ops` only (no `save_pipeline` reruns for step gating).
- Before/After executed sequentially on same workspace.

#### Scenario A (default bench_ops, iterations=5)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 42.49 | 41.53 | -2.26% |
| warm_save | 10.01 | 9.48 | -5.29% |
| incremental_save | 14.44 | 14.43 | -0.07% |
| restore_dry_run | 20.66 | 20.20 | -2.23% |
| restore_apply | 38.99 | 39.59 | +1.54% |

#### Scenario B (large, files=20000 modified_files=5000 dirs=400 iterations=3)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 317.98 | 368.99 | +16.04% |
| warm_save | 45.47 | 45.86 | +0.86% |
| incremental_save | 108.46 | 113.56 | +4.70% |
| restore_dry_run | 156.65 | 156.50 | -0.10% |
| restore_apply | 589.90 | 565.42 | -4.15% |

Decision:
- Mixed result with notable regression risk on large-workload save path.
- Step 1 changes were **rolled back** and not kept.

### Step 2 (Kept): save compression context reuse + restore apply parallelization

Changes applied:
- Reused `zstd::bulk::Compressor` per worker in save file preparation path.
- Parallelized restore apply path for add/change file writes.

#### Scenario A (default bench_ops, iterations=5)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 43.02 | 42.27 | -1.74% |
| warm_save | 9.64 | 10.00 | +3.73% |
| incremental_save | 13.67 | 14.56 | +6.51% |
| restore_dry_run | 21.09 | 21.34 | +1.19% |
| restore_apply | 39.41 | 39.47 | +0.15% |

#### Scenario B (large, files=20000 modified_files=5000 dirs=400 iterations=3)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 373.52 | 369.29 | -1.13% |
| warm_save | 45.40 | 46.70 | +2.86% |
| incremental_save | 113.31 | 108.18 | -4.53% |
| restore_dry_run | 155.87 | 157.01 | +0.73% |
| restore_apply | 546.44 | 466.09 | -14.70% |

Decision:
- Kept. The primary target metric (`restore_apply`) improved significantly on large workload.
- Small-workload save metrics regressed slightly; monitor in follow-up tuning.

### Step 3 (Kept): Skip per-file loose-object lookup when store has no loose objects

Changes applied:
- Added `blob_store_has_loose_objects(objects_dir)` in restore path.
- In `restore_files`, guard `blob_store.exists(hash)` behind one-time `has_loose_objects` check to avoid redundant filesystem metadata lookups when objects are fully packed.

Benchmark policy for this step:
- Before baseline: detached worktree at commit `e5c8c5c` (`/tmp/chkpt-bench-baseline`).
- After: current workspace change set.
- Commands executed in same session and host conditions.

#### Scenario A (default bench_ops, iterations=5)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 42.99 | 43.04 | +0.12% |
| warm_save | 10.64 | 9.78 | -8.08% |
| incremental_save | 14.40 | 14.73 | +2.29% |
| restore_dry_run | 22.23 | 22.02 | -0.94% |
| restore_apply | 38.03 | 37.97 | -0.16% |

#### Scenario B (large, files=20000 modified_files=5000 dirs=400 iterations=3)

Run 1:

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 421.91 | 380.67 | -9.77% |
| warm_save | 44.45 | 45.64 | +2.68% |
| incremental_save | 109.76 | 108.36 | -1.28% |
| restore_dry_run | 157.24 | 157.12 | -0.08% |
| restore_apply | 490.03 | 460.61 | -6.00% |

Run 2:

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 376.81 | 390.09 | +3.52% |
| warm_save | 46.60 | 44.99 | -3.45% |
| incremental_save | 108.71 | 108.55 | -0.15% |
| restore_dry_run | 159.35 | 156.19 | -1.98% |
| restore_apply | 519.86 | 483.68 | -6.96% |

Decision:
- Kept. Target metric `restore_apply` improved consistently in large-workload repeated runs (~6-7%).
- Other save-related metrics fluctuate within expected filesystem/cache noise range.

### Step 4 (Kept): One-pass restore state diff (remove BTreeSet diff allocations)

Changes applied:
- Replaced `target_paths/current_paths` `BTreeSet` construction + `difference/intersection` passes with a single ordered merge-diff over `BTreeMap` iterators.
- Added `diff_restore_states(...)` helper and unit tests for add/change/remove/unchanged classification.

Benchmark policy for this step:
- Before baseline: current `HEAD` at step 3 (`75898c5`) in same workspace/session.
- After: current workspace with step 4 changes.
- Benchmarks executed sequentially (not parallel) to avoid cross-run contention.

#### Scenario A (default bench_ops, iterations=5)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 46.89 | 46.21 | -1.45% |
| warm_save | 10.30 | 9.76 | -5.24% |
| incremental_save | 14.78 | 14.50 | -1.89% |
| restore_dry_run | 22.32 | 20.87 | -6.50% |
| restore_apply | 40.65 | 37.20 | -8.49% |

#### Scenario B (large, files=20000 modified_files=5000 dirs=400 iterations=3)

| Metric | Before Avg (ms) | After Avg (ms) | Delta |
|---|---:|---:|---:|
| cold_save | 387.06 | 381.35 | -1.48% |
| warm_save | 45.46 | 45.50 | +0.09% |
| incremental_save | 107.82 | 107.14 | -0.63% |
| restore_dry_run | 161.16 | 155.58 | -3.46% |
| restore_apply | 489.49 | 479.13 | -2.12% |

Decision:
- Kept. This step improves the restore comparison phase consistently, with notable gains in both `restore_dry_run` and `restore_apply`.
