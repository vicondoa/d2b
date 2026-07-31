#!/usr/bin/env bash
# grant-kvm.sh - grant or revoke this user's access to /dev/kvm.
#
# WHY THIS EXISTS
#   On this host /dev/kvm is mode 0660 root:kvm with a POSIX ACL that lists only
#   the d2b role users. The interactive operator account is NOT in group `kvm`
#   and cannot open it. The lab needs KVM, and adding the operator to group
#   `kvm` (or creating a dedicated lab user) would be a persistent host
#   configuration change, which the lab's no-host-switch rule forbids.
#
#   So we take the narrowest reversible action available: a runtime POSIX ACL
#   entry, removed again on exit and in any case wiped by udev on reboot.
#
# ACCEPTED, UNRESOLVED RISK (AE-1 in the plan)
#   While the grant is in place, ANY process running as this user can open
#   /dev/kvm -- not just the lab. Revocation does not close already-open file
#   descriptors, and an EXIT trap cannot run if the launcher is SIGKILLed or the
#   machine crashes. Run `grant-kvm.sh --revoke` after any hard crash.
#
#   The stronger fix (a dedicated lab UID holding the ACL) requires one
#   persistent host user. If that ever becomes acceptable, it closes AE-1.
set -euo pipefail

DEV=/dev/kvm
# Derive from `id -un`, not $USER: the environment variable can be stale or
# spoofed, and this value decides whose ACL we modify on a privileged device.
USER_NAME="${SUDO_USER:-$(id -un)}"

usage() {
  cat >&2 <<EOF
usage: ${0##*/} [--grant|--revoke|--status|--has-acl]

  --grant    add a rw ACL entry for '$USER_NAME' on $DEV (needs sudo)
  --revoke   remove that ACL entry (needs sudo)
  --status   report whether '$USER_NAME' can currently open $DEV
  --has-acl  exit 0 iff an ACL entry for '$USER_NAME' exists on $DEV
             (used by the launcher's teardown; needs no sudo)

Default is --status. Grants are non-persistent: udev restores the original
ACL on reboot.
EOF
  exit 2
}

# Truth is "can we actually open it", not "does an ACL line exist" -- group
# membership or a permissive mode would also work, and we should not ask for
# sudo we do not need.
can_open() {
  [ -r "$DEV" ] && [ -w "$DEV" ] && : <>"$DEV" 2>/dev/null
}

status() {
  if can_open; then
    echo "ok: '$USER_NAME' can open $DEV"
    return 0
  fi
  echo "denied: '$USER_NAME' cannot open $DEV"
  return 1
}

need_setfacl() {
  command -v setfacl >/dev/null 2>&1 ||
    { echo "error: setfacl not found (install acl)" >&2; exit 1; }
}

case "${1:---status}" in
  --status) status ;;
  --has-acl)
    # Distinct from --status: asks whether an ACL ENTRY exists for this user,
    # not whether the device happens to be openable (group membership or a
    # permissive mode would also allow that). Teardown uses this so it can
    # revoke a grant it did not create, without needing sudo just to look.
    getfacl -p "$DEV" 2>/dev/null | grep -q "^user:${USER_NAME}:"
    ;;
  --grant)
    if can_open; then
      echo "already granted: '$USER_NAME' can open $DEV; not touching the ACL"
      exit 0
    fi
    need_setfacl
    echo "granting rw on $DEV to '$USER_NAME' (reversible, non-persistent)" >&2
    sudo setfacl -m "u:${USER_NAME}:rw" "$DEV"
    status
    ;;
  --revoke)
    need_setfacl
    # -x fails if no such entry exists; that is fine and means nothing to undo.
    if sudo setfacl -x "u:${USER_NAME}" "$DEV" 2>/dev/null; then
      echo "revoked rw on $DEV for '$USER_NAME'" >&2
    else
      echo "no ACL entry for '$USER_NAME' on $DEV; nothing to revoke" >&2
    fi
    ;;
  -h|--help) usage ;;
  *) usage ;;
esac
