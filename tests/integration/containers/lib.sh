#!/usr/bin/env bash
# tests/integration/containers/lib.sh — shared helpers for the podman-based integration
# test layer (the type-G container tests).
#
# Design: each test is a standalone bash script in tests/integration/containers/ that
# sources this file, builds its Nix-built OCI image (flake output
# `containerImages.<system>.<name>`, auto-discovered from
# tests/integration/containers/images/*.nix), loads it into podman, runs it, and asserts.
#
# Scope: this layer is ONLY for things that need a foreign (non-Nix) userland
# — e.g. proving a static d2b binary runs on stock Ubuntu, matching CI.
# It is deliberately NOT used to boot systemd and exercise daemon/socket
# activation: that is covered far more cheaply by the native
# `packages/d2b-priv-broker/tests/socket_activation.rs` test (real
# LISTEN_FDS fd-3 handoff + Hello round-trip, ~0.4 s, unprivileged) plus the
# nix-unit unit-shape cases. A faithful systemd-boot container was measured at
# ~1.4 G to ship (the d2b bundle drags the full per-VM runtime substrate,
# so `systemdMinimal` does not help) for zero marginal coverage — do not add
# one. See tests/integration/containers/README.md.
#
# Portability: podman is the container runtime, rootless throughout. It runs
# identically on a NixOS host and on a GitHub Actions ubuntu-latest runner
# (podman is preinstalled there). If podman is not on PATH (e.g. before the
# host `virtualisation.podman.enable` rebuild lands), it is bootstrapped via
# `nix shell nixpkgs#podman`, mirroring the rust-toolchain bootstrap in
# tests/test-rust.sh.

set -euo pipefail

NLC_HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)
NLC_ROOT=${ROOT:-$(cd -- "$NLC_HERE/../../.." >/dev/null 2>&1 && pwd)}
export NLC_ROOT
export NIX_CONFIG="${NIX_CONFIG:-experimental-features = nix-command flakes}"

nlc_log() { printf '%s [containers] %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
nlc_fail() { nlc_log "FAIL: $*"; exit 1; }
nlc_ok() { nlc_log "PASS: $*"; }

# Skip-with-clear-message guard: container tests need podman + the host kernel.
# On a runner without podman AND without nix, skip cleanly (the gate's other
# tiers still run). Sets NLC_PODMAN to the podman invocation prefix.
nlc_require_podman() {
  if command -v podman >/dev/null 2>&1; then
    NLC_PODMAN=(podman)
  elif command -v nix >/dev/null 2>&1; then
    nlc_log "podman not on PATH; bootstrapping via 'nix shell nixpkgs#podman'"
    local podman_path
    podman_path=$(nix shell --quiet nixpkgs#podman --command bash -lc 'command -v podman') \
      || nlc_fail "could not bootstrap podman via nix"
    NLC_PODMAN=("$podman_path")
  else
    nlc_log "SKIP: neither podman nor nix available — container tests need one"
    exit 0
  fi
  export NLC_PODMAN
  # Rootless podman needs a signature policy; provide a permissive user-level one
  # if the host hasn't installed /etc/containers/policy.json yet (pre-rebuild).
  if [ ! -e /etc/containers/policy.json ] && [ ! -e "$HOME/.config/containers/policy.json" ]; then
    mkdir -p "$HOME/.config/containers"
    printf '%s\n' '{ "default": [ { "type": "insecureAcceptAnything" } ] }' \
      > "$HOME/.config/containers/policy.json"
  fi
}

# Build a containerImages.<system>.<name> OCI tarball via the flake, returning
# its store path on stdout. Uses git+file so the build only sees committed files
# (commit-before-build, per AGENTS.md disk hygiene).
nlc_build_image() {
  local name="$1" system
  system=$(nix eval --raw --impure --expr builtins.currentSystem)
  nix build --no-link --print-out-paths \
    "git+file://$NLC_ROOT#containerImages.${system}.${name}" 2>/dev/null \
    | tail -1
}

# Load an OCI tarball into podman, echoing the loaded image reference.
nlc_load_image() {
  local tarball="$1" loaded
  loaded=$("${NLC_PODMAN[@]}" load -i "$tarball" 2>&1 | sed -n 's/^Loaded image: //p' | tail -1)
  [ -n "$loaded" ] || nlc_fail "podman load produced no image ref from $tarball"
  printf '%s\n' "$loaded"
}

# Assert a substring is present in a value.
nlc_assert_contains() {
  local haystack="$1" needle="$2" what="${3:-output}"
  case "$haystack" in
    *"$needle"*) nlc_ok "$what contains '$needle'" ;;
    *) nlc_fail "$what missing '$needle'; got: $haystack" ;;
  esac
}
