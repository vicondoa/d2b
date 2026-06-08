#!/usr/bin/env bash
# tests/broker-caps-eval.sh — Layer-1 eval gate for P0 broker capability
# hardening.
#
# Asserts that systemd.services.nixling-priv-broker.serviceConfig
# .CapabilityBoundingSet matches the canonical P0 set EXACTLY — no
# additions, no omissions — and that AmbientCapabilities contains the
# sentinel empty-string entry required to drop all ambient caps.
#
# Canonical set (per plan.md §"Canonical broker CapabilityBoundingSet"):
#
#   CAP_NET_ADMIN          tap create/destroy, bridge ops, route table
#   CAP_NET_RAW            nftables socket creation, USBIP firewall carve-outs
#   CAP_DAC_OVERRIDE       writing /etc/hosts + /etc/NetworkManager drop-ins
#   CAP_DAC_READ_SEARCH    reading per-VM state dirs across uids
#   CAP_SYS_ADMIN          cgroup v2 delegation + minijail namespace setup
#   CAP_SETUID             fchown of delegated cgroup subtree to nixlingd uid
#   CAP_SETGID             fchown of delegated cgroup subtree to nixlingd gid
#   CAP_FOWNER             mode ops on broker-owned state
#
# Notable absences that are a hard FAIL if present:
#   CAP_SYS_PTRACE, CAP_CHOWN, CAP_NET_BIND_SERVICE, CAP_AUDIT_WRITE
#
# Wired into tests/static.sh (mid-tier eval pool).
# Authority: plan.md P0 security-r2-1 + ph0-broker-caps-audit.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/broker-caps-eval.sh"

# ---------------------------------------------------------------------------
# Canonical P0 CapabilityBoundingSet (sorted; order-independent comparison).
# ---------------------------------------------------------------------------
CANONICAL_CAPS=(
  CAP_DAC_OVERRIDE
  CAP_DAC_READ_SEARCH
  CAP_FOWNER
  CAP_NET_ADMIN
  CAP_NET_RAW
  CAP_SETGID
  CAP_SETUID
  CAP_SYS_ADMIN
)

# Minimal config: daemonExperimental.enable=true is sufficient to
# materialise nixling-priv-broker.{socket,service}. No per-VM workload
# needed; this is purely a host-broker module test.
EXPR=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
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
        nixling.daemonExperimental.enable = true;
      })
    ];
  };
  svc = nixos.config.systemd.services.nixling-priv-broker.serviceConfig;
in {
  cbs = svc.CapabilityBoundingSet or null;
  ac  = svc.AmbientCapabilities or null;
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "nix eval failed — cannot inspect broker serviceConfig (is daemonExperimental wiring present?)"

# ---------------------------------------------------------------------------
# 1. CapabilityBoundingSet must not be null.
# ---------------------------------------------------------------------------
CBS=$(printf '%s' "$OUT" | jq -r '.cbs')
if [ "$CBS" = "null" ]; then
  fail "CapabilityBoundingSet is null — nixling-priv-broker.serviceConfig missing"
fi

# ---------------------------------------------------------------------------
# 2. Exact set comparison: sort both sides and diff.
# ---------------------------------------------------------------------------
GOT_SORTED=$(printf '%s' "$OUT" | jq -r '.cbs | sort | .[]')
WANT_SORTED=$(printf '%s\n' "${CANONICAL_CAPS[@]}" | sort)

MISSING=$(comm -23 <(printf '%s\n' "$WANT_SORTED") <(printf '%s\n' "$GOT_SORTED"))
EXTRA=$(comm -13   <(printf '%s\n' "$WANT_SORTED") <(printf '%s\n' "$GOT_SORTED"))

if [ -n "$MISSING" ] || [ -n "$EXTRA" ]; then
  log "  FAIL: CapabilityBoundingSet does not match canonical P0 set"
  if [ -n "$MISSING" ]; then
    log "  MISSING caps (in canonical set, absent from broker):"
    while IFS= read -r cap; do
      log "    - $cap"
    done <<< "$MISSING"
  fi
  if [ -n "$EXTRA" ]; then
    log "  EXTRA caps (in broker, NOT in canonical set — security violation):"
    while IFS= read -r cap; do
      log "    + $cap"
    done <<< "$EXTRA"
  fi
  log "  Canonical set : $(printf '%s\n' "$WANT_SORTED" | tr '\n' ' ')"
  log "  Broker has    : $(printf '%s\n' "$GOT_SORTED" | tr '\n' ' ')"
  exit 1
fi
ok "CapabilityBoundingSet matches canonical P0 set exactly (8 caps)"

# ---------------------------------------------------------------------------
# 3. AmbientCapabilities must contain the sentinel "" entry to ensure all
#    ambient capabilities are dropped (systemd drops ambient caps for each
#    entry; the empty string is the canonical NixOS way to emit the directive
#    with no positive grants while still writing the key to the unit file).
# ---------------------------------------------------------------------------
AC=$(printf '%s' "$OUT" | jq -r '.ac')
if [ "$AC" = "null" ]; then
  fail "AmbientCapabilities is absent — broker must set AmbientCapabilities = [\"\"] to drop all ambient caps"
fi

HAS_EMPTY=$(printf '%s' "$OUT" | jq 'if (.ac | type) == "array" then (.ac | map(select(. == "")) | length > 0) elif (.ac | type) == "string" then (.ac == "") else false end')
if [ "$HAS_EMPTY" != "true" ]; then
  fail "AmbientCapabilities does not contain the empty-string sentinel entry (got: $(printf '%s' "$OUT" | jq -c '.ac'))"
fi
ok "AmbientCapabilities contains empty-string sentinel (all ambient caps dropped)"

log "==> broker-caps-eval OK"
