#!/usr/bin/env bash
# tests/per-vm-state-ownership-eval.sh—
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
#   * store.nix's sync helper enforces the same non-recursive
#     `nixlingd:users 0755` shape for store/store-meta directories,
#     never the pre-daemon `root:kvm 0755` shape.

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
  storeGroup = storeEntry.group;
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

store_nix="$ROOT/nixos-modules/store.nix"
grep -Fq 'find "$STORE_DIR" "$META_DIR" -type d -exec chown nixlingd:users {} +' "$store_nix" \
  || fail "nixling-store-sync must chown only store/store-meta directory inodes to nixlingd:users"
grep -Fq 'find "$STORE_DIR" "$META_DIR" -type d -exec chmod 0755 {} +' "$store_nix" \
  || fail "nixling-store-sync must chmod only store/store-meta directory inodes to 0755"
if grep -Fq 'chown root:kvm' "$store_nix"; then
  fail "nixling-store-sync still contains legacy root:kvm store ownership fix-up"
fi
if grep -Fq 'chmod 2775' "$store_nix"; then
  fail "nixling-store-sync must not grant group write on store/store-meta directories"
fi
if grep -R 'store store-meta' "$ROOT/nixos-modules" \
    | grep -E -- '--mode 2775|chmod 2775' >/dev/null; then
  fail "nixos-modules still contain store/store-meta 2775 enforcement"
fi
if grep -n 'path: "store' "$ROOT/packages/nixlingd/src/ownership_preflight.rs" -A5 \
    | grep -Fq 'mode: 0o2775'; then
  fail "daemon canonical ownership preflight still expects store/store-meta 2775"
fi
ok "store-sync directory ownership fix-up matches ownership matrix"

log "==> tests/per-vm-state-ownership-eval.sh OK"
