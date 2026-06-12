#!/usr/bin/env bash
# tests/guest-config-containment-eval.sh — eval-time containment gate
# for the per-VM guest-editable `guestConfigFile`.
#
# `nixling.vms.<vm>.guestConfigFile` is the guest-editable OS layer
# (the surface the in-VM config-sync workflow edits). It must be
# CONTAINED: it may set only guest OS options, never host-owned
# `microvm.*` / `nixling.*` options. assertions.nix enforces this with
# a hard assertion driven by `lib.nix`'s
# `guestConfigForbiddenNamespaces` check, which evaluates the guest file
# over the real nixpkgs NixOS module set with `microvm`/`nixling`
# redeclared as detector options and reports any namespace the guest
# defines (by definition-existence, so imports / generated modules /
# `_file` spoofing are all caught).
#
# This gate evaluates a minimal consumer-style nixosSystem whose
# corp-vm sets `guestConfigFile` to each fixture under
# `eval-cases/guest-fixtures/` and asserts:
#   - clean-guest.nix          -> no containment assertion fires.
#   - reads-standard-option.nix -> no containment assertion fires (reads
#                                  a standard option without crashing).
#   - sets-microvm.nix         -> rejected, naming the microvm.* options.
#   - sets-nixling.nix         -> rejected, naming the nixling.* option.
#   - imports-microvm.nix      -> rejected (bypass via imports).
#   - tofile-microvm.nix       -> rejected (bypass via builtins.toFile).
#   - spoof-file.nix           -> rejected (bypass via _file spoofing).
#
# Run via: tests/guest-config-containment-eval.sh   (wired into static.sh)

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=$(dirname "$HERE")

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export NIX_CONFIG='experimental-features = nix-command flakes'

PASS=0
FAIL=0
log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS + 1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL + 1)); }

# eval_failing_messages <absolute-fixture-path>
# Prints the JSON array of FAILING assertion messages for a host
# config whose corp-vm uses the given guestConfigFile.
eval_failing_messages() {
  local fixture="$1" tmpdir tmp out
  tmpdir=$(nl_mktemp .containment-eval.XXXXXX)
  tmp="$tmpdir/eval.nix"
  cat > "$tmp" <<NIX
let
  system = builtins.currentSystem;
  flake = builtins.getFlake "git+file://${ROOT}";
  nixos = flake.inputs.nixpkgs.lib.nixosSystem {
    inherit system;
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
        nixling.site = { waylandUser = "alice"; launcherUsers = [ "alice" ]; yubikey.enable = false; };
        nixling.envs.work = { lanSubnet = "10.20.0.0/24"; uplinkSubnet = "192.0.2.0/30"; };
        nixling.vms.corp-vm = {
          enable = true; env = "work"; index = 10; ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
          guestConfigFile = ${fixture};
        };
      })
    ];
  };
  failing = builtins.filter (a: !a.assertion) nixos.config.assertions;
in
  map (a: a.message) failing
NIX
  out=$(nix eval --impure --json -f "$tmp" 2>/dev/null)
  printf '%s' "$out"
}

# --- clean guest config: must produce NO failing assertion ----------
log "==> clean-guest.nix (only guest OS options)"
clean=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/clean-guest.nix")
if [ "$clean" = "[]" ]; then
  ok "contained guest config evaluates with no containment failure"
else
  fail "clean guest config unexpectedly produced assertions: $clean"
fi

# --- guest READS a standard option: must NOT false-positive/crash ----
# A guest module that reads `config.networking.hostName` in a mkIf guard
# is contained (it sets only a guest OS option). The containment check
# must evaluate it over real NixOS option context, not crash.
log "==> reads-standard-option.nix (reads a standard config.* option)"
rd=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/reads-standard-option.nix")
if [ "$rd" = "[]" ]; then
  ok "guest reading a standard option is contained (no false positive)"
else
  fail "guest reading a standard option spuriously failed containment: $rd"
fi

# --- guest sets microvm.* : must be rejected, naming the options ----
log "==> sets-microvm.nix (host-owned microvm.*)"
mv=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/sets-microvm.nix")
if printf '%s' "$mv" | grep -q "may only set" \
   && printf '%s' "$mv" | grep -q "microvm.mem" \
   && printf '%s' "$mv" | grep -q "microvm.cloud-hypervisor"; then
  ok "guest setting microvm.* is rejected and the options are named"
else
  fail "guest microvm.* containment did not fire as expected: $mv"
fi

# --- guest sets nixling.* : must be rejected, naming the option -----
log "==> sets-nixling.nix (host-owned nixling.*)"
nv=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/sets-nixling.nix")
if printf '%s' "$nv" | grep -q "may only set" \
   && printf '%s' "$nv" | grep -q "nixling.sshUser"; then
  ok "guest setting nixling.* is rejected and the option is named"
else
  fail "guest nixling.* containment did not fire as expected: $nv"
fi

# --- BYPASS #1: forbidden option set via an imported module ---------
# A `definitionsWithLocations == guestConfigFile` check would miss this
# (the def is attributed to the imported file); definition-existence
# detection must still reject it.
log "==> imports-microvm.nix (bypass via imports)"
iv=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/imports-microvm.nix")
if printf '%s' "$iv" | grep -q "may only set" \
   && printf '%s' "$iv" | grep -q "microvm.mem"; then
  ok "guest setting microvm.* via an IMPORTED module is still rejected"
else
  fail "containment BYPASS via imports not caught: $iv"
fi

# --- BYPASS #2: forbidden option via a builtins.toFile module --------
# The forbidden def comes from a module generated at eval time (no
# on-disk source path). Definition-existence detection must catch it.
log "==> tofile-microvm.nix (bypass via builtins.toFile)"
tv=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/tofile-microvm.nix")
if printf '%s' "$tv" | grep -q "may only set" \
   && printf '%s' "$tv" | grep -q "microvm.mem"; then
  ok "guest setting microvm.* via a builtins.toFile module is still rejected"
else
  fail "containment BYPASS via builtins.toFile not caught: $tv"
fi

# --- BYPASS #3: forbidden option with a spoofed module `_file` -------
log "==> spoof-file.nix (bypass via _file spoofing)"
sv=$(eval_failing_messages "$HERE/eval-cases/guest-fixtures/spoof-file.nix")
if printf '%s' "$sv" | grep -q "may only set" \
   && printf '%s' "$sv" | grep -q "microvm.mem"; then
  ok "guest setting microvm.* with a SPOOFED _file is still rejected"
else
  fail "containment BYPASS via _file spoofing not caught: $sv"
fi

log "guest-config-containment-eval: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
