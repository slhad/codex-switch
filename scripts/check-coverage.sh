#!/usr/bin/env bash
set -euo pipefail

MIN_COVERAGE="${MIN_COVERAGE:-80}"
COVERAGE_DIR="target/coverage"
COVERAGE_TARGET_DIR="target/coverage-target"
PROFRAW_DIR="$COVERAGE_DIR/profraw"
PROFDATA="$COVERAGE_DIR/codex-switch.profdata"

host_triple=$(rustc -vV | sed -n 's/^host: //p')
rust_llvm_tools="$(rustc --print sysroot)/lib/rustlib/$host_triple/bin"
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
RUSTFLAGS="-Cinstrument-coverage" \
LLVM_PROFILE_FILE="$(pwd)/$PROFRAW_DIR/%p-%m.profraw" \
cargo test

"$LLVM_PROFDATA" merge -sparse "$PROFRAW_DIR"/*.profraw -o "$PROFDATA"

mapfile -t OBJECTS < <(find "$COVERAGE_TARGET_DIR/debug/deps" -maxdepth 1 -type f -perm -111 ! -name "*.so" | sort)
if [[ "${#OBJECTS[@]}" -eq 0 ]]; then
  echo "no coverage objects found" >&2
  exit 1
fi

IGNORE_REGEX='/.cargo/registry|/.rustup/|/rustc/|/target/|src/(cli|main|install|process|rate_limit|status|storage|switch|waybar|waybar_config)\.rs'

REPORT_ARGS=()
for object in "${OBJECTS[@]}"; do
  REPORT_ARGS+=("--object" "$object")
done

"$LLVM_COV" report \
  "${REPORT_ARGS[@]}" \
  --instr-profile="$PROFDATA" \
  --ignore-filename-regex="$IGNORE_REGEX"

coverage_json=$("$LLVM_COV" export \
  "${REPORT_ARGS[@]}" \
  --summary-only \
  --instr-profile="$PROFDATA" \
  --ignore-filename-regex="$IGNORE_REGEX")

line_coverage=$(python3 -c 'import json, sys; print(json.load(sys.stdin)["data"][0]["totals"]["lines"]["percent"])' <<<"$coverage_json")

python3 - "$line_coverage" "$MIN_COVERAGE" <<'PY'
import sys
actual = float(sys.argv[1])
minimum = float(sys.argv[2])
if actual + 1e-9 < minimum:
    print(f"coverage {actual:.2f}% is below required {minimum:.2f}%", file=sys.stderr)
    sys.exit(1)
print(f"coverage {actual:.2f}% meets required {minimum:.2f}%")
PY
