#!/usr/bin/env bash
set -euo pipefail

MIN_REGION_COVERAGE="${MIN_REGION_COVERAGE:-95}"
MIN_FUNCTION_COVERAGE="${MIN_FUNCTION_COVERAGE:-95}"
MIN_LINE_COVERAGE="${MIN_LINE_COVERAGE:-95}"
MIN_BRANCH_COVERAGE="${MIN_BRANCH_COVERAGE:-95}"
COVERAGE_DIR="target/coverage"
COVERAGE_TARGET_DIR="target/coverage-target"
PROFRAW_DIR="$COVERAGE_DIR/profraw"
PROFDATA="$COVERAGE_DIR/codex-switch.profdata"

host_triple=$(rustc +nightly -vV | sed -n 's/^host: //p')
rust_llvm_tools="$(rustc +nightly --print sysroot)/lib/rustlib/$host_triple/bin"
LLVM_PROFDATA="${LLVM_PROFDATA:-$rust_llvm_tools/llvm-profdata}"
LLVM_COV="${LLVM_COV:-$rust_llvm_tools/llvm-cov}"

if [[ ! -x "$LLVM_PROFDATA" ]]; then
  LLVM_PROFDATA=$(command -v llvm-profdata || true)
fi
if [[ ! -x "$LLVM_COV" ]]; then
  LLVM_COV=$(command -v llvm-cov || true)
fi

if [[ -z "$LLVM_PROFDATA" || ! -x "$LLVM_PROFDATA" ]]; then
  echo "llvm-profdata is required to check coverage; install rustup component llvm-tools-preview" >&2
  exit 1
fi

if [[ -z "$LLVM_COV" || ! -x "$LLVM_COV" ]]; then
  echo "llvm-cov is required to check coverage; install rustup component llvm-tools-preview" >&2
  exit 1
fi

rm -rf "$COVERAGE_DIR" "$COVERAGE_TARGET_DIR"
mkdir -p "$PROFRAW_DIR"

CARGO_TARGET_DIR="$COVERAGE_TARGET_DIR" \
RUSTFLAGS="-Cinstrument-coverage -Zcoverage-options=branch" \
LLVM_PROFILE_FILE="$(pwd)/$PROFRAW_DIR/%p-%m.profraw" \
cargo +nightly test

"$LLVM_PROFDATA" merge -sparse "$PROFRAW_DIR"/*.profraw -o "$PROFDATA"

mapfile -t OBJECTS < <(find "$COVERAGE_TARGET_DIR/debug" -type f -perm -111 -name "codex_switch-*" | sort)
if [[ "${#OBJECTS[@]}" -eq 0 ]]; then
  echo "no coverage objects found" >&2
  exit 1
fi

# Exclude command-surface and external-process/network orchestration modules. Their
# behavior is covered by focused unit tests, while this gate measures the
# deterministic data, JWT, profile, profile-options, and tracker core.
IGNORE_REGEX='/.cargo/registry|/.rustup/|/rustc/|/target/|src/(auto_switch|cli|completions|main|install|process|rate_limit|status|storage|switch|systemd|waybar|waybar_config)\.rs'

REPORT_ARGS=()
for object in "${OBJECTS[@]}"; do
  REPORT_ARGS+=("--object" "$object")
done

"$LLVM_COV" report \
  "${REPORT_ARGS[@]}" \
  --instr-profile="$PROFDATA" \
  --ignore-filename-regex="$IGNORE_REGEX" \
  --show-branch-summary

coverage_json=$("$LLVM_COV" export \
  "${REPORT_ARGS[@]}" \
  --summary-only \
  --instr-profile="$PROFDATA" \
  --ignore-filename-regex="$IGNORE_REGEX")

readarray -t coverage_values < <(
  python3 -c '
import json, sys
metrics = json.load(sys.stdin)["data"][0]["totals"]
for name in ("regions", "functions", "lines", "branches"):
    print(metrics[name]["percent"])
' <<<"$coverage_json"
)

python3 - \
  "${coverage_values[@]}" \
  "$MIN_REGION_COVERAGE" \
  "$MIN_FUNCTION_COVERAGE" \
  "$MIN_LINE_COVERAGE" \
  "$MIN_BRANCH_COVERAGE" <<'PY'
import sys

names = ("region", "function", "line", "branch")
actuals = [float(value) for value in sys.argv[1:5]]
minimums = [float(value) for value in sys.argv[5:9]]
failed = False
for name, actual, minimum in zip(names, actuals, minimums):
    if actual + 1e-9 < minimum:
        print(f"{name} coverage {actual:.2f}% is below required {minimum:.2f}%", file=sys.stderr)
        failed = True
    else:
        print(f"{name} coverage {actual:.2f}% meets required {minimum:.2f}%")
if failed:
    sys.exit(1)
PY
