#!/usr/bin/env bash
# The Temporal workflow history of a demo task (R32.6), scrubbed for the repository:
# payload blobs become "<payload>", the Cloud address and namespace become placeholders.
#
#   scripts/demo-history.sh <workflow-id> <out.json>
#
# Reads the same references the demo config names: the address and namespace from
# ~/.config/rusty-harness/temporal-{address,namespace} (or TEMPORAL_ADDRESS /
# TEMPORAL_NAMESPACE), the API key from the keyring entry `service temporal project
# rusty-harness` (TEMPORAL_API_KEY when there is none). The key reaches the CLI through its environment,
# never its arguments. Uses `temporal` on PATH, else the one the embedded mode cached.
set -euo pipefail
workflow="${1:?workflow id}"
out="${2:?output file}"

address="${TEMPORAL_ADDRESS:-$(cat ~/.config/rusty-harness/temporal-address)}"
namespace="${TEMPORAL_NAMESPACE:-$(cat ~/.config/rusty-harness/temporal-namespace)}"
# The keyring entry the demo config names first, as the harness reads it; the environment
# only when there is none.
from_keyring="$(secret-tool lookup service temporal project rusty-harness 2>/dev/null || true)"
TEMPORAL_API_KEY="${from_keyring:-${TEMPORAL_API_KEY:-}}"
[[ -n "$TEMPORAL_API_KEY" ]] || { echo "no Temporal API key in the keyring or TEMPORAL_API_KEY" >&2; exit 1; }
export TEMPORAL_API_KEY

cli="$(command -v temporal || true)"
if [[ -z "$cli" ]]; then
  cli="$(find ~/.cache/rusty-harness/temporal -maxdepth 1 -type f -perm -u+x | head -1)"
fi
[[ -n "$cli" ]] || { echo "no temporal CLI: install it or run the embedded mode once" >&2; exit 1; }

set -o pipefail
"$cli" workflow show --workflow-id "$workflow" --address "$address" \
  --namespace "$namespace" --tls --output json |
  python3 -c '
import json, sys
address, namespace = sys.argv[1], sys.argv[2]
def scrub(node):
    if isinstance(node, dict):
        return {k: ("<payload>" if k == "data" and isinstance(v, str) else scrub(v)) for k, v in node.items()}
    if isinstance(node, list):
        return [scrub(v) for v in node]
    if isinstance(node, str):
        return node.replace(address, "<temporal-address>").replace(namespace, "<temporal-namespace>")
    return node
json.dump(scrub(json.load(sys.stdin)), sys.stdout, indent=2)
print()
' "$address" "$namespace" > "$out"
echo "wrote $out ($(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(len(d.get("events", d if isinstance(d, list) else [])))' "$out") events)"
