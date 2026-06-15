#!/usr/bin/env bash
# v1.1 invariant gate: assert
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
acl_helper="$ROOT/nixos-modules/host-activation.d/state-dir-acl.sh"

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

# (b) `kvm` is a GROUP not a USER; enforce `g:kvm:--x`
# grant separately.
if ! grep -q -E 'setfacl.*"g:kvm:--x"' "$activation_module"; then
  printf 'state-dir-acl-eval: FAIL — `setfacl -m "g:kvm:--x" /var/lib/nixling` grant missing (the v1.1-rc1 bug treated kvm as a user; kvm is a Linux group)\n' >&2
  fail=1
fi

# (c) `nixling` traversal grant on the state-dir parent. Without it,
# members of `nixling` could not stat
# `${cfg.site.keysDir}/<vm>_ed25519` because /var/lib/nixling had no
# launcher-group traversal ACL — `nixling vm exec` reported
# "ssh key not found" instead of EACCES. The grant is `--x` only
# (chdir, no list / no read); per-VM subdirs keep their own scoped ACLs.
#
# The group rename uses `g:nixling:--x`; until every consumer has the
# renamed helper, match either literal group or helper-substituted group.
if ! {
  grep -q -E 'setfacl.*"g:nixling(-launcher)?:--x"' "$activation_module" \
    || {
      grep -q -F 'host-activation.d/state-dir-acl.sh' "$activation_module" \
        && grep -q -F '"g:$LAUNCHER_GROUP:--x"' "$acl_helper"
    }
}; then
  printf 'state-dir-acl-eval: FAIL — `setfacl -m "g:nixling:--x" /var/lib/nixling` grant missing (v1.2fu58 — operators in nixling cannot reach SSH keys without traversal grant on the state-dir parent)\n' >&2
  fail=1
fi

# (d) NO `setfacl -d -m` default ACL on the state-dir root. A default
# ACL there would make every future
# child directory inherit launcher-group traversal — widening the
# per-VM TPM-state / runner-socket / audit surface that's
# intentionally scoped via `nixlingVmStatePerms`. Only the top-level
# `setfacl -m` grant is allowed.
if grep -nE 'setfacl[[:space:]]+-d[[:space:]]+-m[[:space:]]+"[^"]+".*(\$state_dir|/var/lib/nixling[^/])' "$activation_module" | grep -v '^[[:space:]]*#'; then
  printf 'state-dir-acl-eval: FAIL — found `setfacl -d -m` default ACL on /var/lib/nixling root (security-2 R3: would widen per-VM subdir surface; only the top-level setfacl -m traversal grant is allowed)\n' >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'state-dir-acl-eval: PASS\n'
