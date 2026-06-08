#!/usr/bin/env bash
# tests/per-vm-state-ownership-eval.sh — ph2-p2-ownership-matrix
# eval-time regression test.
#
# Asserts the canonical per-VM state ownership matrix declared in
# nixos-modules/options-ownership-matrix.nix is:
#   * non-empty
#   * every entry carries all required typed fields
#     (path, owner, group, mode, recursive, description)
#   * mode strings are octal (3 or 4 digits)
#   * the `store` entry exists and has `recursive = false`
#     (hardlink-farm carve-out — recursive ownership ops would leak
#     ACLs into /nix/store via the shared inodes)
#   * the `swtpm` entry exists and has owner/group templated with
#     `<vm>` (per-VM swtpm runner principal)

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/per-vm-state-ownership-eval.sh"

expr=$(cat <<EOF
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
        };
      })
    ];
  };
  matrix = nixos.config.nixling.daemon.perVmStateOwnershipMatrix;
  storeEntry = builtins.head (builtins.filter (e: e.path == "store") matrix);
  swtpmEntry = builtins.head (builtins.filter (e: e.path == "swtpm") matrix);
  octalRe = "^[0-7]{3,4}$";
  fieldsOk = e:
       (builtins.isString e.path)
    && (builtins.isString e.owner)
    && (builtins.isString e.group)
    && (builtins.match octalRe e.mode != null)
    && (builtins.isBool e.recursive)
    && (builtins.isString e.description && e.description != "");
in {
  count = builtins.length matrix;
  allFieldsOk = builtins.all fieldsOk matrix;
  hasStore = builtins.any (e: e.path == "store") matrix;
  hasSwtpm = builtins.any (e: e.path == "swtpm") matrix;
  hasRoot = builtins.any (e: e.path == ".") matrix;
  storeRecursive = storeEntry.recursive;
  swtpmOwnerTemplated = builtins.match ".*<vm>.*" swtpmEntry.owner != null;
  swtpmGroupTemplated = builtins.match ".*<vm>.*" swtpmEntry.group != null;
  storeMode = storeEntry.mode;
  paths = map (e: e.path) matrix;
}
EOF
)

json=$(nix-instantiate --eval --strict --json --expr "$expr" 2>&1) \
  || fail "eval failed: $json"

count=$(printf '%s' "$json" | jq -r '.count')
[ "$count" -ge 5 ] || fail "matrix has $count entries; expected at least 5"
ok "matrix is non-empty ($count entries)"

[ "$(printf '%s' "$json" | jq -r .allFieldsOk)" = "true" ] \
  || fail "at least one entry is missing required typed fields"
ok "every entry has typed {path, owner, group, mode, recursive, description}"

for required in hasStore hasSwtpm hasRoot; do
  [ "$(printf '%s' "$json" | jq -r ".$required")" = "true" ] \
    || fail "matrix missing required entry: $required"
done
ok "matrix contains the canonical {., store, swtpm} entries"

storeRec=$(printf '%s' "$json" | jq -r '.storeRecursive')
if [ "$storeRec" != "false" ]; then
  fail "CRITICAL: store entry has recursive=$storeRec; MUST be false (hardlink-farm carve-out — recursive ops would leak ACLs into /nix/store via shared inodes)"
fi
ok "hardlink-farm carve-out: store entry has recursive=false"

[ "$(printf '%s' "$json" | jq -r .swtpmOwnerTemplated)" = "true" ] \
  || fail "swtpm entry owner must use <vm> template"
[ "$(printf '%s' "$json" | jq -r .swtpmGroupTemplated)" = "true" ] \
  || fail "swtpm entry group must use <vm> template"
ok "swtpm entry owner/group are per-VM templated"

paths=$(printf '%s' "$json" | jq -r '.paths | join(",")')
log "  matrix paths: $paths"

log "==> tests/per-vm-state-ownership-eval.sh OK"
