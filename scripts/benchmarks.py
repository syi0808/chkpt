#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]
ARTIFACT_ROOT = REPO_ROOT / ".benchmarks"
FIXTURE_ROOT = ARTIFACT_ROOT / "fixtures"
RUNS_ROOT = ARTIFACT_ROOT / "runs"
HOMES_ROOT = ARTIFACT_ROOT / "homes"
FIXTURE_VERSION = 3


@dataclass(frozen=True)
class FixtureSpec:
    name: str
    kind: str
    files: int
    dirs: int
    seed: int = 42
    modified_files: int = 0
    small_size: int = 4096
    large_every: int = 0
    large_size: int = 0
    hardlink_packages: int = 0
    hardlink_files_per_package: int = 0
    hardlink_pool_size: int = 0


@dataclass(frozen=True)
class Scenario:
    name: str
    kind: str
    description: str
    command: list[str]
    fixture: str | None = None


FIXTURES: dict[str, FixtureSpec] = {
    "text_small": FixtureSpec(
        name="text_small",
        kind="text",
        files=3000,
        dirs=60,
    ),
    "text_large": FixtureSpec(
        name="text_large",
        kind="text",
        files=20000,
        dirs=400,
    ),
    "mixed_large": FixtureSpec(
        name="mixed_large",
        kind="mixed",
        files=5000,
        dirs=100,
        large_every=10,
        large_size=256 * 1024,
    ),
    "random_binary": FixtureSpec(
        name="random_binary",
        kind="random_binary",
        files=800,
        dirs=40,
        large_size=128 * 1024,
    ),
    "hardlink_modules": FixtureSpec(
        name="hardlink_modules",
        kind="hardlink_modules",
        files=3000,
        dirs=60,
        modified_files=200,
        hardlink_packages=2000,
        hardlink_files_per_package=20,
        hardlink_pool_size=128,
    ),
}


SCENARIOS: dict[str, Scenario] = {
    "bench_catalog_default": Scenario(
        name="bench_catalog_default",
        kind="bench_catalog",
        description="SQLite catalog metadata workload with a 3k-entry manifest and 64 seeded snapshots.",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_catalog",
            "--",
            "--iterations",
            "5",
        ],
    ),
    "bench_catalog_large": Scenario(
        name="bench_catalog_large",
        kind="bench_catalog",
        description="SQLite catalog metadata workload with a 20k-entry manifest and 256 seeded snapshots.",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_catalog",
            "--",
            "--manifest-entries",
            "20000",
            "--blob-count",
            "20000",
            "--seeded-snapshots",
            "256",
            "--iterations",
            "3",
        ],
    ),
    "bench_catalog_node_modules": Scenario(
        name="bench_catalog_node_modules",
        kind="bench_catalog",
        description="SQLite catalog metadata workload with an 80k-entry node_modules-like manifest and 256 seeded snapshots.",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_catalog",
            "--",
            "--manifest-entries",
            "80000",
            "--blob-count",
            "45000",
            "--seeded-snapshots",
            "256",
            "--iterations",
            "2",
        ],
    ),
    "bench_ops_default": Scenario(
        name="bench_ops_default",
        kind="bench_ops",
        description="End-to-end synthetic default workload (3k files, 200 modified).",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_ops",
            "--",
            "--iterations",
            "5",
        ],
    ),
    "bench_ops_large": Scenario(
        name="bench_ops_large",
        kind="bench_ops",
        description="End-to-end larger workload (20k files, 5k modified).",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_ops",
            "--",
            "--files",
            "20000",
            "--modified-files",
            "5000",
            "--dirs",
            "400",
            "--iterations",
            "3",
        ],
    ),
    "bench_ops_node_modules_hardlinks": Scenario(
        name="bench_ops_node_modules_hardlinks",
        kind="bench_ops",
        description="End-to-end include-deps workload with hardlink-heavy node_modules files.",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_ops",
            "--",
            "--files",
            "40000",
            "--modified-files",
            "4000",
            "--dirs",
            "800",
            "--iterations",
            "3",
            "--include-deps",
            "--hardlink-fanout",
            "8",
        ],
    ),
    "bench_ops_node_modules_hardlinks_restore": Scenario(
        name="bench_ops_node_modules_hardlinks_restore",
        kind="bench_ops",
        description="End-to-end include-deps workload with broken hardlink aliases before restore.",
        command=[
            "cargo",
            "run",
            "--release",
            "-q",
            "-p",
            "chkpt-core",
            "--example",
            "bench_ops",
            "--",
            "--files",
            "40000",
            "--modified-files",
            "4000",
            "--dirs",
            "800",
            "--iterations",
            "3",
            "--include-deps",
            "--hardlink-fanout",
            "8",
            "--break-deps-hardlinks",
        ],
    ),
    "save_pipeline_text_small": Scenario(
        name="save_pipeline_text_small",
        kind="save_pipeline",
        description="Sectioned save path on 3k small-text files.",
        command=["cargo", "bench", "-q", "-p", "chkpt-core", "--bench", "save_pipeline", "--"],
        fixture="text_small",
    ),
    "save_pipeline_text_large": Scenario(
        name="save_pipeline_text_large",
        kind="save_pipeline",
        description="Sectioned save path on 20k small-text files.",
        command=["cargo", "bench", "-q", "-p", "chkpt-core", "--bench", "save_pipeline", "--"],
        fixture="text_large",
    ),
    "save_pipeline_mixed_large": Scenario(
        name="save_pipeline_mixed_large",
        kind="save_pipeline",
        description="Sectioned save path on mixed small/large files.",
        command=["cargo", "bench", "-q", "-p", "chkpt-core", "--bench", "save_pipeline", "--"],
        fixture="mixed_large",
    ),
    "save_pipeline_random_binary": Scenario(
        name="save_pipeline_random_binary",
        kind="save_pipeline",
        description="Sectioned save path on incompressible binary files.",
        command=["cargo", "bench", "-q", "-p", "chkpt-core", "--bench", "save_pipeline", "--"],
        fixture="random_binary",
    ),
    "save_pipeline_hardlink_modules": Scenario(
        name="save_pipeline_hardlink_modules",
        kind="save_pipeline",
        description="Sectioned save path on a hardlink-heavy node_modules-like fixture.",
        command=["cargo", "bench", "-q", "-p", "chkpt-core", "--bench", "save_pipeline", "--"],
        fixture="hardlink_modules",
    ),
}

DEFAULT_SCENARIOS = [
    "bench_catalog_default",
    "bench_catalog_large",
    "bench_ops_default",
    "bench_ops_large",
    "save_pipeline_text_small",
    "save_pipeline_text_large",
    "save_pipeline_mixed_large",
    "save_pipeline_random_binary",
]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate benchmark fixtures, run the benchmark suite, and compare results."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    subparsers.add_parser("list", help="List available fixtures and benchmark scenarios.")

    fixtures_parser = subparsers.add_parser("fixtures", help="Generate reusable benchmark fixtures.")
    fixtures_parser.add_argument(
        "--fixture",
        action="append",
        dest="fixtures",
        choices=sorted(FIXTURES.keys()),
        help="Only generate the selected fixture. Repeat to select multiple fixtures.",
    )

    run_parser = subparsers.add_parser("run", help="Run the benchmark suite and store parsed results.")
    run_parser.add_argument("--label", required=True, help="Human-readable label for this run.")
    run_parser.add_argument(
        "--scenario",
        action="append",
        dest="scenarios",
        choices=sorted(SCENARIOS.keys()),
        help="Only run the selected scenario. Repeat to select multiple scenarios.",
    )
    run_parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip the upfront release build and use existing build artifacts.",
    )

    compare_parser = subparsers.add_parser(
        "compare", help="Render a Markdown before/after comparison table from two benchmark runs."
    )
    compare_parser.add_argument("--before", required=True, help="Path to an earlier run dir or results.json.")
    compare_parser.add_argument("--after", required=True, help="Path to a later run dir or results.json.")
    compare_parser.add_argument(
        "--stat",
        choices=["median", "average"],
        default="median",
        help="Primary statistic to compare for iterative benchmarks. Defaults to median.",
    )
    compare_parser.add_argument("--output", help="Optional path to write the rendered Markdown report.")

    compare_commits_parser = subparsers.add_parser(
        "compare-commits",
        help="Build two refs once, run them in alternating order, and render a comparison report.",
    )
    compare_commits_parser.add_argument("--before-ref", required=True, help="Git ref for the baseline.")
    compare_commits_parser.add_argument("--after-ref", required=True, help="Git ref for the candidate.")
    compare_commits_parser.add_argument("--label", required=True, help="Human-readable label for this run.")
    compare_commits_parser.add_argument(
        "--scenario",
        action="append",
        dest="scenarios",
        choices=sorted(SCENARIOS.keys()),
        help="Only run the selected scenario. Repeat to select multiple scenarios.",
    )
    compare_commits_parser.add_argument(
        "--rounds",
        type=int,
        default=2,
        help="Number of alternating A/B rounds. Default: 2.",
    )
    compare_commits_parser.add_argument(
        "--skip-build",
        action="store_true",
        help="Skip the upfront release build in each worktree and use existing build artifacts.",
    )
    compare_commits_parser.add_argument(
        "--stat",
        choices=["median", "average"],
        default="median",
        help="Primary statistic to compare for the final report. Defaults to median.",
    )
    compare_commits_parser.add_argument("--output", help="Optional path to write the rendered Markdown report.")

    return parser.parse_args()


def sanitize_label(label: str) -> str:
    cleaned = re.sub(r"[^a-zA-Z0-9._-]+", "-", label.strip()).strip("-")
    return cleaned or "run"


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def deterministic_bytes(size: int, seed: int) -> bytes:
    mask = (1 << 64) - 1
    state = (seed ^ 0x9E3779B97F4A7C15) & mask
    out = bytearray(size)
    for index in range(size):
        state ^= (state << 13) & mask
        state ^= state >> 7
        state ^= (state << 17) & mask
        out[index] = state & 0xFF
    return bytes(out)


def write_text_file(path: Path, index: int, version: int, size: int) -> None:
    body = "x" * (size + version * 17 + (index % 31))
    path.write_text(f"file={index}\nversion={version}\n{body}", encoding="utf-8")


def create_fixture(spec: FixtureSpec, root: Path) -> Path:
    fixture_dir = root / spec.name
    store_dir = root / f"{spec.name}.store"
    manifest_path = fixture_dir / "fixture-manifest.json"
    manifest = {
        "version": FIXTURE_VERSION,
        "spec": asdict(spec),
    }

    if manifest_path.exists():
        existing = json.loads(manifest_path.read_text(encoding="utf-8"))
        if existing == manifest and (spec.kind != "hardlink_modules" or store_dir.exists()):
            return fixture_dir

    if fixture_dir.exists():
        shutil.rmtree(fixture_dir)
    if store_dir.exists():
        shutil.rmtree(store_dir)
    fixture_dir.mkdir(parents=True, exist_ok=True)

    if spec.kind == "hardlink_modules":
        create_hardlink_modules_fixture(spec, fixture_dir, store_dir)
        manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True), encoding="utf-8")
        return fixture_dir

    for index in range(spec.files):
        dir_path = fixture_dir / f"dir_{index % max(spec.dirs, 1):04d}"
        dir_path.mkdir(parents=True, exist_ok=True)
        if spec.kind == "text":
            write_text_file(dir_path / f"file_{index:05d}.txt", index, 0, spec.small_size)
        elif spec.kind == "mixed":
            if spec.large_every and index % spec.large_every == 0:
                payload = deterministic_bytes(spec.large_size + (index % 1024), spec.seed + index)
                (dir_path / f"file_{index:05d}.bin").write_bytes(payload)
            else:
                write_text_file(dir_path / f"file_{index:05d}.txt", index, 0, spec.small_size)
        elif spec.kind == "random_binary":
            payload = deterministic_bytes(spec.large_size, spec.seed + index)
            (dir_path / f"file_{index:05d}.bin").write_bytes(payload)
        else:
            raise ValueError(f"unsupported fixture kind: {spec.kind}")

    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True), encoding="utf-8")
    return fixture_dir


def create_hardlink_modules_fixture(spec: FixtureSpec, workspace_dir: Path, store_dir: Path) -> None:
    ensure_dir(store_dir)

    source_root = workspace_dir / "src"
    ensure_dir(source_root)
    for index in range(spec.files):
        dir_path = source_root / f"dir_{index % max(spec.dirs, 1):04d}"
        ensure_dir(dir_path)
        write_text_file(dir_path / f"file_{index:05d}.txt", index, 0, spec.small_size)

    source_pool = store_dir / "pool"
    ensure_dir(source_pool)
    for index in range(spec.hardlink_pool_size):
        payload = deterministic_bytes(spec.small_size + (index % 257), spec.seed + index)
        (source_pool / f"asset_{index:04d}.js").write_bytes(payload)

    node_modules_root = workspace_dir / "node_modules"
    ensure_dir(node_modules_root)

    for package_index in range(spec.hardlink_packages):
        package_dir = node_modules_root / f"pkg_{package_index:05d}"
        ensure_dir(package_dir)
        for file_index in range(spec.hardlink_files_per_package):
            source_index = (package_index * spec.hardlink_files_per_package + file_index) % spec.hardlink_pool_size
            source_file = source_pool / f"asset_{source_index:04d}.js"
            target_file = package_dir / f"asset_{file_index:02d}.js"
            if target_file.exists():
                target_file.unlink()
            os.link(source_file, target_file)

    bin_dir = node_modules_root / ".bin"
    ensure_dir(bin_dir)
    for package_index in range(min(spec.hardlink_packages, 32)):
        link_path = bin_dir / f"pkg-{package_index:05d}"
        if link_path.exists() or link_path.is_symlink():
            link_path.unlink()
        target = f"../pkg_{package_index:05d}/asset_00.js"
        try:
            os.symlink(target, link_path)
        except (AttributeError, NotImplementedError, OSError):
            break

    package_json = workspace_dir / "package.json"
    package_json.write_text(
        json.dumps(
            {
                "name": "hardlink-modules-fixture",
                "private": True,
                "dependencies": {
                    f"pkg-{index:05d}": "1.0.0" for index in range(min(spec.hardlink_packages, 20))
                },
            },
            indent=2,
            sort_keys=True,
        ),
        encoding="utf-8",
    )

def run_command(cmd: list[str], env: dict[str, str], cwd: Path = REPO_ROOT) -> str:
    completed = subprocess.run(
        cmd,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=True,
    )
    return completed.stdout


def parse_key_values(line: str) -> dict[str, float]:
    pairs = dict(re.findall(r"([a-zA-Z0-9_]+)=([0-9]+(?:\.[0-9]+)?)", line))
    return {key: float(value) for key, value in pairs.items()}


def summarize_values(values: list[float]) -> dict[str, float]:
    if not values:
        return {}
    median = statistics.median(values)
    mad = statistics.median(abs(value - median) for value in values)
    return {
        "samples": float(len(values)),
        "mean": statistics.fmean(values),
        "median": median,
        "min": min(values),
        "max": max(values),
        "mad": mad,
        "relative_mad_pct": 0.0 if median == 0 else mad / median * 100.0,
        "span_pct": 0.0 if median == 0 else (max(values) - min(values)) / median * 100.0,
    }


def summarize_iterations(iterations: list[dict[str, float]]) -> dict[str, dict[str, float]]:
    metrics: dict[str, list[float]] = {}
    for iteration in iterations:
        for key, value in iteration.items():
            if key == "iteration":
                continue
            metrics.setdefault(key, []).append(value)

    summary: dict[str, dict[str, float]] = {}
    for key, values in metrics.items():
        if not values:
            continue
        summary[key] = summarize_values(values)
    return summary


def summarize_sample_dicts(
    samples: list[dict[str, float]],
) -> tuple[dict[str, float], dict[str, dict[str, float]]]:
    metrics: dict[str, list[float]] = {}
    for sample in samples:
        for key, value in sample.items():
            metrics.setdefault(key, []).append(value)

    average: dict[str, float] = {}
    summary: dict[str, dict[str, float]] = {}
    for key, values in metrics.items():
        if not values:
            continue
        metric_summary = summarize_values(values)
        average[key] = metric_summary["mean"]
        summary[key] = metric_summary
    return average, summary


def parse_bench_ops(output: str) -> dict[str, Any]:
    config: dict[str, float] | None = None
    iterations: list[dict[str, float]] = []
    average: dict[str, float] | None = None
    resources: dict[str, float] = {}

    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("benchmark_config "):
            config = parse_key_values(stripped)
        elif stripped.startswith("iteration="):
            iterations.append(parse_key_values(stripped))
        elif stripped.startswith("average "):
            average = parse_key_values(stripped)

    if config is None or average is None:
        raise ValueError("failed to parse bench_ops output")

    if "peak_rss_kb" in average:
        resources["peak_rss_kb"] = average["peak_rss_kb"]

    return {
        "config": config,
        "iterations": iterations,
        "average": average,
        "summary": summarize_iterations(iterations),
        "resources": resources,
        "resource_summary": {
            "peak_rss_kb": summarize_values([resources["peak_rss_kb"]])
        }
        if "peak_rss_kb" in resources
        else {},
    }


def parse_bench_catalog(output: str) -> dict[str, Any]:
    config: dict[str, float] | None = None
    iterations: list[dict[str, float]] = []
    average: dict[str, float] | None = None
    resources: dict[str, float] = {}

    for line in output.splitlines():
        stripped = line.strip()
        if stripped.startswith("benchmark_config "):
            config = parse_key_values(stripped)
        elif stripped.startswith("iteration="):
            iterations.append(parse_key_values(stripped))
        elif stripped.startswith("average "):
            average = parse_key_values(stripped)

    if config is None or average is None:
        raise ValueError("failed to parse bench_catalog output")

    if "peak_rss_kb" in average:
        resources["peak_rss_kb"] = average["peak_rss_kb"]

    return {
        "config": config,
        "iterations": iterations,
        "average": average,
        "summary": summarize_iterations(iterations),
        "resources": resources,
        "resource_summary": {
            "peak_rss_kb": summarize_values([resources["peak_rss_kb"]])
        }
        if "peak_rss_kb" in resources
        else {},
    }


def parse_save_pipeline(output: str) -> dict[str, Any]:
    phases: dict[str, int] = {}
    index_breakdown: dict[str, int] = {}
    variants: dict[str, int] = {}
    metadata: dict[str, int] = {}
    resources: dict[str, int] = {}

    phase_patterns = {
        "scan_ms": re.compile(r"^\[scan\]\s+(?P<ms>\d+)ms"),
        "index_total_ms": re.compile(
            r"^\[index\]\s+(?P<ms>\d+)ms\s+\(load: (?P<load>\d+)ms, check: (?P<check>\d+)ms, cached: (?P<cached>\d+), new: (?P<new>\d+)\)"
        ),
        "read_hash_compress_ms": re.compile(
            r"^\[read\+hash\+compress\]\s+(?P<ms>\d+)ms\s+\(unique: (?P<unique>\d+), dup: (?P<dup>\d+)\)"
        ),
        "pack_write_ms": re.compile(r"^\[pack_write\]\s+(?P<ms>\d+)ms"),
        "build_tree_ms": re.compile(r"^\[build_tree\]\s+(?P<ms>\d+)ms\s+\((?P<dirs>\d+) dirs\)"),
        "index_flush_ms": re.compile(r"^\[index_flush\]\s+(?P<ms>\d+)ms"),
        "dir_fd_cache_ms": re.compile(r"^dir FD cache build: (?P<ms>\d+)ms \((?P<dirs>\d+) dirs\)"),
        "best_threads": re.compile(r"^>> best: (?P<threads>\d+) threads \((?P<ms>\d+)ms\)"),
    }
    variant_pattern = re.compile(r"^(?P<label>.+?)\s+(?P<ms>\d+)ms\s+\((?P<threads>\d+) threads\)$")
    resource_pattern = re.compile(r"^peak_rss_kb=(?P<kb>\d+)$")

    label_map = {
        "baseline (std::fs, path order)": "baseline_read_hash_ms",
        "inode-sorted": "inode_sorted_read_hash_ms",
        "openat + inode-sorted": "openat_read_hash_ms",
        "mmap hybrid + inode-sorted": "mmap_hybrid_read_hash_ms",
        "ALL COMBINED": "combined_read_hash_ms",
    }

    for raw_line in output.splitlines():
        line = raw_line.strip()
        if not line:
            continue

        matched = False
        for key, pattern in phase_patterns.items():
            match = pattern.match(line)
            if not match:
                continue
            groups = {name: int(value) for name, value in match.groupdict().items()}
            matched = True
            if key == "index_total_ms":
                phases[key] = groups["ms"]
                index_breakdown = {
                    "load_ms": groups["load"],
                    "check_ms": groups["check"],
                    "cached": groups["cached"],
                    "new": groups["new"],
                }
            elif key == "read_hash_compress_ms":
                phases[key] = groups["ms"]
                metadata["unique_objects"] = groups["unique"]
                metadata["duplicate_objects"] = groups["dup"]
            elif key == "build_tree_ms":
                phases[key] = groups["ms"]
                metadata["tree_dirs"] = groups["dirs"]
            elif key == "dir_fd_cache_ms":
                variants[key] = groups["ms"]
                metadata["dir_fd_dirs"] = groups["dirs"]
            elif key == "best_threads":
                variants["best_thread_count"] = groups["threads"]
                variants["best_threads_read_hash_ms"] = groups["ms"]
            else:
                phases[key] = groups["ms"]
            break
        if matched:
            continue

        resource_match = resource_pattern.match(line)
        if resource_match:
            resources["peak_rss_kb"] = int(resource_match.group("kb"))
            continue

        variant_match = variant_pattern.match(line)
        if variant_match:
            label = variant_match.group("label").strip()
            metric_key = label_map.get(label)
            if metric_key:
                variants[metric_key] = int(variant_match.group("ms"))
                if metric_key == "baseline_read_hash_ms":
                    variants["baseline_thread_count"] = int(variant_match.group("threads"))

    required = [
        "scan_ms",
        "index_total_ms",
        "read_hash_compress_ms",
        "pack_write_ms",
        "build_tree_ms",
        "index_flush_ms",
    ]
    missing = [key for key in required if key not in phases]
    if missing:
        raise ValueError(f"failed to parse save_pipeline output: missing {missing}")

    return {
        "phases": phases,
        "phase_summary": {key: summarize_values([float(value)]) for key, value in phases.items()},
        "index_breakdown": index_breakdown,
        "index_breakdown_summary": {
            key: summarize_values([float(value)]) for key, value in index_breakdown.items()
        },
        "variants": variants,
        "variant_summary": {key: summarize_values([float(value)]) for key, value in variants.items()},
        "metadata": metadata,
        "resources": resources,
        "resource_summary": {
            "peak_rss_kb": summarize_values([float(resources["peak_rss_kb"])])
        }
        if "peak_rss_kb" in resources
        else {},
    }


def git_info(repo_root: Path = REPO_ROOT) -> dict[str, str]:
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repo_root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=True,
    ).stdout.strip()
    branch = subprocess.run(
        ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        cwd=repo_root,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        check=True,
    ).stdout.strip()
    return {"commit": commit, "branch": branch}


def load_result(path_str: str) -> dict[str, Any]:
    path = Path(path_str)
    if path.is_dir():
        path = path / "results.json"
    return json.loads(path.read_text(encoding="utf-8"))


def metric_summary(parsed: dict[str, Any]) -> dict[str, dict[str, float]]:
    summary = parsed.get("summary")
    if summary:
        return summary
    iterations = parsed.get("iterations", [])
    if isinstance(iterations, list):
        return summarize_iterations(iterations)
    return {}


def metric_value(parsed: dict[str, Any], key: str, stat: str) -> float | None:
    if stat == "median":
        summary = metric_summary(parsed).get(key)
        if summary is not None:
            return float(summary["median"])

    average = parsed.get("average", {})
    if key in average:
        return float(average[key])

    summary = metric_summary(parsed).get(key)
    if summary is not None:
        return float(summary["mean"])
    return None


def nested_metric_value(
    parsed: dict[str, Any],
    group_key: str,
    summary_key: str,
    metric_key: str,
    stat: str,
) -> float | None:
    if stat == "median":
        summary = parsed.get(summary_key, {}).get(metric_key)
        if summary is not None:
            return float(summary["median"])

    group = parsed.get(group_key, {})
    if metric_key in group:
        return float(group[metric_key])

    summary = parsed.get(summary_key, {}).get(metric_key)
    if summary is not None:
        return float(summary["mean"])
    return None


def resource_value(parsed: dict[str, Any], key: str, stat: str) -> float | None:
    if stat == "median":
        summary = parsed.get("resource_summary", {}).get(key)
        if summary is not None:
            return float(summary["median"])

    resources = parsed.get("resources", {})
    if key in resources:
        return float(resources[key])

    summary = parsed.get("resource_summary", {}).get(key)
    if summary is not None:
        return float(summary["mean"])
    return None


def render_metric_table(rows: list[tuple[str, float, float]], stat_label: str) -> str:
    lines = [
        f"| Metric | Before ({stat_label}) | After ({stat_label}) | Delta |",
        "|---|---:|---:|---:|",
    ]
    for label, before, after in rows:
        delta = after - before
        pct = 0.0 if before == 0 else delta / before * 100.0
        lines.append(f"| {label} | {before:.2f} | {after:.2f} | {delta:+.2f} ({pct:+.2f}%) |")
    return "\n".join(lines)


def collect_instability_notes(
    scenario_name: str,
    parsed: dict[str, Any],
    metrics: list[str],
    label: str,
) -> list[str]:
    notes: list[str] = []
    summary = metric_summary(parsed)
    for metric in metrics:
        metric_stats = summary.get(metric)
        if metric_stats is None:
            continue
        if metric_stats["samples"] < 3:
            continue
        span_pct = metric_stats["span_pct"]
        rel_mad_pct = metric_stats["relative_mad_pct"]
        if span_pct < 15.0 and rel_mad_pct < 5.0:
            continue
        notes.append(
            f"- `{scenario_name}.{metric}` {label}: median `{metric_stats['median']:.2f}`, "
            f"min/max `{metric_stats['min']:.2f}`/`{metric_stats['max']:.2f}`, "
            f"MAD `{metric_stats['mad']:.2f}` ({rel_mad_pct:.1f}%), span `{span_pct:.1f}%`"
        )
    return notes


def compare_runs(before: dict[str, Any], after: dict[str, Any], stat: str) -> str:
    before_scenarios = {entry["name"]: entry for entry in before["results"]}
    after_scenarios = {entry["name"]: entry for entry in after["results"]}
    shared = [name for name in before_scenarios if name in after_scenarios]
    stat_label = "median" if stat == "median" else "average"

    lines = [
        f"# Benchmark Comparison: {before['label']} -> {after['label']}",
        "",
        f"- Before: `{before['git']['commit'][:12]}` on `{before['git']['branch']}`",
        f"- After: `{after['git']['commit'][:12]}` on `{after['git']['branch']}`",
        f"- Primary statistic: `{stat_label}`",
        f"- Generated at: `{datetime.now(timezone.utc).isoformat()}`",
        "",
    ]

    for name in shared:
        left = before_scenarios[name]
        right = after_scenarios[name]
        lines.append(f"## {name}")
        lines.append("")
        lines.append(left["description"])
        lines.append("")
        if left["kind"] in {"bench_ops", "bench_catalog"}:
            if left["kind"] == "bench_ops":
                order = [
                    ("cold_save_ms", "cold_save_ms"),
                    ("warm_save_ms", "warm_save_ms"),
                    ("incremental_save_ms", "incremental_save_ms"),
                    ("restore_dry_run_ms", "restore_dry_run_ms"),
                    ("restore_apply_ms", "restore_apply_ms"),
                ]
            else:
                order = [
                    ("open_ms", "open_ms"),
                    ("bulk_upsert_ms", "bulk_upsert_ms"),
                    ("insert_snapshot_ms", "insert_snapshot_ms"),
                    ("latest_snapshot_ms", "latest_snapshot_ms"),
                    ("resolve_prefix_ms", "resolve_prefix_ms"),
                    ("list_snapshots_ms", "list_snapshots_ms"),
                    ("snapshot_manifest_ms", "snapshot_manifest_ms"),
                    ("blob_lookup_ms", "blob_lookup_ms"),
                ]
            rows = []
            compared_metrics: list[str] = []
            for key, label in order:
                left_value = metric_value(left["parsed"], key, stat)
                right_value = metric_value(right["parsed"], key, stat)
                if left_value is None or right_value is None:
                    continue
                rows.append((label, left_value, right_value))
                compared_metrics.append(key)
            left_rss = left["parsed"].get("resources", {}).get("peak_rss_kb")
            right_rss = right["parsed"].get("resources", {}).get("peak_rss_kb")
            left_rss_value = resource_value(left["parsed"], "peak_rss_kb", stat)
            right_rss_value = resource_value(right["parsed"], "peak_rss_kb", stat)
            if left_rss_value is not None and right_rss_value is not None:
                rows.append(("peak_rss_mib", left_rss_value / 1024.0, right_rss_value / 1024.0))
            lines.append(render_metric_table(rows, stat_label))
            instability_notes = collect_instability_notes(name, left["parsed"], compared_metrics, "before")
            instability_notes.extend(
                collect_instability_notes(name, right["parsed"], compared_metrics, "after")
            )
            if instability_notes:
                lines.append("")
                lines.append("Noise Notes:")
                lines.extend(instability_notes)
        else:
            variant_order = [
                ("scan_ms", "scan_ms"),
                ("index_total_ms", "index_total_ms"),
                ("baseline_read_hash_ms", "baseline_read_hash_ms"),
                ("inode_sorted_read_hash_ms", "inode_sorted_read_hash_ms"),
                ("best_threads_read_hash_ms", "best_threads_read_hash_ms"),
                ("openat_read_hash_ms", "openat_read_hash_ms"),
                ("mmap_hybrid_read_hash_ms", "mmap_hybrid_read_hash_ms"),
                ("combined_read_hash_ms", "combined_read_hash_ms"),
                ("read_hash_compress_ms", "read_hash_compress_ms"),
                ("pack_write_ms", "pack_write_ms"),
                ("build_tree_ms", "build_tree_ms"),
                ("index_flush_ms", "index_flush_ms"),
            ]
            rows = []
            for key, label in variant_order:
                before_value = nested_metric_value(left["parsed"], "phases", "phase_summary", key, stat)
                after_value = nested_metric_value(right["parsed"], "phases", "phase_summary", key, stat)
                if before_value is None:
                    before_value = nested_metric_value(
                        left["parsed"], "variants", "variant_summary", key, stat
                    )
                if after_value is None:
                    after_value = nested_metric_value(
                        right["parsed"], "variants", "variant_summary", key, stat
                    )
                if before_value is None or after_value is None:
                    continue
                rows.append((label, float(before_value), float(after_value)))
            left_rss_value = resource_value(left["parsed"], "peak_rss_kb", stat)
            right_rss_value = resource_value(right["parsed"], "peak_rss_kb", stat)
            if left_rss_value is not None and right_rss_value is not None:
                rows.append(("peak_rss_mib", left_rss_value / 1024.0, right_rss_value / 1024.0))
            lines.append(render_metric_table(rows, stat_label))
            before_best = left["parsed"]["variants"].get("best_thread_count")
            after_best = right["parsed"]["variants"].get("best_thread_count")
            if before_best is not None or after_best is not None:
                lines.append("")
                lines.append(
                    f"Best thread count: before `{before_best}` -> after `{after_best}`"
                )
        lines.append("")

    missing_after = sorted(set(before_scenarios) - set(after_scenarios))
    missing_before = sorted(set(after_scenarios) - set(before_scenarios))
    if missing_after or missing_before:
        lines.append("## Scenario Coverage")
        lines.append("")
        if missing_after:
            lines.append(f"- Missing in after: {', '.join(missing_after)}")
        if missing_before:
            lines.append(f"- Missing in before: {', '.join(missing_before)}")
        lines.append("")

    return "\n".join(lines).rstrip() + "\n"


def aggregate_iterative_parsed(parsed_runs: list[dict[str, Any]]) -> dict[str, Any]:
    config = parsed_runs[0].get("config")
    iterations: list[dict[str, float]] = []
    resource_samples: list[float] = []
    for parsed in parsed_runs:
        for iteration in parsed.get("iterations", []):
            sample = dict(iteration)
            sample.pop("iteration", None)
            iterations.append(sample)
        peak_rss = parsed.get("resources", {}).get("peak_rss_kb")
        if peak_rss is not None:
            resource_samples.append(float(peak_rss))

    average, summary = summarize_sample_dicts(iterations)
    resources: dict[str, float] = {}
    resource_summary: dict[str, dict[str, float]] = {}
    if resource_samples:
        resources["peak_rss_kb"] = statistics.fmean(resource_samples)
        resource_summary["peak_rss_kb"] = summarize_values(resource_samples)

    return {
        "config": config,
        "iterations": [
            {"iteration": float(index + 1), **sample} for index, sample in enumerate(iterations)
        ],
        "average": average,
        "summary": summary,
        "resources": resources,
        "resource_summary": resource_summary,
    }


def aggregate_save_pipeline_parsed(parsed_runs: list[dict[str, Any]]) -> dict[str, Any]:
    phase_average, phase_summary = summarize_sample_dicts(
        [parsed.get("phases", {}) for parsed in parsed_runs]
    )
    variant_average, variant_summary = summarize_sample_dicts(
        [parsed.get("variants", {}) for parsed in parsed_runs]
    )
    index_average, index_summary = summarize_sample_dicts(
        [parsed.get("index_breakdown", {}) for parsed in parsed_runs]
    )
    resource_samples = [
        float(parsed["resources"]["peak_rss_kb"])
        for parsed in parsed_runs
        if "peak_rss_kb" in parsed.get("resources", {})
    ]
    resources: dict[str, float] = {}
    resource_summary: dict[str, dict[str, float]] = {}
    if resource_samples:
        resources["peak_rss_kb"] = statistics.fmean(resource_samples)
        resource_summary["peak_rss_kb"] = summarize_values(resource_samples)

    return {
        "phases": phase_average,
        "phase_summary": phase_summary,
        "index_breakdown": index_average,
        "index_breakdown_summary": index_summary,
        "variants": variant_average,
        "variant_summary": variant_summary,
        "metadata": parsed_runs[0].get("metadata", {}),
        "resources": resources,
        "resource_summary": resource_summary,
    }


def aggregate_result_payloads(label: str, payloads: list[dict[str, Any]]) -> dict[str, Any]:
    if not payloads:
        raise ValueError("cannot aggregate empty payload list")

    aggregated_results: list[dict[str, Any]] = []
    scenario_names = [result["name"] for result in payloads[0]["results"]]
    for scenario_name in scenario_names:
        scenario_runs = []
        for payload in payloads:
            scenario = next(result for result in payload["results"] if result["name"] == scenario_name)
            scenario_runs.append(scenario)

        first = scenario_runs[0]
        parsed_runs = [scenario["parsed"] for scenario in scenario_runs]
        if first["kind"] in {"bench_ops", "bench_catalog"}:
            parsed = aggregate_iterative_parsed(parsed_runs)
        else:
            parsed = aggregate_save_pipeline_parsed(parsed_runs)

        aggregated_results.append(
            {
                "name": first["name"],
                "kind": first["kind"],
                "description": first["description"],
                "command": first["command"],
                "fixture": first.get("fixture"),
                "parsed": parsed,
                "ab_run_count": len(scenario_runs),
            }
        )

    return {
        "label": label,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "git": payloads[0]["git"],
        "benchmark_home": "",
        "results": aggregated_results,
    }


def create_detached_worktree(path: Path, ref: str) -> None:
    completed = subprocess.run(
        ["git", "worktree", "add", "--detach", str(path), ref],
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stdout.strip() or "git worktree add failed")


def remove_worktree(path: Path) -> None:
    completed = subprocess.run(
        ["git", "worktree", "remove", "--force", str(path)],
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stdout.strip() or "git worktree remove failed")


def scenario_runtime_args(scenario: Scenario) -> list[str]:
    if "--" not in scenario.command:
        return []
    separator = scenario.command.index("--")
    return scenario.command[separator + 1 :]


def fixed_bench_ops_source() -> str:
    return (REPO_ROOT / "crates/chkpt-core/examples/bench_ops.rs").read_text(encoding="utf-8")


def create_fixed_bench_ops_project(project_dir: Path, repo_root: Path) -> None:
    ensure_dir(project_dir / "src")
    cargo_toml = f"""[package]
name = "fixed-bench-ops"
version = "0.1.0"
edition = "2021"

[dependencies]
chkpt-core = {{ path = "{(repo_root / 'crates/chkpt-core').as_posix()}" }}
tempfile = "3"
libc = "0.2"
"""
    (project_dir / "Cargo.toml").write_text(cargo_toml, encoding="utf-8")
    (project_dir / "src/main.rs").write_text(fixed_bench_ops_source(), encoding="utf-8")


def fixed_bench_ops_binary_path(project_dir: Path) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return project_dir / "target" / "release" / f"fixed-bench-ops{suffix}"


def build_fixed_bench_ops_runner(project_dir: Path, repo_root: Path) -> Path:
    create_fixed_bench_ops_project(project_dir, repo_root)
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    run_command(["cargo", "build", "--release"], env, cwd=project_dir)
    return fixed_bench_ops_binary_path(project_dir)


def build_benchmark_artifacts(repo_root: Path, selected: list[str]) -> None:
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    run_command(build_command_for_scenarios(selected), env, cwd=repo_root)


def execute_benchmark_run(
    label: str,
    selected: list[str],
    skip_build: bool,
    repo_root: Path,
    persist_run_dir: bool = True,
    fixed_bench_ops_binary: Path | None = None,
) -> tuple[dict[str, Any], Path | None]:
    needed_fixtures = sorted(
        {SCENARIOS[name].fixture for name in selected if SCENARIOS[name].fixture is not None}
    )
    fixture_paths = generate_selected_fixtures([name for name in needed_fixtures if name is not None])

    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir: Path | None = None
    raw_dir: Path | None = None
    if persist_run_dir:
        run_dir = RUNS_ROOT / f"{timestamp}-{sanitize_label(label)}"
        raw_dir = run_dir / "raw"
        ensure_dir(raw_dir)
    ensure_dir(HOMES_ROOT)

    benchmark_home = HOMES_ROOT / f"{timestamp}-{sanitize_label(label)}"
    ensure_dir(benchmark_home)

    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    env["CHKPT_HOME"] = str(benchmark_home)

    if not skip_build:
        print("Building release benchmark artifacts...")
        build_output = run_command(build_command_for_scenarios(selected), env, cwd=repo_root)
        if raw_dir is not None:
            (raw_dir / "_build.txt").write_text(build_output, encoding="utf-8")

    results: list[dict[str, Any]] = []
    for name in selected:
        scenario = SCENARIOS[name]
        if scenario.kind == "bench_ops" and fixed_bench_ops_binary is not None:
            command = [str(fixed_bench_ops_binary), *scenario_runtime_args(scenario)]
        else:
            command = list(scenario.command)
            if scenario.fixture:
                command.append(fixture_paths[scenario.fixture])
        print(f"Running {name}...")
        output = run_command(command, env, cwd=repo_root)
        if raw_dir is not None:
            (raw_dir / f"{name}.txt").write_text(output, encoding="utf-8")
        if scenario.kind == "bench_ops":
            parsed = parse_bench_ops(output)
        elif scenario.kind == "bench_catalog":
            parsed = parse_bench_catalog(output)
        else:
            parsed = parse_save_pipeline(output)
        results.append(
            {
                "name": name,
                "kind": scenario.kind,
                "description": scenario.description,
                "command": command,
                "fixture": fixture_paths.get(scenario.fixture) if scenario.fixture else None,
                "parsed": parsed,
            }
        )

    payload = {
        "label": label,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "git": git_info(repo_root),
        "benchmark_home": str(benchmark_home),
        "results": results,
    }
    if run_dir is not None:
        ensure_dir(run_dir)
        results_path = run_dir / "results.json"
        results_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        return payload, results_path
    return payload, None


def generate_selected_fixtures(names: list[str]) -> dict[str, str]:
    ensure_dir(FIXTURE_ROOT)
    created: dict[str, str] = {}
    for name in names:
        fixture_dir = create_fixture(FIXTURES[name], FIXTURE_ROOT)
        created[name] = str(fixture_dir)
    return created


def scenario_names_from_args(values: list[str] | None, known: list[str]) -> list[str]:
    return values if values else known


def build_command_for_scenarios(selected: list[str]) -> list[str]:
    cmd = ["cargo", "build", "--release", "-p", "chkpt-core"]
    needs_bench_ops = False
    needs_bench_catalog = False
    needs_save_pipeline = False

    for name in selected:
        kind = SCENARIOS[name].kind
        if kind == "bench_ops":
            needs_bench_ops = True
        elif kind == "bench_catalog":
            needs_bench_catalog = True
        elif kind == "save_pipeline":
            needs_save_pipeline = True

    if needs_bench_ops:
        cmd.extend(["--example", "bench_ops"])
    if needs_bench_catalog:
        cmd.extend(["--example", "bench_catalog"])
    if needs_save_pipeline:
        cmd.extend(["--bench", "save_pipeline"])

    return cmd


def cmd_list() -> int:
    print("Fixtures:")
    for name, spec in FIXTURES.items():
        print(f"  {name}: {spec.kind} ({spec.files} files, {spec.dirs} dirs)")
    print()
    print("Scenarios:")
    for name, scenario in SCENARIOS.items():
        fixture = f" fixture={scenario.fixture}" if scenario.fixture else ""
        print(f"  {name}: {scenario.kind}{fixture} - {scenario.description}")
    return 0


def cmd_fixtures(args: argparse.Namespace) -> int:
    selected = scenario_names_from_args(args.fixtures, sorted(FIXTURES.keys()))
    created = generate_selected_fixtures(selected)
    for name in selected:
        print(f"{name}: {created[name]}")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    selected = scenario_names_from_args(args.scenarios, DEFAULT_SCENARIOS)
    _payload, results_path = execute_benchmark_run(
        args.label,
        selected,
        args.skip_build,
        REPO_ROOT,
        persist_run_dir=True,
    )
    assert results_path is not None
    print(results_path)
    return 0


def cmd_compare(args: argparse.Namespace) -> int:
    before = load_result(args.before)
    after = load_result(args.after)
    rendered = compare_runs(before, after, args.stat)
    if args.output:
        output_path = Path(args.output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(rendered, encoding="utf-8")
    sys.stdout.write(rendered)
    return 0


def cmd_compare_commits(args: argparse.Namespace) -> int:
    if args.rounds < 1:
        raise ValueError("--rounds must be at least 1")

    selected = scenario_names_from_args(args.scenarios, DEFAULT_SCENARIOS)
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    base_dir = Path(tempfile.mkdtemp(prefix=f"chkpt-ab-{sanitize_label(args.label)}-"))
    before_worktree = base_dir / "before"
    after_worktree = base_dir / "after"
    before_fixed_bench_ops: Path | None = None
    after_fixed_bench_ops: Path | None = None

    before_runs: list[dict[str, Any]] = []
    after_runs: list[dict[str, Any]] = []
    schedule: list[str] = []
    bench_ops_selected = [name for name in selected if SCENARIOS[name].kind == "bench_ops"]
    build_selected = [name for name in selected if SCENARIOS[name].kind != "bench_ops"]

    try:
        create_detached_worktree(before_worktree, args.before_ref)
        create_detached_worktree(after_worktree, args.after_ref)

        if not args.skip_build and build_selected:
            print(f"Building before ref {args.before_ref}...")
            build_benchmark_artifacts(before_worktree, build_selected)
            print(f"Building after ref {args.after_ref}...")
            build_benchmark_artifacts(after_worktree, build_selected)

        if bench_ops_selected:
            print(f"Building fixed bench_ops harness for {args.before_ref}...")
            before_fixed_bench_ops = build_fixed_bench_ops_runner(base_dir / "before-bench-ops", before_worktree)
            print(f"Building fixed bench_ops harness for {args.after_ref}...")
            after_fixed_bench_ops = build_fixed_bench_ops_runner(base_dir / "after-bench-ops", after_worktree)

        for round_index in range(args.rounds):
            order = ["before", "after"] if round_index % 2 == 0 else ["after", "before"]
            for side in order:
                repo_root = before_worktree if side == "before" else after_worktree
                fixed_bench_ops_binary = before_fixed_bench_ops if side == "before" else after_fixed_bench_ops
                run_label = f"{args.label}-{side}-r{round_index + 1}"
                print(f"Round {round_index + 1}/{args.rounds}: running {side}...")
                payload, _ = execute_benchmark_run(
                    run_label,
                    selected,
                    skip_build=True,
                    repo_root=repo_root,
                    persist_run_dir=False,
                    fixed_bench_ops_binary=fixed_bench_ops_binary,
                )
                schedule.append(f"round={round_index + 1}:{side}")
                if side == "before":
                    before_runs.append(payload)
                else:
                    after_runs.append(payload)

        before_aggregate = aggregate_result_payloads(f"{args.label}-before", before_runs)
        after_aggregate = aggregate_result_payloads(f"{args.label}-after", after_runs)
        before_aggregate["ab_metadata"] = {
            "ref": args.before_ref,
            "rounds": args.rounds,
            "schedule": schedule,
        }
        after_aggregate["ab_metadata"] = {
            "ref": args.after_ref,
            "rounds": args.rounds,
            "schedule": schedule,
        }

        run_dir = RUNS_ROOT / f"{timestamp}-{sanitize_label(args.label)}-ab"
        ensure_dir(run_dir)
        before_path = run_dir / "before.results.json"
        after_path = run_dir / "after.results.json"
        report_path = run_dir / "comparison.md"
        before_path.write_text(json.dumps(before_aggregate, indent=2, sort_keys=True), encoding="utf-8")
        after_path.write_text(json.dumps(after_aggregate, indent=2, sort_keys=True), encoding="utf-8")

        rendered = compare_runs(before_aggregate, after_aggregate, args.stat)
        report_header = "\n".join(
            [
                f"# Alternating Benchmark Comparison: {args.label}",
                "",
                f"- Before ref: `{args.before_ref}`",
                f"- After ref: `{args.after_ref}`",
                f"- Rounds: `{args.rounds}`",
                f"- Schedule: `{', '.join(schedule)}`",
                "",
            ]
        )
        rendered = report_header + rendered
        report_path.write_text(rendered, encoding="utf-8")

        if args.output:
            output_path = Path(args.output)
            output_path.parent.mkdir(parents=True, exist_ok=True)
            output_path.write_text(rendered, encoding="utf-8")

        sys.stdout.write(rendered)
        sys.stdout.write(f"\nArtifacts: {run_dir}\n")
        return 0
    finally:
        for worktree in (before_worktree, after_worktree):
            if worktree.exists():
                try:
                    remove_worktree(worktree)
                except (RuntimeError, OSError):
                    pass
        shutil.rmtree(base_dir, ignore_errors=True)


def main() -> int:
    args = parse_args()
    if args.command == "list":
        return cmd_list()
    if args.command == "fixtures":
        return cmd_fixtures(args)
    if args.command == "run":
        return cmd_run(args)
    if args.command == "compare":
        return cmd_compare(args)
    if args.command == "compare-commits":
        return cmd_compare_commits(args)
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
