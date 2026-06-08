#!/usr/bin/env bash
# Public vms.json exposes only public-safe fields.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
SCHEMA_DIR=${SCHEMA_DIR:-$ROOT/docs/reference/schemas/v1}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if [ ! -f packages/nixling-core/src/bundle.rs ] || [ ! -d "$SCHEMA_DIR" ]; then
  log "schemas absent — skipping static-invariant-world-readable-leak (W1 unstaged)"
  exit 0
fi

scratch=$(nl_mktemp .world-readable-invariant.XXXXXX)
add_cleanup "rm -rf -- \"$scratch\""

if ! manifest=$(nl_smoke_vms_json); then
  fail "static-invariant-world-readable-leak: could not render smoke vms.json"
fi

positive=$scratch/positive.json
negative=$scratch/negative.json
cat > "$positive" <<'JSON'
{"_manifest":{"manifestVersion":3},"corp-vm":{"name":"corp-vm","env":"work","index":10,"sshUser":"alice","sshPort":22,"ipv4":"10.20.0.10","mac":"02:00:00:00:00:0a","isNetVm":false}}
JSON
cat > "$negative" <<'JSON'
{"corp-vm":{"name":"corp-vm","privateKeyPath":"/var/lib/nixling/vms/corp-vm/id_ed25519"}}
JSON

jq_filter='
  def allowed:
    ["_manifest","_observability","name","env","index","hostName","hostname","sshHost","sshPort","sshUser","ipv4","ip","mac","tap","bridge","netVm","isNetVm","routerVm","isRouter","autostart","graphics","tpm","audio","usbip","usbipYubikey","usbipdHostIp","observability","vsockCid","vsockHostSocket","agentSocket","state","status","pendingRestart","closure","current","booted","runner","store","stateDir","apiSocket","gpuSocket","tpmSocket","audioStateFile","audioService","staticIp"];
  [paths(scalars) as $p
    | ($p | map(tostring)) as $sp
    | select(($sp[-1] as $k | allowed | index($k) | not))
    | select(($sp | join(".") | test("^_manifest\\.|^_observability\\.") | not))
    | $sp | join(".")]
'

if [ "$(jq "$jq_filter | length" "$positive")" -eq 0 ]; then
  ok "static-invariant-world-readable-leak: positive fixture accepted"
else
  fail "static-invariant-world-readable-leak: positive fixture rejected"
fi
if [ "$(jq "$jq_filter | length" "$negative")" -gt 0 ]; then
  ok "static-invariant-world-readable-leak: negative fixture rejected"
else
  fail "static-invariant-world-readable-leak: negative fixture accepted"
fi

leaks=$(jq -r "$jq_filter | .[]" "$manifest")
if [ -z "$leaks" ]; then
  ok "static-invariant-world-readable-leak: rendered vms.json field set is public-safe"
else
  printf '%s\n' "$leaks" >&2
  fail "static-invariant-world-readable-leak: rendered vms.json exposes non-allowlisted fields"
fi
