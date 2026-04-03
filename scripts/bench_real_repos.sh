#!/usr/bin/env bash
set -euo pipefail

# Benchmark chkpt against real open-source projects.
# Usage: ./scripts/bench_real_repos.sh [--runs N] [--keep]

RUNS=3
KEEP=false
BENCH_DIR="/tmp/chkpt-bench"
HOME_DIR="/tmp/chkpt-bench-home"
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CHKPT="$SCRIPT_DIR/target/release/chkpt"

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --runs) RUNS="$2"; shift 2 ;;
    --keep) KEEP=true; shift ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# Repos: name, url
REPOS=(
  "React|https://github.com/facebook/react.git"
  "Rust|https://github.com/rust-lang/rust.git"
  "Linux|https://github.com/torvalds/linux.git"
)

# --- Helpers ---

human_size() {
  local bytes=$1
  if (( bytes >= 1073741824 )); then
    printf "%.1f GB" "$(echo "scale=1; $bytes / 1073741824" | bc)"
  elif (( bytes >= 1048576 )); then
    printf "%.1f MB" "$(echo "scale=1; $bytes / 1048576" | bc)"
  else
    printf "%.1f KB" "$(echo "scale=1; $bytes / 1024" | bc)"
  fi
}

# Measure wall-clock seconds for a command (returns float)
measure_time() {
  local start end
  start=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  "$@" > /dev/null 2>&1
  end=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  echo "$end - $start" | bc
}

# Get median of sorted values (newline-separated)
median() {
  local vals=()
  while IFS= read -r v; do vals+=("$v"); done
  local n=${#vals[@]}
  local mid=$((n / 2))
  echo "${vals[$mid]}"
}

format_time() {
  local secs=$1
  if (( $(echo "$secs >= 60" | bc -l) )); then
    local mins=$(echo "$secs / 60" | bc)
    local remainder=$(echo "$secs - $mins * 60" | bc)
    printf "%dm %.1fs" "$mins" "$remainder"
  elif (( $(echo "$secs >= 10" | bc -l) )); then
    printf "%.1fs" "$secs"
  else
    printf "%.2fs" "$secs"
  fi
}

# --- Main ---

echo "=== chkpt Real-World Benchmark ==="
echo ""

# Build release binary
echo "Building chkpt (release)..."
(cd "$SCRIPT_DIR" && cargo build --release -p chkpt-cli 2>&1 | tail -1)
echo "Binary: $CHKPT"
echo ""

mkdir -p "$BENCH_DIR" "$HOME_DIR"

# Results arrays
declare -a R_NAME R_FILES R_SIZE R_COLD R_INCR R_RESTORE R_STORAGE R_RATIO

for repo_entry in "${REPOS[@]}"; do
  IFS='|' read -r name url <<< "$repo_entry"
  repo_dir="$BENCH_DIR/$name"
  store_home="$HOME_DIR/$name"

  echo "--- $name ---"

  # Clone if not cached
  if [[ ! -d "$repo_dir" ]]; then
    echo "  Cloning $url (shallow)..."
    git clone --depth 1 --quiet "$url" "$repo_dir"
  else
    echo "  Using cached clone at $repo_dir"
  fi

  cd "$repo_dir"

  # Measure project stats
  file_count=$(find . -not -path './.git/*' -not -path './.git' -type f | wc -l | tr -d ' ')
  total_bytes=$(find . -not -path './.git/*' -not -path './.git' -type f -exec stat -f%z {} + 2>/dev/null | awk '{s+=$1} END {print s}')
  echo "  Files: $file_count, Size: $(human_size "$total_bytes")"

  # Cold save (median of N runs)
  echo "  Running cold save ($RUNS runs)..."
  cold_times=()
  for ((i=1; i<=RUNS; i++)); do
    rm -rf "$store_home"
    mkdir -p "$store_home"
    t=$(CHKPT_HOME="$store_home" measure_time "$CHKPT" save -m "cold run $i")
    cold_times+=("$t")
    echo "    Run $i: ${t}s"
  done
  cold_median=$(printf '%s\n' "${cold_times[@]}" | sort -n | median)

  # Storage size (after last cold save)
  rm -rf "$store_home"
  mkdir -p "$store_home"
  CHKPT_HOME="$store_home" "$CHKPT" save -m "measure storage" > /dev/null 2>&1
  storage_bytes=$(find "$store_home" -type f -exec stat -f%z {} + 2>/dev/null | awk '{s+=$1} END {print s}')

  # Incremental save (modify 5 files, median of N runs)
  echo "  Running incremental save ($RUNS runs)..."
  incr_times=()
  for ((i=1; i<=RUNS; i++)); do
    # Find 5 source files to modify
    targets=()
    while IFS= read -r f; do targets+=("$f"); done < <(find . -not -path './.git/*' -type f \( -name '*.rs' -o -name '*.js' -o -name '*.c' -o -name '*.h' -o -name '*.py' \) 2>/dev/null | head -5)
    for f in "${targets[@]}"; do
      echo "// chkpt-bench-marker-$i" >> "$f"
    done
    t=$(CHKPT_HOME="$store_home" measure_time "$CHKPT" save -m "incremental run $i")
    incr_times+=("$t")
    echo "    Run $i: ${t}s"
    # Revert changes
    for f in "${targets[@]}"; do
      git checkout -- "$f" 2>/dev/null || true
    done
  done
  incr_median=$(printf '%s\n' "${incr_times[@]}" | sort -n | median)

  # Restore (median of N runs)
  echo "  Running restore ($RUNS runs)..."
  restore_times=()
  for ((i=1; i<=RUNS; i++)); do
    t=$(CHKPT_HOME="$store_home" measure_time "$CHKPT" restore latest)
    restore_times+=("$t")
    echo "    Run $i: ${t}s"
  done
  restore_median=$(printf '%s\n' "${restore_times[@]}" | sort -n | median)

  # Compression ratio
  ratio=$(echo "scale=1; $total_bytes / $storage_bytes" | bc)

  # Store results
  R_NAME+=("$name")
  R_FILES+=("$file_count")
  R_SIZE+=("$(human_size "$total_bytes")")
  R_COLD+=("$(format_time "$cold_median")")
  R_INCR+=("$(format_time "$incr_median")")
  R_RESTORE+=("$(format_time "$restore_median")")
  R_STORAGE+=("$(human_size "$storage_bytes")")
  R_RATIO+=("${ratio}x")

  echo "  Done: cold=$(format_time "$cold_median") incr=$(format_time "$incr_median") restore=$(format_time "$restore_median") storage=$(human_size "$storage_bytes") ratio=${ratio}x"
  echo ""
done

# Print markdown table
echo ""
echo "=== Markdown Table ==="
echo ""
echo "| Project | Files | Size | Cold Save | Incr. Save | Restore | Storage | Ratio |"
echo "|---------|------:|-----:|----------:|-----------:|--------:|--------:|------:|"
for ((i=0; i<${#R_NAME[@]}; i++)); do
  printf "| %s | %s | %s | %s | %s | %s | %s | %s |\n" \
    "${R_NAME[$i]}" "${R_FILES[$i]}" "${R_SIZE[$i]}" \
    "${R_COLD[$i]}" "${R_INCR[$i]}" "${R_RESTORE[$i]}" \
    "${R_STORAGE[$i]}" "${R_RATIO[$i]}"
done
echo ""
echo "> Benchmarked on MacBook Pro (M2 Pro, 16 GB RAM, APFS SSD). Release build, median of $RUNS runs."
echo "> Incremental save: 5 source files modified. Storage = .chkpt store size after cold save (LZ4-compressed, deduplicated)."

# Cleanup
if [[ "$KEEP" == false ]]; then
  echo ""
  echo "Cleaning up..."
  rm -rf "$HOME_DIR"
  # Keep cloned repos for re-runs (they take a while to clone)
  echo "Note: Cloned repos kept at $BENCH_DIR for re-runs. Delete manually if needed."
else
  echo ""
  echo "Keeping benchmark data at $BENCH_DIR and $HOME_DIR"
fi
