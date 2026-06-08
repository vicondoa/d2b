#!/usr/bin/env bash
# Socket-activation eval gate.
#
# Asserts that host-broker.nix wires up nixling-priv-broker correctly for
# socket activation.  Specifically it must hold that:
#
#   (A) ExecStart does NOT contain --socket-path (the broker must adopt the
#       inherited fd via SD_LISTEN_FDS, not bind the path itself).
#
#   (B) systemd.sockets.nixling-priv-broker exists (socket-activated, not
#       plain service).
#
#   (C) The socket unit's FileDescriptorName is "priv.sock" (matches the
#       name that adopt_listen_fd() validates against LISTEN_FDNAMES).
#
#   (D) The socket unit listens at /run/nixling/priv.sock.
#
# Wired into tests/static.sh via the eval-cases/ pattern.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/broker-socket-activation-eval.sh"

HOST_BROKER_NIX="$ROOT/nixos-modules/host-broker.nix"

# ---------------------------------------------------------------------------
# (A) ExecStart source must NOT contain --socket-path.
#
# Note: evaluating serviceConfig.ExecStart via nix-instantiate causes an
# infinite recursion because it forces the broker derivation build.  Instead
# we inspect the source text of host-broker.nix directly.  The ExecStart
# assignment is a simple string concatenation — no computed socket-path
# flag appears in the source.
# ---------------------------------------------------------------------------
EXEC_START_LINES=$(grep -n 'ExecStart' "$HOST_BROKER_NIX" | grep -v '^\s*#')
if printf '%s' "$EXEC_START_LINES" | grep -q -- '--socket-path'; then
  fail "host-broker.nix ExecStart contains --socket-path; with SD_LISTEN_FDS \
the broker must adopt the inherited fd, not bind the path itself. \
Remove --socket-path from host-broker.nix ExecStart."
fi
ok "host-broker.nix ExecStart does not contain --socket-path"

# Also assert the ExecStart assignment is actually present (sanity check
# that the file wasn't hollowed out).
if ! printf '%s' "$EXEC_START_LINES" | grep -q 'ExecStart'; then
  fail "host-broker.nix does not define ExecStart at all; file may be broken"
fi
ok "host-broker.nix ExecStart assignment is present"

# ---------------------------------------------------------------------------
# (B–D) Evaluate the socket unit config via nix-instantiate.
#       Accessing serviceConfig.ExecStart causes infinite recursion
#       (forces the broker derivation); socketConfig fields are safe.
# ---------------------------------------------------------------------------
EXPR=$(cat <<'EOF'
let
  flake = builtins.getFlake (toString ROOT);
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
        nixling.daemonExperimental.enable = true;
      })
    ];
  };
  hasSock  = nixos.config.systemd.sockets ? nixling-priv-broker;
  sockCfg  = nixos.config.systemd.sockets.nixling-priv-broker.socketConfig or {};
  fdName   = sockCfg.FileDescriptorName or "";
  listenAt = sockCfg.ListenSequentialPacket or "";
in {
  hasSocket            = hasSock;
  fdName               = fdName;
  listenSequentialPacket = listenAt;
}
EOF
)

# Substitute $ROOT into the expression (nix's `toString ROOT` needs a shell var).
EXPR=${EXPR//ROOT/$ROOT}

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) || \
  fail "nix-instantiate failed; cannot inspect broker socket-activation config"

HAS_SOCKET=$(printf '%s' "$OUT"             | jq -r '.hasSocket')
FD_NAME=$(printf '%s' "$OUT"               | jq -r '.fdName')
LISTEN_SEQ=$(printf '%s' "$OUT"            | jq -r '.listenSequentialPacket')

# (B) Socket unit must exist.
if [ "$HAS_SOCKET" != "true" ]; then
  fail "systemd.sockets.nixling-priv-broker is absent; the broker MUST be \
socket-activated (P0 socket-activation contract)."
fi
ok "systemd.sockets.nixling-priv-broker exists"

# (C) FileDescriptorName must be "priv.sock".
if [ "$FD_NAME" != "priv.sock" ]; then
  fail "socketConfig.FileDescriptorName is '${FD_NAME}', expected 'priv.sock'. \
adopt_listen_fd() validates LISTEN_FDNAMES against this name."
fi
ok "socketConfig.FileDescriptorName = 'priv.sock'"

# (D) ListenSequentialPacket must be /run/nixling/priv.sock.
if [ "$LISTEN_SEQ" != "/run/nixling/priv.sock" ]; then
  fail "socketConfig.ListenSequentialPacket is '${LISTEN_SEQ}', expected \
'/run/nixling/priv.sock'."
fi
ok "socketConfig.ListenSequentialPacket = /run/nixling/priv.sock"

log "==> broker-socket-activation-eval.sh: all checks passed"
