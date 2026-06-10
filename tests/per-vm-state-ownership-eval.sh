#!/usr/bin/env bash
# tests/per-vm-state-ownership-eval.sh—
# eval-time regression test.
#
# Asserts the canonical per-VM state ownership matrix declared in
# nixos-modules/options-ownership-matrix.nix is:
#   * non-empty
#   * every entry carries all required typed fields
#     (path, owner, group, mode, kind, required, recursive, description)
#   * mode strings are octal (3 or 4 digits); kind is "dir" | "file";
#     required/recursive are booleans
#   * the `store` entry exists, has `recursive = false` (hardlink-farm
#     carve-out — recursive ownership ops would leak ACLs into
#     /nix/store via the shared inodes) and `required = false` (legacy
#     recovery artifact, absent on native post-cutover VMs)
#   * the `swtpm` entry exists and has owner/group templated with
#     `<vm>` (per-VM swtpm runner principal)
#   * the signed store-view layout (ADR 0027) is encoded:
#       - `store-view/meta` exists and the legacy `store-view/generations`
#         path is gone;
#       - `store-view/sync.lock` is file-kind `nixling:nixling 0600`;
#       - the live readiness marker is file-kind, `required = false`,
#         `nixling:users 0644`;
#       - `store-view/meta` is dir-kind runner-readable `nixling:users
#         0755`;
#       - host-only `store-view/state` and `store-view/gcroots` are
#         `nixling:nixling 0750` (NOT runner-readable `users 0755`).
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
  syncLockEntry = builtins.head (builtins.filter (e: e.path == "store-view/sync.lock") matrix);
  metaEntry = builtins.head (builtins.filter (e: e.path == "store-view/meta") matrix);
  stateEntry = builtins.head (builtins.filter (e: e.path == "store-view/state") matrix);
  gcrootsEntry = builtins.head (builtins.filter (e: e.path == "store-view/gcroots") matrix);
  markerEntry =
    builtins.head (builtins.filter (e: e.path == "store-view/live/.nixling-marker-<vm>") matrix);
  octalRe = "^[0-7]{3,4}$";
  fieldsOk = e:
       (builtins.isString e.path)
    && (builtins.isString e.owner)
    && (builtins.isString e.group)
    && (builtins.match octalRe e.mode != null)
    && (e.kind == "dir" || e.kind == "file")
    && (builtins.isBool e.required)
    && (builtins.isBool e.recursive)
    && (builtins.isString e.description && e.description != "");
in {
  count = builtins.length matrix;
  allFieldsOk = builtins.all fieldsOk matrix;
  hasStore = builtins.any (e: e.path == "store") matrix;
  hasSwtpm = builtins.any (e: e.path == "swtpm") matrix;
  hasRoot = builtins.any (e: e.path == ".") matrix;
  hasStoreViewMeta = builtins.any (e: e.path == "store-view/meta") matrix;
  hasNoLegacyGenerations = !(builtins.any (e: e.path == "store-view/generations") matrix);
  storeRecursive = storeEntry.recursive;
  storeRequired = storeEntry.required;
  swtpmOwnerTemplated = builtins.match ".*<vm>.*" swtpmEntry.owner != null;
  swtpmGroupTemplated = builtins.match ".*<vm>.*" swtpmEntry.group != null;
  storeMode = storeEntry.mode;
  storeGroup = storeEntry.group;
  # signed store-view layout assertions
  syncLockKind = syncLockEntry.kind;
  syncLockMode = syncLockEntry.mode;
  syncLockGroup = syncLockEntry.group;
  metaKind = metaEntry.kind;
  metaMode = metaEntry.mode;
  metaGroup = metaEntry.group;
  stateMode = stateEntry.mode;
  stateGroup = stateEntry.group;
  gcrootsMode = gcrootsEntry.mode;
  gcrootsGroup = gcrootsEntry.group;
  markerKind = markerEntry.kind;
  markerMode = markerEntry.mode;
  markerGroup = markerEntry.group;
  markerRequired = markerEntry.required;
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

# Signed store-view layout (ADR 0027): kind/required schema + posture.
[ "$(printf '%s' "$json" | jq -r .hasStoreViewMeta)" = "true" ] \
  || fail "matrix must contain the guest meta share root store-view/meta"
[ "$(printf '%s' "$json" | jq -r .hasNoLegacyGenerations)" = "true" ] \
  || fail "matrix must NOT reference the retired store-view/generations path"
ok "matrix has store-view/meta and dropped the legacy store-view/generations path"

[ "$(printf '%s' "$json" | jq -r .storeRequired)" = "false" ] \
  || fail "legacy store entry must be required=false (posture-if-present on native VMs)"
ok "legacy store artifact is required=false"

# Broker-private lock: file-kind, nixling:nixling 0600.
[ "$(printf '%s' "$json" | jq -r .syncLockKind)" = "file" ] \
  || fail "store-view/sync.lock must be kind=file"
[ "$(printf '%s' "$json" | jq -r .syncLockMode)" = "0600" ] \
  || fail "store-view/sync.lock must be mode 0600 (broker-private)"
[ "$(printf '%s' "$json" | jq -r .syncLockGroup)" = "nixling" ] \
  || fail "store-view/sync.lock must be group nixling (host-only)"
ok "store-view/sync.lock is file-kind nixling:nixling 0600"

# Live readiness marker: file-kind, optional, guest-readable nixling:users 0644.
[ "$(printf '%s' "$json" | jq -r .markerKind)" = "file" ] \
  || fail "live readiness marker must be kind=file"
[ "$(printf '%s' "$json" | jq -r .markerRequired)" = "false" ] \
  || fail "live readiness marker must be required=false (absent before first sync)"
[ "$(printf '%s' "$json" | jq -r .markerMode)" = "0644" ] \
  || fail "live readiness marker must be mode 0644 (guest-readable)"
[ "$(printf '%s' "$json" | jq -r .markerGroup)" = "users" ] \
  || fail "live readiness marker must be group users (runner/guest-readable)"
ok "live readiness marker is file-kind, optional, nixling:users 0644"

# Runner-readable guest meta share root: dir-kind nixling:users 0755.
[ "$(printf '%s' "$json" | jq -r .metaKind)" = "dir" ] \
  || fail "store-view/meta must be kind=dir"
[ "$(printf '%s' "$json" | jq -r .metaMode)" = "0755" ] \
  || fail "store-view/meta must be mode 0755 (runner-readable)"
[ "$(printf '%s' "$json" | jq -r .metaGroup)" = "users" ] \
  || fail "store-view/meta must be group users (runner-readable)"
ok "store-view/meta is dir-kind runner-readable nixling:users 0755"

# Host-only state/gcroots: nixling:nixling 0750, NOT runner-readable users 0755.
for pair in "stateMode:0750:store-view/state mode" "stateGroup:nixling:store-view/state group" \
            "gcrootsMode:0750:store-view/gcroots mode" "gcrootsGroup:nixling:store-view/gcroots group"; do
  key=${pair%%:*}; rest=${pair#*:}; want=${rest%%:*}; label=${rest#*:}
  got=$(printf '%s' "$json" | jq -r ".$key")
  [ "$got" = "$want" ] || fail "$label must be $want (host-only), got $got"
done
ok "store-view/state and store-view/gcroots are host-only nixling:nixling 0750 (not users 0755)"

paths=$(printf '%s' "$json" | jq -r '.paths | join(",")')
log "  matrix paths: $paths"

store_nix="$ROOT/nixos-modules/store.nix"
grep -Fq 'find "$META_DIR" -type d -exec chown nixlingd:users {} +' "$store_nix" \
  || fail "nixling-store-sync must chown only meta directory inodes to nixlingd:users"
grep -Fq 'find "$META_DIR" -type d -exec chmod 0755 {} +' "$store_nix" \
  || fail "nixling-store-sync must chmod only meta directory inodes to 0755"
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
