#!/usr/bin/env bash
# tests/store-sync-export-eval.sh — static gate for the StoreSync-only
# observability export wiring on the HOST side.
#
# Companion to the broker-side Rust unit tests in
# `packages/nixling-priv-broker/src/ops/store_sync_export.rs` (which
# assert the exported JSON key-set equals the allow-list and that the
# redaction fields are absent). This gate asserts the host Alloy
# config in `nixos-modules/components/observability/host.nix`:
#
#   1. Alloy reads the StoreSync export glob
#      (`<stateDir>/observability/store-sync/store-sync-*.jsonl`) via a
#      `local.file_match` + `loki.source.file` pair, and forwards it
#      into the existing host->stack OTLP/Loki receiver.
#   2. Alloy does NOT read the unified broker audit log
#      (`<stateDir>/audit/broker-*.jsonl`), the privileged daemon
#      socket (`priv.sock`), or any nixlingd state — and alloy is
#      never granted nixlingd group membership.
#   3. The StoreSync stream labels are exactly the host singleton set
#      {vm=host, env=host, role=host, source=store-sync-audit}. The
#      TARGET vm/env stay in JSON content as target_vm/target_env and
#      are NEVER promoted to Loki stream labels.
#   4. The `alloy` identity gets focused read/traverse ACLs on the
#      export directory ONLY; the broker audit directory gets no alloy
#      ACL.
#
# Pure text-grep; it does not build or evaluate the module.
#
# Run via:
#   bash tests/store-sync-export-eval.sh

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
# a positive check nor trip a negative one (the let-bindings document
# the broker audit path in prose, which must NOT count as wiring).
code_only() {
  sed -e 's/[[:space:]]#.*$//' -e '/^[[:space:]]*#/d' "$HOST"
}
CODE=$(code_only)

want() {  # want <description> <ERE>
  if grep -Eq "$2" <<<"$CODE"; then ok "$1"; else fail "$1 (missing /$2/)"; fi
}
deny() {  # deny <description> <ERE>
  if grep -Eq "$2" <<<"$CODE"; then fail "$1 (found /$2/)"; else ok "$1"; fi
}

# --- 1. Reads the StoreSync export glob -------------------------------
want "export dir resolves under <stateDir>/observability/store-sync" \
  'storeSyncExportDir[[:space:]]*=.*/observability/store-sync"'
want "export glob targets store-sync-*.jsonl rotation shape" \
  'storeSyncExportGlob[[:space:]]*=.*/store-sync-\*\.jsonl"'
want "a local.file_match tails the store-sync export" \
  'local\.file_match "store_sync_audit"'
want "file_match __path__ is the store-sync glob (follows new/rotated files)" \
  '"__path__"[[:space:]]*=[[:space:]]*\$\{quote storeSyncExportGlob\}'
want "a loki.source.file consumes the file_match targets" \
  'loki\.source\.file "store_sync_audit"'

# loki.source.file wiring + forward into the existing host->stack path.
SF=$(grep -A4 'loki\.source\.file "store_sync_audit"' <<<"$CODE")
if grep -Eq 'targets[[:space:]]*=[[:space:]]*local\.file_match\.store_sync_audit\.targets' <<<"$SF"; then
  ok "loki.source.file binds local.file_match.store_sync_audit.targets"
else
  fail "loki.source.file does not bind the store_sync_audit file_match targets"
fi
if grep -Eq 'forward_to[[:space:]]*=[[:space:]]*\[otelcol\.receiver\.loki\.journal\.receiver\]' <<<"$SF"; then
  ok "store-sync logs forward into the host->stack loki receiver"
else
  fail "store-sync loki.source.file does not forward into otelcol.receiver.loki.journal.receiver"
fi

# --- 2. Does NOT read broker audit / daemon socket / nixlingd ---------
deny "Alloy config never references the broker audit log path" \
  'audit/broker'
deny "Alloy config never globs broker-*.jsonl" \
  'broker-\*'
deny "Alloy config never references the privileged daemon socket" \
  'priv\.sock'
deny "obs host wiring never references nixlingd (no alloy->nixlingd grant)" \
  'nixlingd'

# --- 3. Host-singleton labels; target_vm/target_env stay in content ---
# Slice the path_targets map for the store-sync source.
PT=$(awk '
  /local\.file_match "store_sync_audit"/ { f = 1 }
  f { print }
  f && /\}\]/ { exit }
' <<<"$CODE")
want_pt() {  # want_pt <description> <ERE>
  if grep -Eq "$2" <<<"$PT"; then ok "$1"; else fail "$1 (missing /$2/)"; fi
}
deny_pt() {
  if grep -Eq "$2" <<<"$PT"; then fail "$1 (found /$2/)"; else ok "$1"; fi
}
want_pt "label vm is the host singleton" '"vm"[[:space:]]*=[[:space:]]*"host"'
want_pt "label env is the host singleton" '"env"[[:space:]]*=[[:space:]]*"host"'
want_pt "label role is the host singleton" '"role"[[:space:]]*=[[:space:]]*"host"'
want_pt "label source is store-sync-audit" '"source"[[:space:]]*=[[:space:]]*"store-sync-audit"'
deny_pt "target_vm is NOT promoted to a stream label" 'target_vm'
deny_pt "target_env is NOT promoted to a stream label" 'target_env'

# --- 4. Focused alloy ACL on the export dir; none on the audit dir ----
want "alloy gets traverse (--x) on the state dir" \
  'setfacl -m "u:alloy:--x" "\$state_dir"'
want "alloy gets traverse (--x) on the observability dir" \
  'setfacl -m "u:alloy:--x" "\$obs_dir"'
want "alloy gets read+traverse (r-x) on the export dir" \
  'setfacl -m "u:alloy:r-x" "\$export_dir"'
want "rotated export files inherit alloy read via a default ACL" \
  'setfacl -d -m "u:alloy:r--" "\$export_dir"'
deny "no alloy ACL is granted on any audit path" \
  'setfacl.*alloy.*audit'

log "summary: PASS=$PASS FAIL=$FAIL"
if (( FAIL > 0 )); then
  exit 1
fi
exit 0
