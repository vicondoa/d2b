#!/usr/bin/env bash
# W1 static invariant: process writable paths must be documented by bundle schema.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCHEMA_DIR=${SCHEMA_DIR:-$ROOT/docs/reference/schemas/v1}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if [ ! -f "$SCHEMA_DIR/processes.json" ] || [ ! -f "$SCHEMA_DIR/bundle.json" ]; then
  log "schemas absent — skipping static-invariant-writable-paths (W1 unstaged)"
  exit 0
fi

scratch=$(nl_mktemp .writable-paths-invariant.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""
positive=$scratch/positive.json
negative=$scratch/negative.json
cat > "$positive" <<'JSON'
{"bundle":{"writablePaths":["/var/lib/nixling/vms/corp-vm"]},"processes":[{"role":"store-sync","writablePaths":["/var/lib/nixling/vms/corp-vm"]}]}
JSON
cat > "$negative" <<'JSON'
{"bundle":{"writablePaths":["/var/lib/nixling/vms/corp-vm"]},"processes":[{"role":"runner","writablePaths":["/run/secrets"]}]}
JSON

jq_filter='
  def declared: [(.bundle.writablePaths // .bundle.writable_paths // .bundle.paths.writable // [])[] | tostring] | unique;
  def used: [(.processes // .profiles // [])[] | (.writablePaths // .writable_paths // .mounts?.writable // [])[] | tostring] | unique;
  used - declared
'

if [ "$(jq "$jq_filter | length" "$positive")" -eq 0 ]; then
  ok "static-invariant-writable-paths: positive fixture accepted"
else
  fail "static-invariant-writable-paths: positive fixture rejected"
fi
if [ "$(jq "$jq_filter | length" "$negative")" -gt 0 ]; then
  ok "static-invariant-writable-paths: negative fixture rejected"
else
  fail "static-invariant-writable-paths: negative fixture accepted"
fi

process_terms=$(jq -r '[.. | objects | (.properties? // {}) | keys[]? | select(test("writable|mount"; "i"))] | unique | .[]' "$SCHEMA_DIR/processes.json")
bundle_terms=$(jq -r '[.. | objects | (.properties? // {}) | keys[]? | select(test("writable|mount|path"; "i"))] | unique | .[]' "$SCHEMA_DIR/bundle.json")
if [ -n "$process_terms" ] && [ -n "$bundle_terms" ]; then
  ok "static-invariant-writable-paths: writable path vocabulary present in processes and bundle schemas"
else
  fail "static-invariant-writable-paths: writable paths must be represented in both processes.json and bundle.json schemas"
fi
