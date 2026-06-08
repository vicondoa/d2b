#!/usr/bin/env bash
# manifests/bundle artifacts expose opaque key IDs, not host paths.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCHEMA_DIR=${SCHEMA_DIR:-$ROOT/docs/reference/schemas/v1}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if [ ! -f packages/nixling-core/src/bundle.rs ] || [ ! -d "$SCHEMA_DIR" ]; then
  log "schemas absent — skipping static-invariant-opaque-key-ids (W1 unstaged)"
  exit 0
fi

scratch=$(nl_mktemp .opaque-key-ids.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
manifest=$scratch/vms.json
positive=$scratch/positive.json
negative=$scratch/negative.json
cat > "$positive" <<'JSON'
{"keys":{"ssh":{"key_id":"corp-vm-host-key"}},"secrets":[{"secret_id":"api-token"}]}
JSON
cat > "$negative" <<'JSON'
{"keys":{"ssh":{"privateKeyPath":"/var/lib/nixling/vms/corp-vm/id_ed25519"}},"secret_path":"/run/secrets/token"}
JSON

jq_filter='
  [paths(scalars) as $p
    | ($p[-1] | tostring) as $k
    | select($k | test("(?i)(keyPath|privateKeyPath|secret_path|secretPath|tokenPath|credentialPath)$"))
    | select(getpath($p) | tostring | test("/"))
    | ($p | map(tostring) | join(".")) + "=" + (getpath($p) | tostring)]
'

if [ "$(jq "$jq_filter | length" "$positive")" -eq 0 ]; then
  ok "static-invariant-opaque-key-ids: positive fixture accepted"
else
  fail "static-invariant-opaque-key-ids: positive fixture rejected"
fi
if [ "$(jq "$jq_filter | length" "$negative")" -gt 0 ]; then
  ok "static-invariant-opaque-key-ids: negative fixture rejected"
else
  fail "static-invariant-opaque-key-ids: negative fixture accepted"
fi

if ! manifest=$(nl_smoke_vms_json); then
  fail "static-invariant-opaque-key-ids: could not render smoke vms.json"
fi

violations=$(jq -r "$jq_filter | .[]" "$manifest")
if [ -z "$violations" ]; then
  ok "static-invariant-opaque-key-ids: rendered vms.json has no path-bearing key/secret fields"
else
  printf '%s\n' "$violations" >&2
  fail "static-invariant-opaque-key-ids: path-bearing key/secret fields leaked"
fi
