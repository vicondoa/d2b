#!/usr/bin/env bash
# tests/store-overlay-emit-eval.sh— v1.2 DiskInit plan-op emit gate.
#
# Asserts that `processes-json.nix` emits a `DiskInit` plan-op in the
# cloud-hypervisor node's `planOps` field when the guest VM config sets
# `microvm.writableStoreOverlay` to a non-null path.
#
# Specifically checks:
#   1. The CH node has `planOps` containing exactly one entry.
#   2. The entry has `kind = "diskInit"`.
#   3. `targetPath` ends with `/<vm>/store-overlay.img`.
#   4. `sizeBytes` equals the default (1 GiB = 1073741824).
#   5. `mode` = 384 (0o600 decimal).
#   6. `ifAbsent` = true.
#   7. `ownerUid` and `ownerGid` are positive integers (runner profile uid/gid).
#
# Also checks the absence case: when `microvm.writableStoreOverlay` is not
# set (null default), the `planOps` field is absent/empty.
#
# Exit 0 on PASS, exit 1 on FAIL.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; FAILED=$((FAILED + 1)); }

FAILED=0

log "==> tests/store-overlay-emit-eval.sh"

# ---------------------------------------------------------------------------
# Case 1: microvm.writableStoreOverlay = "/nix/.rw-store" — DiskInit emitted.
# ---------------------------------------------------------------------------
EXPR_ENABLED=$(cat <<'NIXEOF'
let
  flake = builtins.getFlake "git+file://ROOTPLACEHOLDER";
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.overlay-probe = {
          enable   = true;
          env      = "work";
          index    = 11;
          ssh.user = "alice";
          config = {
            # Set the guest-side writable overlay option.
            microvm.writableStoreOverlay = "/nix/.rw-store";
            networking.hostName = lib.mkDefault "overlay-probe";
            users.users.alice = { isNormalUser = true; uid = 1000; };
            system.stateVersion = "25.11";
          };
        };
      })
    ];
  };
  vms     = nixos.config.nixling._bundle.processesJson.data.vms;
  vm      = builtins.head (builtins.filter (v: v.vm == "overlay-probe") vms);
  nodes   = vm.nodes;
  chNode  = builtins.head (builtins.filter (n: n.id == "cloud-hypervisor") nodes);
  planOps = chNode.planOps or [];
in {
  planOpsCount  = builtins.length planOps;
  firstOp       = if planOps == [] then null else builtins.head planOps;
}
NIXEOF
)

# Substitute the ROOT path.
EXPR_ENABLED="${EXPR_ENABLED//ROOTPLACEHOLDER/$ROOT}"

log "[case-1] Evaluating with microvm.writableStoreOverlay = /nix/.rw-store ..."
OUT1=$(nix-instantiate --eval --strict --json --expr "$EXPR_ENABLED" 2>/dev/null) || {
  log "eval failed (case 1) — re-running with stderr for diagnosis:"
  nix-instantiate --eval --strict --json --expr "$EXPR_ENABLED" 2>&1 | tail -20 >&2
  exit 1
}

PLAN_OPS_COUNT=$(printf '%s' "$OUT1" | jq -r '.planOpsCount')
FIRST_OP_KIND=$(printf '%s' "$OUT1" | jq -r '.firstOp.kind // "null"')
FIRST_OP_TARGET=$(printf '%s' "$OUT1" | jq -r '.firstOp.targetPath // "null"')
FIRST_OP_SIZE=$(printf '%s' "$OUT1" | jq -r '.firstOp.sizeBytes // "null"')
FIRST_OP_MODE=$(printf '%s' "$OUT1" | jq -r '.firstOp.mode // "null"')
FIRST_OP_IF_ABSENT=$(printf '%s' "$OUT1" | jq -r '.firstOp.ifAbsent // "null"')
FIRST_OP_UID=$(printf '%s' "$OUT1" | jq -r '.firstOp.ownerUid // "null"')
FIRST_OP_GID=$(printf '%s' "$OUT1" | jq -r '.firstOp.ownerGid // "null"')

log "  planOps count      = ${PLAN_OPS_COUNT}"
log "  firstOp.kind       = ${FIRST_OP_KIND}"
log "  firstOp.targetPath = ${FIRST_OP_TARGET}"
log "  firstOp.sizeBytes  = ${FIRST_OP_SIZE}"
log "  firstOp.mode       = ${FIRST_OP_MODE}"
log "  firstOp.ifAbsent   = ${FIRST_OP_IF_ABSENT}"
log "  firstOp.ownerUid   = ${FIRST_OP_UID}"
log "  firstOp.ownerGid   = ${FIRST_OP_GID}"

if [ "$PLAN_OPS_COUNT" = "1" ]; then
  ok "planOps count = 1"
else
  fail "planOps count = ${PLAN_OPS_COUNT} (expected 1)"
fi

if [ "$FIRST_OP_KIND" = "diskInit" ]; then
  ok "firstOp.kind = diskInit"
else
  fail "firstOp.kind = ${FIRST_OP_KIND} (expected diskInit)"
fi

# targetPath must end with /overlay-probe/store-overlay.img
if printf '%s' "$FIRST_OP_TARGET" | grep -q "/overlay-probe/store-overlay\\.img$"; then
  ok "firstOp.targetPath ends with /overlay-probe/store-overlay.img"
else
  fail "firstOp.targetPath = ${FIRST_OP_TARGET} (expected suffix /overlay-probe/store-overlay.img)"
fi

if [ "$FIRST_OP_SIZE" = "1073741824" ]; then
  ok "firstOp.sizeBytes = 1073741824 (1 GiB default)"
else
  fail "firstOp.sizeBytes = ${FIRST_OP_SIZE} (expected 1073741824)"
fi

if [ "$FIRST_OP_MODE" = "384" ]; then
  ok "firstOp.mode = 384 (0o600)"
else
  fail "firstOp.mode = ${FIRST_OP_MODE} (expected 384 = 0o600)"
fi

if [ "$FIRST_OP_IF_ABSENT" = "true" ]; then
  ok "firstOp.ifAbsent = true"
else
  fail "firstOp.ifAbsent = ${FIRST_OP_IF_ABSENT} (expected true)"
fi

# ownerUid and ownerGid must be positive integers (> 0)
if [ "$FIRST_OP_UID" != "null" ] && [ "$FIRST_OP_UID" -gt 0 ] 2>/dev/null; then
  ok "firstOp.ownerUid = ${FIRST_OP_UID} (positive)"
else
  fail "firstOp.ownerUid = ${FIRST_OP_UID} (expected positive integer)"
fi
if [ "$FIRST_OP_GID" != "null" ] && [ "$FIRST_OP_GID" -gt 0 ] 2>/dev/null; then
  ok "firstOp.ownerGid = ${FIRST_OP_GID} (positive)"
else
  fail "firstOp.ownerGid = ${FIRST_OP_GID} (expected positive integer)"
fi

# ---------------------------------------------------------------------------
# Case 2: microvm.writableStoreOverlay not set — planOps absent/empty.
# ---------------------------------------------------------------------------
EXPR_DISABLED=$(cat <<'NIXEOF'
let
  flake = builtins.getFlake "git+file://ROOTPLACEHOLDER";
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
        };
        nixling.envs.work = {
          lanSubnet    = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.no-overlay-probe = {
          enable   = true;
          env      = "work";
          index    = 12;
          ssh.user = "alice";
          # microvm.writableStoreOverlay not set (null default)
          config = {
            networking.hostName = lib.mkDefault "no-overlay-probe";
            users.users.alice = { isNormalUser = true; uid = 1000; };
            system.stateVersion = "25.11";
          };
        };
      })
    ];
  };
  vms    = nixos.config.nixling._bundle.processesJson.data.vms;
  vm     = builtins.head (builtins.filter (v: v.vm == "no-overlay-probe") vms);
  nodes  = vm.nodes;
  chNode = builtins.head (builtins.filter (n: n.id == "cloud-hypervisor") nodes);
in {
  hasPlanOps = chNode ? planOps;
  planOps    = chNode.planOps or [];
}
NIXEOF
)

EXPR_DISABLED="${EXPR_DISABLED//ROOTPLACEHOLDER/$ROOT}"

log "[case-2] Evaluating with microvm.writableStoreOverlay = null (default) ..."
OUT2=$(nix-instantiate --eval --strict --json --expr "$EXPR_DISABLED" 2>/dev/null) || {
  log "eval failed (case 2)"
  nix-instantiate --eval --strict --json --expr "$EXPR_DISABLED" 2>&1 | tail -20 >&2
  exit 1
}

HAS_PLAN_OPS=$(printf '%s' "$OUT2" | jq -r '.hasPlanOps')
PLAN_OPS_LEN=$(printf '%s' "$OUT2" | jq -r '.planOps | length')
PLAN_OPS_EMPTY=$(printf '%s' "$OUT2" | jq -r '.planOps | length == 0')

log "  hasPlanOps     = ${HAS_PLAN_OPS}"
log "  planOps length = ${PLAN_OPS_LEN}"

if [ "$HAS_PLAN_OPS" = "false" ] || [ "$PLAN_OPS_EMPTY" = "true" ]; then
  ok "planOps absent/empty when microvm.writableStoreOverlay = null"
else
  fail "planOps unexpectedly present when microvm.writableStoreOverlay = null"
fi

if [ "$FAILED" -gt 0 ]; then
  log "==> FAIL: ${FAILED} assertion(s) failed (v1.2 D9/P5.1+P5.2)"
  exit 1
fi

log "==> PASS: DiskInit plan-op emitted correctly (v1.2 D9/P5.1+P5.2)"
