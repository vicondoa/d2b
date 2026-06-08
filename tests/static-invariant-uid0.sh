#!/usr/bin/env bash
# Uid 0 long-lived profiles require start-root + ADR carve-out.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCHEMA_DIR=${SCHEMA_DIR:-$ROOT/docs/reference/schemas/v1}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if [ ! -f "$SCHEMA_DIR/processes.json" ] || [ ! -f "$SCHEMA_DIR/privileges.json" ] || [ ! -f "$SCHEMA_DIR/minijail-profile.json" ]; then
  log "schemas absent — skipping static-invariant-uid0 (W1 unstaged)"
  exit 0
fi

scratch=$(nl_mktemp .uid0-invariant.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
positive=$scratch/positive.json
negative=$scratch/negative.json
cat > "$positive" <<'JSON'
{"profiles":[{"role":"swtpm-flush","longLived":true,"uid":0,"requiresStartRoot":true,"adr":"ADR 0004"}]}
JSON
cat > "$negative" <<'JSON'
{"profiles":[{"role":"runner","longLived":true,"uid":0,"requiresStartRoot":false}]}
JSON

jq_filter='
  def profiles: .. | objects | select(has("uid") or has("requiresStartRoot") or has("user"));
  def uid_value: if has("uid") then .uid elif (.user|type)=="object" and (.user|has("uid")) then .user.uid else null end;
  def long_lived: (.longLived // .long_lived // (if .lifecycle? == "oneshot" then false else true end));
  def has_adr: ((.adr // .adrRef // .adr_ref // .adrCarveOut // .adr_carve_out // .carveOutAdr // .carve_out_adr // "") | tostring | test("(ADR|adr)[ -]?[0-9]{4}|[0-9]{4}"));
  [profiles | select(long_lived != false) | select(uid_value == 0) | select((.requiresStartRoot // .requires_start_root // false) != true or (has_adr | not))]
'

if [ "$(jq "$jq_filter | length" "$positive")" -eq 0 ]; then
  ok "static-invariant-uid0: positive fixture accepted"
else
  fail "static-invariant-uid0: positive fixture rejected"
fi
if [ "$(jq "$jq_filter | length" "$negative")" -gt 0 ]; then
  ok "static-invariant-uid0: negative fixture rejected"
else
  fail "static-invariant-uid0: negative fixture accepted"
fi

schema_hits=$(jq -s 'flatten | [.[] | .. | objects | select((.properties.uid? or .properties.requiresStartRoot? or .properties.requires_start_root?) and ((.properties.adr? or .properties.adrRef? or .properties.adr_ref? or .properties.adrCarveOut? or .properties.adr_carve_out? or .properties.carveOutAdr? or .properties.carve_out_adr?) | not))] | length' "$SCHEMA_DIR/processes.json" "$SCHEMA_DIR/minijail-profile.json")
if [ "$schema_hits" -eq 0 ]; then
  ok "static-invariant-uid0: uid/root-capable schema shapes include ADR carve-out fields"
else
  fail "static-invariant-uid0: uid/root-capable schema shape lacks ADR carve-out field"
fi
