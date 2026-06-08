#!/usr/bin/env bash
# Broad capabilities require explicit caps + ADR row.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCHEMA_DIR=${SCHEMA_DIR:-$ROOT/docs/reference/schemas/v1}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if [ ! -f "$SCHEMA_DIR/processes.json" ] || [ ! -f "$SCHEMA_DIR/minijail-profile.json" ]; then
  log "schemas absent — skipping static-invariant-broad-caps (W1 unstaged)"
  exit 0
fi

scratch=$(nl_mktemp .broad-caps-invariant.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
positive=$scratch/positive.json
negative=$scratch/negative.json
cat > "$positive" <<'JSON'
{"profiles":[{"role":"tap-broker","longLived":true,"caps":["CAP_NET_ADMIN"],"adr":"ADR 0004"}]}
JSON
cat > "$negative" <<'JSON'
{"profiles":[{"role":"runner","longLived":true,"caps":["CAP_SYS_ADMIN"]}]}
JSON

jq_filter='
  def broad: ["CAP_SYS_ADMIN", "CAP_NET_ADMIN"];
  def caps: ((.caps // .capabilities // .linuxCapabilities // .linux_capabilities // []) | map(tostring));
  def requested_broad: [caps[] | select(. as $c | broad | index($c))];
  def has_adr: ((.adr // .adrRef // .adr_ref // .capabilityAdr // .capability_adr // "") | tostring | test("(ADR|adr)[ -]?[0-9]{4}|[0-9]{4}"));
  [.. | objects | select((requested_broad | length) > 0) | select(has_adr | not)]
'

if [ "$(jq "$jq_filter | length" "$positive")" -eq 0 ]; then
  ok "static-invariant-broad-caps: positive fixture accepted"
else
  fail "static-invariant-broad-caps: positive fixture rejected"
fi
if [ "$(jq "$jq_filter | length" "$negative")" -gt 0 ]; then
  ok "static-invariant-broad-caps: negative fixture rejected"
else
  fail "static-invariant-broad-caps: negative fixture accepted"
fi

if jq -e '.. | objects | select(.enum? and ((.enum | index("CAP_SYS_ADMIN")) or (.enum | index("CAP_NET_ADMIN"))))' "$SCHEMA_DIR/processes.json" "$SCHEMA_DIR/minijail-profile.json" >/dev/null; then
  ok "static-invariant-broad-caps: broad caps are explicit enum values when allowed"
else
  log "static-invariant-broad-caps: no broad caps enum in schemas (no broad-cap carve-outs declared)"
fi
