#!/usr/bin/env bash
# tests/wave-evidence-schema-eval.sh— doc/schema drift gate.
#
# Asserts every readiness wave declared in
# nixos-modules/options-daemon.nix:readinessWaveSpecs has a matching
# per-wave row in docs/reference/wave-evidence-schema.md. Adding a
# new wave to the validator without documenting it (or vice versa)
# fails this gate.
#
# Layer 1, eval-only. No Nix eval, no daemon, no broker.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/wave-evidence-schema-eval.sh"

OPTIONS_FILE="$ROOT/nixos-modules/options-daemon.nix"
DOC_FILE="$ROOT/docs/reference/wave-evidence-schema.md"
SCHEMA_FILE="$ROOT/docs/reference/wave-evidence-schema.json"

for f in "$OPTIONS_FILE" "$DOC_FILE" "$SCHEMA_FILE"; do
  if [ ! -r "$f" ]; then
    echo "wave-evidence-schema-eval: missing or unreadable: $f" >&2
    exit 1
  fi
done

# Extract wave keys from the readinessWaveSpecs block. Top-level keys
# sit at 4-space indent of the form:
#     <name> = {
waves=$(
  awk '
    /readinessWaveSpecs = \{/ { in_block = 1; next }
    in_block && /^[[:space:]]{2}\};/ { in_block = 0 }
    in_block {
      if (match($0, /^[[:space:]]{4}([A-Za-z][A-Za-z0-9_]*)[[:space:]]*=[[:space:]]*\{/, m)) {
        print m[1]
      }
    }
  ' "$OPTIONS_FILE"
)

if [ -z "$waves" ]; then
  echo "wave-evidence-schema-eval: failed to parse any waves from $OPTIONS_FILE" >&2
  echo "    (expected readinessWaveSpecs block with 4-space-indented '<name> = {' rows)" >&2
  exit 1
fi

missing=0
for wave in $waves; do
  # Per-wave inventory rows are formatted as: `| \`<wave>\` |`.
  if ! grep -qE "^\|[[:space:]]+\`${wave}\`[[:space:]]+\|" "$DOC_FILE"; then
    echo "wave-evidence-schema-eval: $DOC_FILE is missing a per-wave inventory row for \`${wave}\`" >&2
    missing=$((missing + 1))
  fi
done

if [ "$missing" -gt 0 ]; then
  echo "wave-evidence-schema-eval: $missing wave(s) declared in $OPTIONS_FILE have no row in $DOC_FILE" >&2
  echo "    Add a row of the form '| \`<wave>\` | ... | ... |' under '## Per-wave inventory'." >&2
  exit 1
fi

# Sanity-check the JSON Schema companion is parseable and declares
# the three required fields the validator enforces.
required=$(jq -r '.required | sort | join(",")' "$SCHEMA_FILE")
expected="operatorSignature,timestamp,wave"
if [ "$required" != "$expected" ]; then
  echo "wave-evidence-schema-eval: $SCHEMA_FILE .required = [$required], expected [$expected]" >&2
  exit 1
fi

echo "wave-evidence-schema-eval: OK ($(echo "$waves" | wc -w) wave(s) cross-checked)"
