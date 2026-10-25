#!/usr/bin/env bash
# Measures the shipped artifacts against the NFR budgets (T7): the `rusty-harness`
# binary as shipped, profile `dist` (< 25 MB) and the npm package's wasm (< 3 MB gzipped).
#
#   scripts/measure.sh            build both, print a markdown table, fail over budget
#   scripts/measure.sh --no-build measure what target/ and dist/ already hold
set -euo pipefail
cd "$(dirname "$0")/.."

binary_budget=$((25 * 1000 * 1000))
wasm_budget=$((3 * 1000 * 1000))

if [[ "${1:-}" != "--no-build" ]]; then
  cargo build --profile dist --locked -p rusty_harness --features cli --bin rusty-harness
  (cd crates/rusty-harness-wasm && bun run build > /dev/null)
fi

exe=target/dist/rusty-harness
[[ -f "$exe.exe" ]] && exe="$exe.exe"
wasm=crates/rusty-harness-wasm/dist/web/rusty_harness_bg.wasm
binary_bytes=$(wc -c < "$exe" | tr -d ' ')
wasm_bytes=$(wc -c < "$wasm" | tr -d ' ')
wasm_gz=$(gzip -9 -c "$wasm" | wc -c | tr -d ' ')

status=0
verdict() { if (($1 <= $2)); then echo "within"; else echo "OVER"; status=1; fi; }
echo "| Artifact | Bytes | Budget | Verdict |"
echo "|---|---|---|---|"
echo "| \`rusty-harness\` (profile dist) | $binary_bytes | $binary_budget | $(verdict "$binary_bytes" "$binary_budget") |"
echo "| \`rusty_harness_bg.wasm\` (gzip -9) | $wasm_gz (raw $wasm_bytes) | $wasm_budget | $(verdict "$wasm_gz" "$wasm_budget") |"
exit "$status"
