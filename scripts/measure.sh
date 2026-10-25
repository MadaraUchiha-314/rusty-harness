#!/usr/bin/env bash
# Measures the NFR targets (T7, Q13: measured, not gates): the `rusty-harness` binary as
# shipped, profile `dist` (< 25 MB), the npm package's wasm (< 3 MB gzipped) and the TUI's
# cold start to the first-message prompt (< 200 ms, median of five, Linux and macOS).
#
#   scripts/measure.sh             build, then print a markdown table
#   scripts/measure.sh --no-build  measure what target/ and dist/ already hold
#   scripts/measure.sh --strict    also exit non-zero when a target is missed
set -euo pipefail
cd "$(dirname "$0")/.."

binary_budget=$((25 * 1000 * 1000))
wasm_budget=$((3 * 1000 * 1000))

build=1 strict=0
for arg in "$@"; do
  case "$arg" in
    --no-build) build=0 ;;
    --strict) strict=1 ;;
    *) echo "usage: $0 [--no-build] [--strict]" >&2; exit 2 ;;
  esac
done

if ((build)); then
  cargo build --profile dist --locked -p rusty_harness --features cli --bin rusty-harness
  (cd crates/rusty-harness-wasm && bun run build > /dev/null)
fi

exe=target/dist/rusty-harness
[[ -f "$exe.exe" ]] && exe="$exe.exe"
wasm=crates/rusty-harness-wasm/dist/web/rusty_harness_bg.wasm
binary_bytes=$(wc -c < "$exe" | tr -d ' ')
wasm_bytes=$(wc -c < "$wasm" | tr -d ' ')
wasm_gz=$(gzip -9 -c "$wasm" | wc -c | tr -d ' ')

cold_budget=200
cold_ms=$(python3 scripts/cold-start.py "$exe" 5)

status=0
# Sets $v to the verdict; updates $status in this shell (no subshell).
verdict() { if (($1 <= $2)); then v=within; else v=OVER; status=1; fi; }
echo "| Target | Measured | Budget | Verdict |"
echo "|---|---|---|---|"
verdict "$binary_bytes" "$binary_budget"
echo "| \`rusty-harness\` (profile dist), bytes | $binary_bytes | $binary_budget | $v |"
verdict "$wasm_gz" "$wasm_budget"
echo "| \`rusty_harness_bg.wasm\` (gzip -9), bytes | $wasm_gz (raw $wasm_bytes) | $wasm_budget | $v |"
verdict "${cold_ms%.*}" "$cold_budget"
echo "| TUI cold start, ms (median of 5) | $cold_ms | $cold_budget | $v |"
if ((strict)); then exit "$status"; fi
