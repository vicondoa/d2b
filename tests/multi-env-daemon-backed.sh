#!/usr/bin/env bash
# tests/multi-env-daemon-backed.sh — Layer-1 gate for the W3 s5
# daemon-backed `examples/multi-env` variant.
#
# Asserts:
#   1. `nix flake check` of the example flake passes (both the legacy
#      `demo` variant and the new `multi-env-daemon-experimental`
#      variant evaluate cleanly).
#   2. The daemon-experimental variant's emitted `host.json` propagates
#      the v0.4.0 env-level network knobs from `nixling.envs.work`:
#        * `mtu == 1400`
#        * `mssClamp == 1360`  (derived integer from mtu - 40 when the
#          per-env `mssClamp = true` opt-in is set; the on-disk
#          schema stores the resolved integer per
#          docs/reference/schemas/v1/host.json)
#        * `lan.allowEastWest == true`
#        * `lan.effectiveEastWest == true` (double opt-in satisfied)
#      and the site-level acknowledgement:
#        * `site.allowUnsafeEastWest == true`
#   3. Per-TAP `bridgePortFlags` for the daemon variant work env:
#        * `workload-lan` role: east-west enabled (`isolated = false`,
#          `neighSuppress = false`, `learning = true`,
#          `unicastFlood = true`)
#        * `net-vm-lan` role: gateway-facing defaults stay open
#          (`isolated = false`, `neighSuppress = false`,
#          `learning = true`, `unicastFlood = true`)
#        * `uplink` role: point-to-point hardening stays on
#          (`isolated = true`, `neighSuppress = true`,
#          `learning = false`, `unicastFlood = false`)
#   4. Emitted `vms.json` (manifest) for the daemon-experimental
#      variant does NOT contain a `microvm@work-app.service` or
#      `nixling@work-app.service` reference (work-app is daemon-
#      supervised; single-writer invariant per plan §"W3 daemon-vs-
#      legacy migration boundary").
#   5. Emitted `processes.json` for the daemon-experimental variant
#      drops the per-node systemd `unit` field for every node of the
#      daemon-supervised `work-app` VM, while keeping the unit field
#      for the systemd-supervised `personal-app` VM.
#   6. Legacy variant (`nixosConfigurations.demo`) keeps the v0.4.0
#      systemd-supervised contract intact: `processes.json` still
#      contains `microvm@work-app.service` and `microvm@personal-app.service`.
#
# This script does NOT mutate host state; it only evaluates the
# in-tree example flake. Scratch state goes under repo-local
# scratch via `nl_mktemp` so the W2fu4 H8/H9 reaper can clean it.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=./lib.sh
. "$ROOT/tests/lib.sh"

log "==> tests/multi-env-daemon-backed.sh"

EXAMPLE_DIR="$ROOT/examples/multi-env"

# ----- (1) flake check -------------------------------------------------
log "step 1: nix flake check on $EXAMPLE_DIR"
if ! nix flake check --no-build --all-systems --no-write-lock-file \
      "$EXAMPLE_DIR" 2> >(grep -v 'evaluation warning' >&2); then
  fail "nix flake check failed for $EXAMPLE_DIR"
fi
ok "flake check passes for $EXAMPLE_DIR (both variants)"

# ----- helpers ---------------------------------------------------------
eval_attr() {
  # $1 = variant name (e.g. demo / multi-env-daemon-experimental)
  # $2 = attribute path under config.nixling._bundle.<attr>.jsonText
  local variant="$1" attr="$2"
  ( cd "$EXAMPLE_DIR" && \
    nix eval --raw --no-write-lock-file \
      ".#nixosConfigurations.${variant}.config.nixling._bundle.${attr}.jsonText" \
    2> >(grep -v 'evaluation warning' >&2) )
}

eval_manifest_text() {
  local variant="$1"
  ( cd "$EXAMPLE_DIR" && \
    nix eval --raw --no-write-lock-file \
      ".#nixosConfigurations.${variant}.config.nixling._manifestPkg.text" \
    2> >(grep -v 'evaluation warning' >&2) )
}

SCRATCH=$(nl_mktemp .nl-multi-env-daemon.XXXXXX) || fail "nl_mktemp failed"

# ----- (2,3) host.json propagation ------------------------------------
log "step 2-3: assert host.json env-level propagation (daemon variant)"
HOST_JSON="$SCRATCH/host.json"
if ! eval_attr multi-env-daemon-experimental hostJson > "$HOST_JSON"; then
  fail "could not eval host.json for multi-env-daemon-experimental"
fi
[ -s "$HOST_JSON" ] || fail "host.json eval produced empty output"

# site.allowUnsafeEastWest
sue=$(jq -r '.site.allowUnsafeEastWest' "$HOST_JSON")
[ "$sue" = "true" ] || fail "site.allowUnsafeEastWest expected true, got '$sue'"
ok "site.allowUnsafeEastWest == true"

# per-env work knobs
work=$(jq -c '.environments[] | select(.env=="work")' "$HOST_JSON")
[ -n "$work" ] || fail "no environments[] entry for env=work"

mtu=$(jq -r '.mtu' <<<"$work")
[ "$mtu" = "1400" ] || fail "envs.work.mtu expected 1400, got '$mtu'"
ok "envs.work.mtu == 1400"

mss=$(jq -r '.mssClamp' <<<"$work")
[ "$mss" = "1360" ] || \
  fail "envs.work.mssClamp expected 1360 (= mtu - 40 with mssClamp=true), got '$mss'"
ok "envs.work.mssClamp == 1360 (mssClamp opt-in propagated)"

aew=$(jq -r '.lan.allowEastWest' <<<"$work")
[ "$aew" = "true" ] || fail "envs.work.lan.allowEastWest expected true, got '$aew'"
ok "envs.work.lan.allowEastWest == true"

eff=$(jq -r '.lan.effectiveEastWest' <<<"$work")
[ "$eff" = "true" ] || \
  fail "envs.work.lan.effectiveEastWest expected true (double opt-in), got '$eff'"
ok "envs.work.lan.effectiveEastWest == true (double opt-in)"

# bridge-port flags for env=work
for spec in \
  'workload-lan isolated false' \
  'workload-lan neighSuppress false' \
  'workload-lan learning true' \
  'workload-lan unicastFlood true' \
  'net-vm-lan isolated false' \
  'net-vm-lan neighSuppress false' \
  'net-vm-lan learning true' \
  'net-vm-lan unicastFlood true' \
  'uplink isolated true' \
  'uplink neighSuppress true' \
  'uplink learning false' \
  'uplink unicastFlood false'
do
  set -- $spec
  role=$1
  field=$2
  expected=$3
  value=$(jq -r --arg r "$role" --arg f "$field" \
    '.bridgePortFlags[] | select(.role==$r) | .[$f]' <<<"$work")
  [ "$value" = "$expected" ] || \
    fail "envs.work bridgePortFlags[$role].$field expected $expected, got '$value'"
  ok "envs.work bridgePortFlags[$role].$field == $expected"
done

# Sanity: the safe `personal` env keeps default isolation (no east-west
# opt-in). This guards against the daemon variant accidentally relaxing
# isolation for envs that did not opt in.
personal=$(jq -c '.environments[] | select(.env=="personal")' "$HOST_JSON")
[ -n "$personal" ] || fail "no environments[] entry for env=personal"
peff=$(jq -r '.lan.effectiveEastWest' <<<"$personal")
[ "$peff" = "false" ] || \
  fail "envs.personal.lan.effectiveEastWest expected false (no opt-in), got '$peff'"
for spec in \
  'workload-lan isolated true' \
  'workload-lan neighSuppress true' \
  'workload-lan learning true' \
  'workload-lan unicastFlood false'
do
  set -- $spec
  role=$1
  field=$2
  expected=$3
  value=$(jq -r --arg r "$role" --arg f "$field" \
    '.bridgePortFlags[] | select(.role==$r) | .[$f]' <<<"$personal")
  [ "$value" = "$expected" ] || \
    fail "envs.personal bridgePortFlags[$role].$field expected $expected, got '$value'"
  ok "envs.personal bridgePortFlags[$role].$field == $expected"
done
ok "envs.personal stays isolated (negative control)"

# ----- (4) vms.json: no microvm@/nixling@ unit info for work-app -------
log "step 4: assert vms.json drops microvm@work-app / nixling@work-app"
VMS_JSON="$SCRATCH/vms.json"
if ! eval_manifest_text multi-env-daemon-experimental > "$VMS_JSON"; then
  fail "could not eval vms.json for multi-env-daemon-experimental"
fi
[ -s "$VMS_JSON" ] || fail "vms.json eval produced empty output"

if grep -q 'microvm@work-app' "$VMS_JSON"; then
  fail "vms.json unexpectedly contains 'microvm@work-app' (daemon-supervised VM must not surface a systemd unit reference)"
fi
if grep -q 'nixling@work-app\.' "$VMS_JSON"; then
  fail "vms.json unexpectedly contains 'nixling@work-app.' (daemon-supervised VM must not surface a systemd unit reference)"
fi
ok "vms.json contains no microvm@work-app / nixling@work-app reference"

# ----- (5) processes.json: unit fields gated on supervisor -------------
log "step 5: assert processes.json unit fields are gated on supervisor"
PROC_JSON="$SCRATCH/processes-daemon.json"
if ! eval_attr multi-env-daemon-experimental processesJson > "$PROC_JSON"; then
  fail "could not eval processes.json for multi-env-daemon-experimental"
fi
[ -s "$PROC_JSON" ] || fail "processes.json eval produced empty output"

# Daemon-supervised work-app: no node carries a `unit` field.
work_app_units=$(jq -r \
  '[.vms[] | select(.vm=="work-app") | .nodes[] | .unit // empty] | length' \
  "$PROC_JSON")
[ "$work_app_units" = "0" ] || \
  fail "processes.json work-app expected 0 nodes with unit, got $work_app_units"
ok "processes.json work-app (supervisor=nixlingd) emits no node-level unit fields"

# Systemd-supervised personal-app: still carries unit fields.
personal_units=$(jq -r \
  '[.vms[] | select(.vm=="personal-app") | .nodes[] | .unit // empty] | length' \
  "$PROC_JSON")
[ "$personal_units" -ge 1 ] || \
  fail "processes.json personal-app expected ≥1 node with unit, got $personal_units"
ok "processes.json personal-app (supervisor=systemd) keeps node-level unit fields"

# And specifically: microvm@personal-app.service is still emitted.
if ! jq -e '[.vms[] | select(.vm=="personal-app") | .nodes[] | .unit] | index("microvm@personal-app.service") != null' \
      "$PROC_JSON" >/dev/null; then
  fail "processes.json personal-app missing microvm@personal-app.service unit reference"
fi
ok "processes.json personal-app retains microvm@personal-app.service"

# Daemon-supervised work-app: no microvm@ reference at all.
if grep -q 'microvm@work-app' "$PROC_JSON"; then
  fail "processes.json unexpectedly contains microvm@work-app for daemon-supervised VM"
fi
ok "processes.json carries no microvm@work-app for daemon-supervised VM"

# ----- (6) legacy variant unchanged ----------------------------------
log "step 6: assert legacy variant keeps per-VM systemd unit info"
LEGACY_PROC="$SCRATCH/processes-legacy.json"
if ! eval_attr demo processesJson > "$LEGACY_PROC"; then
  fail "could not eval processes.json for legacy demo variant"
fi
[ -s "$LEGACY_PROC" ] || fail "legacy processes.json eval produced empty output"

for vm in work-app personal-app; do
  if ! jq -e --arg u "microvm@${vm}.service" \
        '[.vms[] | select(.vm==$ARGS.named.v) | .nodes[] | .unit] | index($u) != null' \
        --arg v "$vm" "$LEGACY_PROC" >/dev/null; then
    fail "legacy variant processes.json missing microvm@${vm}.service unit reference"
  fi
done
ok "legacy variant retains per-VM microvm@<vm>.service unit references"

# Cleanup is handled by nl_mktemp's registered cleanup; nothing further.
log "OK tests/multi-env-daemon-backed.sh"
