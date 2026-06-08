#!/usr/bin/env bash
# v1.1-P5 invariant gate: assert
#   (a) `/var/lib/nixling` is declared `0750 root nixlingd` in
#       `nixos-modules/host-daemon.nix` tmpfiles (NOT 0755), and
#   (b) `nixlingStateDirAcl` activation script in
#       `nixos-modules/host-activation.nix` enumerates per-sidecar
#       traversal ACLs via setfacl (so the 0750 parent does not
#       block sidecar users from reaching their per-VM subdirs).
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

daemon_module="$ROOT/nixos-modules/host-daemon.nix"
activation_module="$ROOT/nixos-modules/host-activation.nix"

fail=0

# (a) 0750 declaration present
if ! grep -q -E '"d /var/lib/nixling 0750 root nixlingd' "$daemon_module"; then
  printf 'state-dir-acl-eval: FAIL — /var/lib/nixling not declared `0750 root nixlingd` in %s\n' "$daemon_module" >&2
  fail=1
fi

# (a) 0755 workaround absent
if grep -q -E '"d /var/lib/nixling 0755' "$daemon_module" "$activation_module"; then
  printf 'state-dir-acl-eval: FAIL — found `0755 /var/lib/nixling` workaround\n' >&2
  fail=1
fi

# (b) nixlingStateDirAcl activation script present
if ! grep -q -E 'nixlingStateDirAcl' "$activation_module"; then
  printf 'state-dir-acl-eval: FAIL — nixlingStateDirAcl activation script missing in %s\n' "$activation_module" >&2
  fail=1
fi

# (b) setfacl invocation present in the ACL block (matches either
# literal /var/lib/nixling or the $state_dir variable that resolves
# to it).
if ! grep -q -E 'setfacl.*"u:[^"]+:--x".*(\$state_dir|var/lib/nixling)' "$activation_module"; then
  printf 'state-dir-acl-eval: FAIL — no `setfacl -m "u:<user>:--x" <state-dir>` invocation found in %s\n' "$activation_module" >&2
  fail=1
fi

# (b) v1.1-P5fu (security closure): `kvm` is a GROUP not a USER;
# enforce `g:kvm:--x` grant separately.
if ! grep -q -E 'setfacl.*"g:kvm:--x"' "$activation_module"; then
  printf 'state-dir-acl-eval: FAIL — `setfacl -m "g:kvm:--x" /var/lib/nixling` grant missing (the v1.1-rc1 bug treated kvm as a user; kvm is a Linux group)\n' >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'state-dir-acl-eval: PASS\n'
