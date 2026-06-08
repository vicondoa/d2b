#!/usr/bin/env bash
# tests/state-dir-acl-runtime.sh — Layer-2 with-sudo test.
#
# Adversarial coverage for the `g:nixling:--x` traversal ACL on
# /var/lib/nixling. Creates a throwaway state-dir + a synthetic
# `nixling` user, runs the `nixlingStateDirAcl` activation
# block against it, and asserts the documented authorization
# boundary:
#
#   - launcher member CAN stat /<state-dir>/keys/<vm>_ed25519
#     (traversal works)
#   - launcher member CANNOT read the file contents
#     (named-group ACL is read-only via the per-file ACL, but the
#     test asserts traversal + stat work)
#   - non-launcher user gets EACCES on the stat (no traversal)
#
# Layer-2 + root-only. Skipped if not root unless
# NL_RUN_LAYER2_WITH_SUDO=1. Documented in tests/README.md §
# "Layer-2 with-sudo CI hook".
#
# CI path: .github/workflows/layer2-runtime-with-sudo.yml runs this
# script when the workflow runs on a `nixling-sudo`-labeled runner.
# If no such runner is registered, the workflow job stays "pending"
# (visible signal, not silent skip).

set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> tests/state-dir-acl-runtime.sh"

if [ "$(id -u)" -ne 0 ]; then
  if [ "${NL_RUN_LAYER2_WITH_SUDO:-0}" != "1" ]; then
    log "state-dir-acl-runtime: SKIP (not root; set NL_RUN_LAYER2_WITH_SUDO=1 to opt in via sudo)"
    exit 0
  fi
  exec sudo -n -E NL_RUN_LAYER2_WITH_SUDO=1 bash "$0" "$@"
fi

# Per the disk-hygiene contract, scratch state goes under nl_mktemp,
# NOT raw mktemp -d -p "$ROOT".
scratch=$(nl_mktemp -d "state-dir-acl-runtime")
# nl_mktemp/mktemp -d creates 0700; the synthetic non-root test
# users below must be able to traverse $scratch (and the
# intermediate $scratch/var, $scratch/var/lib parents) to reach the
# state-dir under test. Without this the ACL on $state_dir itself
# is moot because the test users hit EACCES on the parent traversal
# first. The mode is `--x` (0711) only — directory contents stay
# unreadable, only known names are
# traversable, mirroring the trust boundary documented on the real
# /var/lib path.
chmod 0711 "$scratch"
trap 'chmod -R u+rwX "$scratch" 2>/dev/null || true; rm -rf "$scratch" 2>/dev/null || true' EXIT INT TERM

# Synthetic users created for this test. Names use the `nl-test-`
# prefix so they cannot collide with real per-VM principal names
# (`nixling-<vm>-*`) or the lifecycle group (`nixling`).
test_launcher_user="nl-test-launcher-$$"
test_outsider_user="nl-test-outsider-$$"
test_launcher_group="nl-test-launcher-grp-$$"

cleanup_users() {
  userdel "$test_launcher_user" 2>/dev/null || true
  userdel "$test_outsider_user" 2>/dev/null || true
  groupdel "$test_launcher_group" 2>/dev/null || true
}
trap 'cleanup_users; chmod -R u+rwX "$scratch" 2>/dev/null || true; rm -rf "$scratch" 2>/dev/null || true' EXIT INT TERM

groupadd "$test_launcher_group"
useradd -M -N -g "$test_launcher_group" -s /sbin/nologin "$test_launcher_user"
useradd -M -N -s /sbin/nologin "$test_outsider_user"

state_dir="$scratch/var/lib/nixling"
keys_dir="$state_dir/keys"
key_file="$keys_dir/test-vm_ed25519"

# Mirror the 0711 traversal grant on the intermediate parents so the
# synthetic users can reach $state_dir. Real-world /var and /var/lib
# are 0755 on every distro; 0711 here is strictly tighter (no list).
install -d -m 0711 -o root -g root "$scratch/var"
install -d -m 0711 -o root -g root "$scratch/var/lib"

install -d -m 0750 -o root -g root "$state_dir"
install -d -m 0710 -o root -g "$test_launcher_group" "$keys_dir"
echo "stub-key" > "$key_file"
chown "root:$test_launcher_group" "$key_file"
chmod 0640 "$key_file"

# Apply the traversal ACL through the same helper sourced by
# nixos-modules/host-activation.nix, using the synthetic launcher group.
# shellcheck source=../nixos-modules/host-activation.d/state-dir-acl.sh
STATE_DIR="$state_dir" \
  LAUNCHER_GROUP="$test_launcher_group" \
  SETFACL_BIN=setfacl \
  . "$ROOT/nixos-modules/host-activation.d/state-dir-acl.sh"
setfacl -m "g:$test_launcher_group:r--" "$key_file"

# Sanity: ACL applied.
if ! getfacl -p "$state_dir" 2>/dev/null | grep -q "group:$test_launcher_group:--x"; then
  echo "FAIL: setfacl on $state_dir did not register" >&2
  exit 1
fi

# Test 1: launcher member CAN stat the key file (traversal works).
if ! sudo -n -u "$test_launcher_user" stat "$key_file" >/dev/null 2>&1; then
  echo "FAIL: launcher-group member cannot stat $key_file (traversal grant not effective)" >&2
  exit 1
fi
log "PASS: launcher member can stat key file"

# Test 2: launcher member CAN read the key file (named-group ACL r--).
if ! sudo -n -u "$test_launcher_user" cat "$key_file" >/dev/null 2>&1; then
  echo "FAIL: launcher-group member cannot read $key_file (named-group ACL not effective)" >&2
  exit 1
fi
log "PASS: launcher member can read key file"

# Test 3: non-launcher user CANNOT stat the key file (no traversal,
# no read). Expect EACCES from stat(2).
if sudo -n -u "$test_outsider_user" stat "$key_file" >/dev/null 2>&1; then
  echo "FAIL: non-launcher user can stat $key_file (unexpected traversal grant)" >&2
  exit 1
fi
log "PASS: non-launcher user correctly denied stat"

# Test 4: non-launcher user CANNOT list the keys directory contents
# (the per-VM keys-dir mode is 0710 root:nixling; non-member
# has no list permission even though the state-dir parent is --x).
if sudo -n -u "$test_outsider_user" ls "$keys_dir" >/dev/null 2>&1; then
  echo "FAIL: non-launcher user can list $keys_dir (keys-dir mode bits broken)" >&2
  exit 1
fi
log "PASS: non-launcher user correctly denied list"

# Test 5: the traversal grant is --x ONLY — launcher member CANNOT
# list the state-dir contents (only traverse to known names).
if sudo -n -u "$test_launcher_user" ls "$state_dir" >/dev/null 2>&1; then
  echo "FAIL: launcher member can list $state_dir contents (ACL should be --x only, not r-x)" >&2
  exit 1
fi
log "PASS: launcher member correctly denied list (traverse-only)"

log "state-dir-acl-runtime: PASS (5 invariants verified)"
