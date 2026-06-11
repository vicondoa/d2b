#!/usr/bin/env bash
# tests/store-sync-export-eval.sh — static gate for the StoreSync-only
# observability export wiring on the HOST side.
#
# Companion to the broker-side Rust unit tests in
# `packages/nixling-priv-broker/src/ops/store_sync_export.rs` (which
# assert the exported JSON key-set equals the allow-list and that the
# redaction fields are absent). This gate asserts the native OTel host
# collector wiring in `nixos-modules/components/observability/host.nix`:
#
#   1. The host collector reads
#      `<stateDir>/observability/store-sync/store-sync-*.jsonl` via a
#      `filelog/store_sync_audit` receiver and forwards it through the
#      host->stack OTLP exporter.
#   2. The collector does NOT read the unified broker audit log
#      (`<stateDir>/audit/broker-*.jsonl`) or privileged daemon socket.
#   3. StoreSync resource attributes are exactly the host singleton set
#      plus `source=store-sync-audit`; target_vm/target_env stay in JSON
#      content and are never promoted to attributes.
#   4. The `nixling-host-otel-collector` identity gets focused
#      read/traverse ACLs on the export directory only.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
HOST="$ROOT/nixos-modules/components/observability/host.nix"

PASS=0
FAIL=0
log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL + 1)); }

if [[ ! -f "$HOST" ]]; then
  fail "missing required file: $HOST"
  exit 1
fi

# Comment-free view of the module so a `#`-comment can neither satisfy
# a positive check nor trip a negative one.
code_only() {
  sed -e 's/[[:space:]]#.*$//' -e '/^[[:space:]]*#/d' "$HOST"
}
CODE=$(code_only)

want() {
  if grep -Eq "$2" <<<"$CODE"; then ok "$1"; else fail "$1 (missing /$2/)"; fi
}
deny() {
  if grep -Eq "$2" <<<"$CODE"; then fail "$1 (found /$2/)"; else ok "$1"; fi
}

want "export dir resolves under <stateDir>/observability/store-sync" \
  'storeSyncExportDir[[:space:]]*=.*/observability/store-sync"'
want "export glob targets store-sync-*.jsonl rotation shape" \
  'storeSyncExportGlob[[:space:]]*=.*/store-sync-\*\.jsonl"'
want "host collector uses a filelog receiver for StoreSync export" \
  '"filelog/store_sync_audit"[[:space:]]*='
want "filelog receiver includes the StoreSync export glob" \
  'include[[:space:]]*=[[:space:]]*\[[[:space:]]*storeSyncExportGlob[[:space:]]*\]'
want "filelog receiver parses JSON log bodies" \
  'type[[:space:]]*=[[:space:]]*"json_parser"'
want "StoreSync logs have a dedicated OTel pipeline" \
  'pipelines\."logs/store_sync_audit"[[:space:]]*='
want "StoreSync logs forward to the existing OTLP exporter" \
  'exporters[[:space:]]*=[[:space:]]*\[[[:space:]]*"otlp"[[:space:]]*\]'

deny "host collector never references the broker audit log path" \
  'audit/broker'
deny "host collector never globs broker-*.jsonl" \
  'broker-\*'
deny "host collector never references the privileged daemon socket" \
  'priv\.sock'

want "StoreSync resource marks vm.name as host" \
  'key[[:space:]]*=[[:space:]]*"vm.name";[[:space:]]*value[[:space:]]*=[[:space:]]*"host"'
want "StoreSync resource marks vm.env as host" \
  'key[[:space:]]*=[[:space:]]*"vm.env";[[:space:]]*value[[:space:]]*=[[:space:]]*"host"'
want "StoreSync resource marks vm.role as host" \
  'key[[:space:]]*=[[:space:]]*"vm.role";[[:space:]]*value[[:space:]]*=[[:space:]]*"host"'
want "StoreSync resource marks service.name as nixling-store-sync" \
  'key[[:space:]]*=[[:space:]]*"service.name";[[:space:]]*value[[:space:]]*=[[:space:]]*"nixling-store-sync"'
want "StoreSync resource marks source as store-sync-audit" \
  'key[[:space:]]*=[[:space:]]*"source";[[:space:]]*value[[:space:]]*=[[:space:]]*"store-sync-audit"'
deny "target_vm is NOT promoted to a resource attribute" \
  'key[[:space:]]*=[[:space:]]*"target_vm"'
deny "target_env is NOT promoted to a resource attribute" \
  'key[[:space:]]*=[[:space:]]*"target_env"'

want "collector gets traverse (--x) on the state dir" \
  'setfacl -m "u:nixling-host-otel-collector:--x" "\$state_dir"'
want "collector gets traverse (--x) on the observability dir" \
  'setfacl -m "u:nixling-host-otel-collector:--x" "\$obs_dir"'
want "collector gets read+traverse (r-x) on the export dir" \
  'setfacl -m "u:nixling-host-otel-collector:r-x" "\$export_dir"'
want "rotated export files inherit collector read via a default ACL" \
  'setfacl -d -m "u:nixling-host-otel-collector:r--" "\$export_dir"'
deny "no collector ACL is granted on any audit path" \
  'setfacl.*nixling-host-otel-collector.*audit'

log "summary: PASS=$PASS FAIL=$FAIL"
if (( FAIL > 0 )); then
  exit 1
fi
exit 0
