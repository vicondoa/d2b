#!/usr/bin/env bash
# tests/broker-bundle-path-eval.sh — P0 bundle-path alignment eval gate.
#
# Asserts that the three independent bundle-path declarations in the NixOS
# module tree all agree on the same canonical path:
#
#   (A) nixos-modules/host-broker.nix bundleManifestPath default
#       (the literal fallback used when cfg.site.bundle.currentManifest
#       is absent) resolves to /etc/nixling/bundle.json.
#
#   (B) nixos-modules/bundle.nix emits the bundle at
#       environment.etc."nixling/bundle.json" — i.e. the file lands at
#       /etc/nixling/bundle.json — matching the broker's --bundle-path.
#
#   (C) nixos-modules/host-daemon.nix daemonConfigJson artifacts.bundlePath
#       equals /etc/nixling/bundle.json, so the daemon and broker share the
#       same bundle location at runtime.
#
# (A) and (C) are source-text checks; nix eval of serviceConfig.ExecStart
# forces the broker/daemon derivation builds and causes infinite recursion,
# so we inspect the .nix source directly (same pattern as
# broker-socket-activation-eval.sh check A).
#
# (B) is verified via nix eval: we materialise a minimal NixOS config and
# assert that environment.etc."nixling/bundle.json" is present, proving
# bundle.nix does emit at that path.
#
# Wired into tests/static.sh alongside the other broker-* eval gates.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/broker-bundle-path-eval.sh"

HOST_BROKER_NIX="$ROOT/nixos-modules/host-broker.nix"
# Intentionally unset BUNDLE_NIX (was declared for cross-eval but the
# bundle.nix probe path moved to host-daemon.nix; keep the comment so
# the next reviewer doesn't re-add it).
HOST_DAEMON_NIX="$ROOT/nixos-modules/host-daemon.nix"

CANONICAL_PATH="/etc/nixling/bundle.json"

# ---------------------------------------------------------------------------
# (A) broker bundleManifestPath default must be /etc/nixling/bundle.json.
#
# The assignment in host-broker.nix reads:
#   bundleManifestPath =
#     cfg.site.bundle.currentManifest or "/etc/nixling/bundle.json";
#
# We extract the literal string that appears as the `or` fallback.
# ---------------------------------------------------------------------------
BROKER_DEFAULT=$(grep -o '"[^"]*bundle[^"]*"' "$HOST_BROKER_NIX" | grep -v 'current-bundle\|manifest\.json' | head -1 | tr -d '"')

if [ -z "$BROKER_DEFAULT" ]; then
  # Fall back to extracting any quoted path on the bundleManifestPath line
  BROKER_DEFAULT=$(grep -A2 'bundleManifestPath' "$HOST_BROKER_NIX" | grep -o '"/[^"]*"' | tail -1 | tr -d '"')
fi

if [ "$BROKER_DEFAULT" != "$CANONICAL_PATH" ]; then
  fail "host-broker.nix bundleManifestPath fallback is '${BROKER_DEFAULT}', \
expected '${CANONICAL_PATH}'. Broker and daemon bundle paths would diverge."
fi
ok "host-broker.nix bundleManifestPath fallback = ${CANONICAL_PATH}"

# Also verify --bundle-path appears near ExecStart (sanity that it's wired).
BUNDLE_PATH_LINE=$(grep -- '--bundle-path' "$HOST_BROKER_NIX" | grep -v '^\s*#' || true)
if [ -z "$BUNDLE_PATH_LINE" ]; then
  fail "host-broker.nix ExecStart does not contain --bundle-path; \
the broker must receive the bundle path at start time."
fi
ok "host-broker.nix ExecStart contains --bundle-path"

# ---------------------------------------------------------------------------
# (B) bundle.nix must emit environment.etc."nixling/bundle.json".
#
# We do a nix eval against the minimal consumer-config to confirm the key is
# present in environment.etc (proving the file will land at the right path).
# ---------------------------------------------------------------------------
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
          waylandUser   = "alice";
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
  etcAttrs = nixos.config.environment.etc;
in {
  bundleJsonPresent = etcAttrs ? "nixling/bundle.json";
}
EOF
)

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "nix-instantiate failed; cannot verify bundle.nix emits environment.etc.\"nixling/bundle.json\""

BUNDLE_JSON_PRESENT=$(printf '%s' "$OUT" | jq -r '.bundleJsonPresent')
if [ "$BUNDLE_JSON_PRESENT" != "true" ]; then
  fail "environment.etc.\"nixling/bundle.json\" is absent from the evaluated NixOS config. \
bundle.nix must emit the bundle at /etc/nixling/bundle.json to match the broker's --bundle-path."
fi
ok "environment.etc.\"nixling/bundle.json\" present (bundle lands at ${CANONICAL_PATH})"

# ---------------------------------------------------------------------------
# (C) host-daemon.nix daemonConfigJson artifacts.bundlePath must be
#     /etc/nixling/bundle.json.
#
# We inspect the source text for the bundlePath assignment inside the
# daemonConfigJson block. The nix expression writes a literal string.
# ---------------------------------------------------------------------------
DAEMON_BUNDLE_PATH=$(grep 'bundlePath' "$HOST_DAEMON_NIX" | grep -o '"/[^"]*"' | tr -d '"' | head -1)

if [ -z "$DAEMON_BUNDLE_PATH" ]; then
  fail "host-daemon.nix does not define bundlePath in daemonConfigJson; \
cannot verify daemon/broker bundle-path agreement."
fi

if [ "$DAEMON_BUNDLE_PATH" != "$CANONICAL_PATH" ]; then
  fail "host-daemon.nix daemonConfigJson artifacts.bundlePath is '${DAEMON_BUNDLE_PATH}', \
expected '${CANONICAL_PATH}'. Daemon and broker bundle paths diverge — \
the daemon and broker must agree on the bundle location."
fi
ok "host-daemon.nix daemonConfigJson artifacts.bundlePath = ${CANONICAL_PATH}"

# ---------------------------------------------------------------------------
# Summary: all three declarations agree.
# ---------------------------------------------------------------------------
log "==> broker-bundle-path-eval.sh: all checks passed"
log "    broker default  = ${BROKER_DEFAULT}"
log "    bundle.nix etc  = ${CANONICAL_PATH} (eval-confirmed)"
log "    daemon config   = ${DAEMON_BUNDLE_PATH}"
