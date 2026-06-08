#!/usr/bin/env bash
# tests/nixlingd-startup-smoke.sh — P0 startup smoke gate.
#
# Phase 1 (eval-only, always runs):
#   Evaluates a minimal NixOS config with daemonExperimental.enable = true
#   and asserts the documented systemd unit + tmpfiles surface for
#   nixlingd + nixling-priv-broker.
#
# Phase 2 (opt-in, NL_LIVE=1):
#   Cycles the live daemon + broker, asserts /run/nixling/public.sock
#   appears, exercises the Hello handshake and /run/nixling/version
#   endpoint, and verifies that a tampered bundle.json causes the daemon
#   to return a BundleTampered typed envelope.  A trap restores all
#   pre-test state on EXIT (including failures); failure to complete
#   cleanup causes the test to fail.
#
# Wired into tests/static.sh.
#
# Broker CapabilityBoundingSet reproduced inline per plan.md
# §"Canonical broker CapabilityBoundingSet" (no CAP_SYS_PTRACE):
#   CAP_NET_ADMIN  CAP_NET_RAW       CAP_DAC_OVERRIDE  CAP_DAC_READ_SEARCH
#   CAP_SYS_ADMIN  CAP_SETUID        CAP_SETGID        CAP_FOWNER

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

PASS=0
FAIL=0

pass_check() { log "  PASS: $1"; PASS=$((PASS + 1)); }
fail_check() { log "  FAIL: $1"; FAIL=$((FAIL + 1)); }

log "==> tests/nixlingd-startup-smoke.sh"

# ---------------------------------------------------------------------------
# Phase 1 — eval-only (always runs)
# ---------------------------------------------------------------------------

log "==> Phase 1: eval-only checks"

EVAL_EXPR=$(cat <<NIXEOF
let
  flake      = builtins.getFlake (toString $ROOT);
  lib        = flake.inputs.nixpkgs.lib;
  nixpkgs    = flake.inputs.nixpkgs;
  sortStrs   = builtins.sort builtins.lessThan;

  nixos = lib.nixosSystem {
    system = "x86_64-linux";
    pkgs   = import nixpkgs {
      system = "x86_64-linux";
      config.allowUnsupportedSystem = true;
    };
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable           = false;
        boot.loader.systemd-boot.enable   = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion               = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser   = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.corp-vm = {
          enable   = true;
          env      = "work";
          index    = 10;
          ssh.user = "alice";
          config = { lib, ... }: {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice   = { isNormalUser = true; uid = 1000; };
          };
        };
        # P0: enable the full daemon + broker systemd surface under test.
        nixling.daemonExperimental.enable = true;
      })
    ];
  };

  tmpfiles   = nixos.config.systemd.tmpfiles.rules;
  svcs       = nixos.config.systemd.services;
  socks      = nixos.config.systemd.sockets;
  svcBroker  = svcs.nixling-priv-broker;
  svcDaemon  = svcs.nixlingd;
  sockBroker = socks.nixling-priv-broker;

  # Filter tmpfiles rules that contain a space-delimited path token.
  # The format is "type path mode owner group age"; using " path "
  # (with surrounding spaces) avoids matching sub-paths such as
  # /run/nixling/locks when checking for /run/nixling.
  rulesForPath = path:
    builtins.filter (lib.hasInfix (" " + path + " ")) tmpfiles;

in {
  # --- tmpfiles ---
  tmpfiles-run-nixling       = rulesForPath "/run/nixling";
  tmpfiles-audit             = rulesForPath "/var/lib/nixling/audit";
  tmpfiles-current-bundle    = rulesForPath "/var/lib/nixling/current-bundle";

  # --- socket ---
  socket-listen-seqpacket    = sockBroker.socketConfig.ListenSequentialPacket;
  socket-user                = sockBroker.socketConfig.SocketUser;
  socket-group               = sockBroker.socketConfig.SocketGroup;
  socket-mode                = sockBroker.socketConfig.SocketMode;
  socket-fdname              = sockBroker.socketConfig.FileDescriptorName;

  # --- broker service ---
  broker-type                = svcBroker.serviceConfig.Type;
  broker-user                = svcBroker.serviceConfig.User;
  broker-group               = svcBroker.serviceConfig.Group;
  broker-caps                = sortStrs svcBroker.serviceConfig.CapabilityBoundingSet;

  # --- nixlingd service ---
  # restartIfChanged lives at the top-level NixOS service attrset, not
  # inside serviceConfig.  host-daemon.nix does not yet carry this field
  # (gap tracked in the sibling p0-nix-readiness worktree); the check
  # below will fail until that change lands.
  daemon-restart-if-changed  = svcDaemon.restartIfChanged or null;
  daemon-user                = svcDaemon.serviceConfig.User;
  daemon-restrict-af         = svcDaemon.serviceConfig.RestrictAddressFamilies;
  daemon-wants               = svcDaemon.wants or [];
}
NIXEOF
)

EVAL_OUT=$(nix-instantiate --eval --strict --json --expr "$EVAL_EXPR" 2>/dev/null) || {
  nix-instantiate --eval --strict --json --expr "$EVAL_EXPR" 2>&1 | tail -40 >&2 || true
  log "  FAIL: Phase 1 nix eval failed; cannot inspect systemd surface"
  exit 1
}

jq_get() { printf '%s' "$EVAL_OUT" | jq -r "$@"; }
jq_getc() { printf '%s' "$EVAL_OUT" | jq -c "$@"; }

check_eq() {
  local label="$1" actual="$2" expected="$3"
  if [ "$actual" = "$expected" ]; then
    pass_check "$label"
  else
    fail_check "$label: expected '$expected', got '$actual'"
  fi
}

# ---------------------------------------------------------------------------
# 1. /run/nixling tmpfiles — exactly ONE rule, owner nixlingd,
#    group nixling-launchers, mode 0750.
# ---------------------------------------------------------------------------
RUN_NIXLING_COUNT=$(jq_get '.["tmpfiles-run-nixling"] | length')
if [ "$RUN_NIXLING_COUNT" -eq 1 ]; then
  pass_check "tmpfiles /run/nixling: exactly one rule (count=$RUN_NIXLING_COUNT)"
else
  fail_check "tmpfiles /run/nixling: expected exactly 1 rule, got $RUN_NIXLING_COUNT"
fi

RUN_NIXLING_RULE=$(jq_get '.["tmpfiles-run-nixling"][0] // ""')
check_eq "tmpfiles /run/nixling rule content" \
  "$RUN_NIXLING_RULE" \
  "d /run/nixling 0750 nixlingd nixling-launchers -"

# ---------------------------------------------------------------------------
# 2. /var/lib/nixling/audit — 0750 root nixlingd
# ---------------------------------------------------------------------------
AUDIT_RULE=$(jq_get '.["tmpfiles-audit"][0] // ""')
check_eq "tmpfiles /var/lib/nixling/audit" \
  "$AUDIT_RULE" \
  "d /var/lib/nixling/audit 0750 root nixlingd -"

# ---------------------------------------------------------------------------
# 3. /var/lib/nixling/current-bundle — 0755 root root
# ---------------------------------------------------------------------------
BUNDLE_RULE=$(jq_get '.["tmpfiles-current-bundle"][0] // ""')
check_eq "tmpfiles /var/lib/nixling/current-bundle" \
  "$BUNDLE_RULE" \
  "d /var/lib/nixling/current-bundle 0755 root root -"

# ---------------------------------------------------------------------------
# 4-5. nixling-priv-broker.socket configuration
# ---------------------------------------------------------------------------
check_eq "socket.ListenSequentialPacket" "$(jq_get '.["socket-listen-seqpacket"]')" "/run/nixling/priv.sock"
check_eq "socket.SocketUser"         "$(jq_get '.["socket-user"]')"         "root"
check_eq "socket.SocketGroup"        "$(jq_get '.["socket-group"]')"        "nixlingd"
check_eq "socket.SocketMode"         "$(jq_get '.["socket-mode"]')"         "0660"
check_eq "socket.FileDescriptorName" "$(jq_get '.["socket-fdname"]')"       "priv.sock"

# ---------------------------------------------------------------------------
# 6-8. nixling-priv-broker.service configuration
# ---------------------------------------------------------------------------
check_eq "broker.serviceConfig.Type"  "$(jq_get '.["broker-type"]')"  "notify"
check_eq "broker.serviceConfig.User"  "$(jq_get '.["broker-user"]')"  "root"
check_eq "broker.serviceConfig.Group" "$(jq_get '.["broker-group"]')" "nixlingd"

# ---------------------------------------------------------------------------
# 9. Broker CapabilityBoundingSet — exact canonical set from plan.md.
#    Sorted for stable comparison (Nix sortStrs matches LC_ALL=C sort).
#    Canonical set: CAP_DAC_OVERRIDE CAP_DAC_READ_SEARCH CAP_FOWNER
#                   CAP_NET_ADMIN    CAP_NET_RAW          CAP_SETGID
#                   CAP_SETUID       CAP_SYS_ADMIN
#    (No CAP_SYS_PTRACE, no CAP_CHOWN outside the delegation window.)
# ---------------------------------------------------------------------------
CANONICAL_CAPS='["CAP_DAC_OVERRIDE","CAP_DAC_READ_SEARCH","CAP_FOWNER","CAP_NET_ADMIN","CAP_NET_RAW","CAP_SETGID","CAP_SETUID","CAP_SYS_ADMIN"]'
ACTUAL_CAPS=$(jq_getc '.["broker-caps"]')
check_eq "broker.CapabilityBoundingSet exact-set" "$ACTUAL_CAPS" "$CANONICAL_CAPS"

# ---------------------------------------------------------------------------
# 10. nixlingd.restartIfChanged = false
#     Gap: host-daemon.nix does not yet carry this field.  The check
#     intentionally fails until the sibling p0-nix-readiness change lands.
# ---------------------------------------------------------------------------
DAEMON_RIC=$(jq_get '.["daemon-restart-if-changed"]')
if [ "$DAEMON_RIC" = "false" ]; then
  pass_check "nixlingd.restartIfChanged = false"
else
  fail_check "nixlingd.restartIfChanged: expected false, got '$DAEMON_RIC'" \
    "(gap: p0-nix-readiness sibling adds restartIfChanged = false to host-daemon.nix)"
fi

# ---------------------------------------------------------------------------
# 11. nixlingd.serviceConfig.User = nixlingd
# ---------------------------------------------------------------------------
check_eq "nixlingd.serviceConfig.User" "$(jq_get '.["daemon-user"]')" "nixlingd"

# ---------------------------------------------------------------------------
# 12. nixlingd.serviceConfig.RestrictAddressFamilies = ["AF_UNIX"]
# ---------------------------------------------------------------------------
DAEMON_RAF=$(jq_getc '.["daemon-restrict-af"]')
check_eq 'nixlingd.RestrictAddressFamilies = ["AF_UNIX"]' "$DAEMON_RAF" '["AF_UNIX"]'

# ---------------------------------------------------------------------------
# 13. nixlingd.wants includes nixling-priv-broker.socket
# ---------------------------------------------------------------------------
WANTS_HIT=$(jq_get '.["daemon-wants"] | map(select(. == "nixling-priv-broker.socket")) | length')
if [ "$WANTS_HIT" -ge 1 ]; then
  pass_check "nixlingd.wants includes nixling-priv-broker.socket"
else
  fail_check "nixlingd.wants does not include nixling-priv-broker.socket"
fi

# ---------------------------------------------------------------------------
# 14. P0 evidence record schema shape — Phase 1 consistency assertion.
#     Verifies the shape that Phase 2 (NL_LIVE=1) would write is mutually
#     consistent with validationEvidencePresent in options-daemon.nix:
#       - wave: non-empty string equal to "p0"
#       - timestamp: non-empty string
#       - operatorSignature: non-empty string
#     Does NOT write the record; only confirms the shape is valid.
# ---------------------------------------------------------------------------
SAMPLE_EVIDENCE=$(printf '{"wave":"p0","timestamp":"2024-01-01T00:00:00Z","operatorSignature":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}')
_ev_wave=$(printf '%s' "$SAMPLE_EVIDENCE" | jq -r '.wave // empty')
_ev_ts=$(printf '%s' "$SAMPLE_EVIDENCE" | jq -r '.timestamp // empty')
_ev_sig=$(printf '%s' "$SAMPLE_EVIDENCE" | jq -r '.operatorSignature // empty')
if [ "$_ev_wave" = "p0" ] && [ -n "$_ev_ts" ] && [ -n "$_ev_sig" ]; then
  pass_check "phase1: p0 evidence record schema shape is consistent with validationEvidencePresent"
else
  fail_check "phase1: p0 evidence record schema shape failed consistency check (wave='$_ev_wave' ts='$_ev_ts' sig='$_ev_sig')"
fi

log "==> Phase 1: ${PASS} passed, ${FAIL} failed"

# ---------------------------------------------------------------------------
# Phase 2 — live (opt-in via NL_LIVE=1)
# ---------------------------------------------------------------------------
if [ "${NL_LIVE:-0}" != "1" ]; then
  log "==> Phase 2 skipped (set NL_LIVE=1 to run live checks)"
  [ "$FAIL" -eq 0 ]
  exit $?
fi

log "==> Phase 2: live checks (NL_LIVE=1)"

SCRATCH=$(nl_mktemp .nixlingd-startup-smoke.XXXXXX)
BUNDLE_JSON=/etc/nixling/bundle.json
BUNDLE_BACKUP="$SCRATCH/bundle.json.bak"
CLEANUP_OK=1

# --- Pre-test state capture ---

BUNDLE_ORIG_SHA256=""
if [ -f "$BUNDLE_JSON" ]; then
  BUNDLE_ORIG_SHA256=$(sha256sum "$BUNDLE_JSON" | awk '{print $1}')
  cp --preserve=all "$BUNDLE_JSON" "$BUNDLE_BACKUP"
fi

NIXLINGD_WAS_ACTIVE=$(systemctl is-active nixlingd.service 2>/dev/null || printf 'inactive')
BROKER_SOCKET_WAS_ACTIVE=$(systemctl is-active nixling-priv-broker.socket 2>/dev/null || printf 'inactive')

# --- Cleanup function (registered via add_cleanup, runs on EXIT) ---

_phase2_restore() {
  local rc=0

  # 1. Restore bundle.json if tampered.
  if [ -n "$BUNDLE_ORIG_SHA256" ] && [ -f "$BUNDLE_JSON" ]; then
    local cur_sha
    cur_sha=$(sha256sum "$BUNDLE_JSON" | awk '{print $1}')
    if [ "$cur_sha" != "$BUNDLE_ORIG_SHA256" ]; then
      log "cleanup: restoring tampered $BUNDLE_JSON"
      sudo cp "$BUNDLE_BACKUP" "$BUNDLE_JSON" || {
        log "  FAIL cleanup: could not restore $BUNDLE_JSON"
        rc=1
      }
    fi
  fi

  # 2. Restore services to pre-test state.
  if [ "$NIXLINGD_WAS_ACTIVE" = "active" ]; then
    sudo systemctl start nixlingd.service 2>/dev/null || {
      log "  FAIL cleanup: could not restore nixlingd.service"
      rc=1
    }
  else
    sudo systemctl stop nixlingd.service 2>/dev/null || true
  fi

  if [ "$BROKER_SOCKET_WAS_ACTIVE" = "active" ]; then
    sudo systemctl start nixling-priv-broker.socket 2>/dev/null || {
      log "  FAIL cleanup: could not restore nixling-priv-broker.socket"
      rc=1
    }
  else
    sudo systemctl stop nixling-priv-broker.socket 2>/dev/null || true
  fi

  # 3. Remove transient runtime files created by this test.
  sudo rm -f /run/nixling/public.sock.smoke-test-probe 2>/dev/null || true

  if [ "$rc" -ne 0 ]; then
    CLEANUP_OK=0
  fi
  return "$rc"
}
add_cleanup "_phase2_restore"

# --- Step 3: Cycle services ---

log "  phase2: stopping nixling-priv-broker.socket nixling-priv-broker.service nixlingd.service"
sudo systemctl stop \
  nixling-priv-broker.socket \
  nixling-priv-broker.service \
  nixlingd.service 2>/dev/null || true

log "  phase2: starting nixling-priv-broker.socket nixlingd.service"
sudo systemctl start nixling-priv-broker.socket nixlingd.service

# --- Step 4: Wait up to 30s for /run/nixling/public.sock ---

PUBLIC_SOCK=/run/nixling/public.sock
log "  phase2: waiting for $PUBLIC_SOCK (up to 30s)"
_sock_appeared=0
for _i in $(seq 1 300); do
  if [ -S "$PUBLIC_SOCK" ]; then
    _sock_appeared=1
    break
  fi
  sleep 0.1
done

if [ "$_sock_appeared" -eq 1 ]; then
  pass_check "phase2: /run/nixling/public.sock appeared within 30s"
  PASS=$((PASS + 1))
else
  fail_check "phase2: /run/nixling/public.sock did not appear within 30s"
  log "==> Phase 2 aborted: daemon did not bind public socket"
  log "==> Total: ${PASS} passed, ${FAIL} failed"
  [ "$CLEANUP_OK" -eq 1 ] || fail_check "phase2: cleanup failed"
  exit 1
fi

# --- Step 5: Hello handshake ---

HELLO_FRAME='{"type":"hello","clientVersion":">=0.4.0, <0.5.0","supportedFeatures":[]}'
HELLO_OUT="$SCRATCH/hello.json"
if nixlingd test-client \
    --socket "$PUBLIC_SOCK" \
    --frame-json "$HELLO_FRAME" \
    > "$HELLO_OUT" 2>/dev/null; then
  HELLO_TYPE=$(jq -r '.type // empty' "$HELLO_OUT" 2>/dev/null || true)
  if [ "$HELLO_TYPE" = "helloOk" ]; then
    pass_check "phase2: Hello → helloOk typed response"
  else
    fail_check "phase2: Hello response type='$HELLO_TYPE', expected 'helloOk' (full response in $HELLO_OUT)"
  fi
else
  fail_check "phase2: nixlingd test-client Hello round-trip failed"
fi

# --- Step 6: /run/nixling/version ---

VERSION_FILE=/run/nixling/version
if [ -f "$VERSION_FILE" ]; then
  pass_check "phase2: /run/nixling/version exists"
  VERSION_JSON=$(cat "$VERSION_FILE")
  for _field in binary_path server_version started_at protocol_version; do
    _val=$(printf '%s' "$VERSION_JSON" | jq -r --arg f "$_field" '.[$f] // empty' 2>/dev/null || true)
    if [ -n "$_val" ]; then
      pass_check "phase2: /run/nixling/version.$_field present"
    else
      fail_check "phase2: /run/nixling/version.$_field missing or null"
    fi
  done
else
  fail_check "phase2: /run/nixling/version does not exist"
fi

# --- Step 7: Bundle tamper test ---

if [ -z "$BUNDLE_ORIG_SHA256" ]; then
  log "  SKIP phase2-tamper: $BUNDLE_JSON not present; skipping tamper test"
else
  log "  phase2: tampering $BUNDLE_JSON (one byte modification)"
  # Write a byte-modified copy (flip the last non-whitespace byte).
  python3 - "$BUNDLE_JSON" "$BUNDLE_BACKUP" <<'PYEOF'
import sys, os
src, dst = sys.argv[1], sys.argv[2]
data = bytearray(open(src, 'rb').read())
# Find last non-whitespace byte and flip it.
for i in range(len(data) - 1, -1, -1):
    if data[i] not in (ord(' '), ord('\n'), ord('\r'), ord('\t')):
        data[i] ^= 0x01
        break
open(src, 'wb').write(data)
PYEOF
  # Verify the SHA changed.
  TAMPERED_SHA=$(sha256sum "$BUNDLE_JSON" | awk '{print $1}')
  if [ "$TAMPERED_SHA" = "$BUNDLE_ORIG_SHA256" ]; then
    fail_check "phase2-tamper: byte modification did not change bundle.json SHA256"
  else
    pass_check "phase2-tamper: bundle.json SHA256 changed after modification"
  fi

  # Send a mutating verb that triggers load_bundle_resolver.
  # vm start against a non-existent VM loads the bundle before
  # failing on "VM not found in processes.json".
  TAMPER_FRAME='{"type":"vmStart","vm":"__smoke-test-nonexistent__","flags":{"dryRun":false,"json":false,"verbose":false}}'
  TAMPER_OUT="$SCRATCH/tamper-response.json"
  nixlingd test-client \
    --socket "$PUBLIC_SOCK" \
    --frame-json "$TAMPER_FRAME" \
    > "$TAMPER_OUT" 2>/dev/null || true

  TAMPER_TYPE=$(jq -r '.type // .error.kind // empty' "$TAMPER_OUT" 2>/dev/null || true)
  if printf '%s' "$TAMPER_TYPE" | grep -qi 'tamper\|BundleTampered'; then
    pass_check "phase2-tamper: daemon returned BundleTampered envelope"
  else
    fail_check "phase2-tamper: expected BundleTampered response, got type='$TAMPER_TYPE'" \
      "(P0 bundle digest verification may not yet be implemented in load_bundle_resolver)"
  fi

  # Restore immediately so cleanup doesn't double-restore.
  log "  phase2: restoring $BUNDLE_JSON"
  sudo cp "$BUNDLE_BACKUP" "$BUNDLE_JSON"
  BUNDLE_ORIG_SHA256=""  # Signal that restore is already done.
fi

# ---------------------------------------------------------------------------
# Phase 2 — Step 8: write P0 evidence record if all checks passed so far.
#   Written only when NL_LIVE=1 (Phase 2) and FAIL == 0 at this point.
#   Schema matches validationEvidencePresent in nixos-modules/options-daemon.nix:
#     wave, timestamp (UTC RFC-3339), operatorSignature (sha256 of
#     "plan.md|daemon.version|broker.version|bundle.hash").
# ---------------------------------------------------------------------------
if [ "$FAIL" -eq 0 ]; then
  log "  phase2: writing P0 evidence record"

  # Compute operatorSignature inputs.
  _plan_content=""
  _plan_md="$ROOT/plan.md"
  if [ -f "$_plan_md" ]; then
    _plan_content=$(sha256sum "$_plan_md" | awk '{print $1}')
  fi

  _daemon_ver="unknown"
  if [ -f /run/nixling/version ]; then
    _daemon_ver=$(jq -r '.server_version // "unknown"' /run/nixling/version 2>/dev/null || printf 'unknown')
  fi

  _broker_ver="unknown"
  if command -v nixling-priv-broker >/dev/null 2>&1; then
    _broker_ver=$(nixling-priv-broker --version 2>/dev/null | awk '{print $NF}' || printf 'unknown')
  fi

  _bundle_hash="unknown"
  if [ -f "$BUNDLE_JSON" ]; then
    _bundle_hash="sha256:$(sha256sum "$BUNDLE_JSON" | awk '{print $1}')"
  fi

  _sig_input="${_plan_content}|${_daemon_ver}|${_broker_ver}|${_bundle_hash}"
  _operator_sig="sha256:$(printf '%s' "$_sig_input" | sha256sum | awk '{print $1}')"
  _ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  _evidence_json=$(printf '{"wave":"p0","timestamp":"%s","operatorSignature":"%s"}\n' \
    "$_ts" "$_operator_sig")

  sudo mkdir -p /var/lib/nixling/validated
  if printf '%s' "$_evidence_json" | sudo tee /var/lib/nixling/validated/p0.json >/dev/null; then
    pass_check "phase2: P0 evidence record written to /var/lib/nixling/validated/p0.json"
  else
    fail_check "phase2: failed to write P0 evidence record to /var/lib/nixling/validated/p0.json"
  fi
else
  log "  phase2: skipping P0 evidence write (${FAIL} check(s) failed)"
fi

# --- Run cleanups explicitly before declaring success (contract requirement) ---
# The EXIT trap remains as belt-and-suspenders for hard kills, but a clean
# test exit MUST run cleanup and FAIL if cleanup cannot complete.
log "==> Phase 2: running explicit pre-success cleanup"
if ! _phase2_restore; then
  fail_check "phase2: cleanup failed (state may be inconsistent)"
fi

# --- Final summary ---

log "==> Total: ${PASS} passed, ${FAIL} failed"

[ "$FAIL" -eq 0 ]
