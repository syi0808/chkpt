# Benchmarking Workflow

This repository now includes a repeatable benchmark workflow for fixture generation, benchmark execution, and before/after comparison.

## Goals

- Reuse the same fixture set across optimization steps.
- Keep benchmark state isolated from the developer's real `~/.chkpt` store.
- Record every run as structured JSON so each optimization can be evaluated one step at a time.

## Commands

List fixtures and scenarios:

```bash
python3 scripts/benchmarks.py list
```

Generate all reusable fixtures:

```bash
python3 scripts/benchmarks.py fixtures
```

Run the default suite and save parsed results:

```bash
python3 scripts/benchmarks.py run --label baseline
```

Run only a subset while iterating on one change:

```bash
python3 scripts/benchmarks.py run \
  --label step-01 \
  --scenario bench_ops_default \
  --scenario bench_ops_large \
  --scenario save_pipeline_hardlink_modules \
  --scenario save_pipeline_text_large
```

Compare two benchmark runs and write a Markdown table:

```bash
python3 scripts/benchmarks.py compare \
  --before .benchmarks/runs/<before>/results.json \
  --after .benchmarks/runs/<after>/results.json \
  --output docs/benchmarks/step-01.md
```

## Step-By-Step Optimization Loop

1. Generate fixtures once with `python3 scripts/benchmarks.py fixtures`.
2. Capture a baseline with `python3 scripts/benchmarks.py run --label baseline`.
3. Apply exactly one optimization.
4. Capture a second run with `python3 scripts/benchmarks.py run --label step-XX`.
5. Render the comparison table with `python3 scripts/benchmarks.py compare ...`.
6. Keep the change only if the comparison table shows a real improvement or an acceptable tradeoff.

## Isolation

The benchmark runner sets `CHKPT_HOME` to a dedicated directory under `.benchmarks/homes/`. That keeps all benchmark saves out of the user's normal store and makes repeated runs easier to reason about.

## Current Scenarios

- `bench_ops_default`: end-to-end synthetic default workload.
- `bench_ops_large`: end-to-end larger synthetic workload.
- `save_pipeline_text_small`: sectioned save path on small text files.
- `save_pipeline_text_large`: sectioned save path on a larger text fixture.
- `save_pipeline_mixed_large`: sectioned save path on mixed small and large files.
- `save_pipeline_random_binary`: sectioned save path on incompressible binary data.
- `save_pipeline_hardlink_modules`: sectioned save path on a hardlink-heavy node_modules-like fixture.
