#!/usr/bin/env bash
# Live host USBIP guestd lifecycle check.
#
# Requires a deployed d2b host, a USBIP-enabled running VM, and a real
# busid. This is intentionally Layer 2/manual: it restarts d2bd and mutates
# live USBIP attachment state through `d2b usb` only.
#
# Usage:
#   D2B_LIVE=1 D2B_USBIP_VM=work-ssd D2B_USBIP_BUSID=1-2.1 \
#     bash tests/integration/live/usbip-guestd-lifecycle.sh

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

if [ "${D2B_LIVE:-0}" != "1" ]; then
  log "SKIP: set D2B_LIVE=1 to run live USBIP lifecycle checks"
  exit 0
fi

vm=${D2B_USBIP_VM:-}
busid=${D2B_USBIP_BUSID:-}
if [ -z "$vm" ] || [ -z "$busid" ]; then
  log "SKIP: set D2B_USBIP_VM and D2B_USBIP_BUSID"
  exit 0
fi

if ! command -v jq >/dev/null 2>&1; then
  fail "jq is required"
  exit 1
fi

status_for() {
  d2b usb probe --json \
    | jq -r --arg vm "$vm" --arg bus "$busid" '
      .entries[]
      | select(.vm == $vm and .busId == $bus)
      | [.status, (.ownerVm // "")] | @tsv
    ' \
    | head -n1
}

require_owner() {
  local expected_owner=$1
  local row status owner
  row=$(status_for)
  status=${row%%$'\t'*}
  owner=${row#*$'\t'}
  [ "$status" = "bound" ] || fail "expected $busid to be bound to $expected_owner, got status=$status owner=$owner"
  [ "$owner" = "$expected_owner" ] || fail "expected $busid owner $expected_owner, got $owner"
  ok "$busid bound to $expected_owner"
}

initial=$(status_for || true)
initial_status=${initial%%$'\t'*}
initial_owner=${initial#*$'\t'}
if [ "$initial_status" = "bound" ] && [ "$initial_owner" != "$vm" ]; then
  log "SKIP: $busid is owned by $initial_owner, not $vm"
  exit 0
fi

restore() {
  if [ "$initial_status" = "bound" ] && [ "$initial_owner" = "$vm" ]; then
    d2b usb attach --apply "$vm" "$busid" >/dev/null 2>&1 || true
  else
    d2b usb detach --apply "$vm" "$busid" >/dev/null 2>&1 || true
  fi
}
trap restore EXIT

log "detaching stale guest/host USBIP state"
d2b usb detach --apply "$vm" "$busid" >/dev/null

log "attaching through daemon/broker/guestd"
d2b usb attach --apply "$vm" "$busid" >/dev/null
require_owner "$vm"

log "restarting d2bd to verify adoption"
if ! sudo -n true >/dev/null 2>&1; then
  fail "passwordless sudo is required to restart d2bd"
  exit 1
fi
sudo -n systemctl restart d2bd.service
sleep 5
require_owner "$vm"

log "detaching after daemon restart"
d2b usb detach --apply "$vm" "$busid" >/dev/null

log "reattaching after daemon restart"
d2b usb attach --apply "$vm" "$busid" >/dev/null
require_owner "$vm"

ok "USBIP guestd lifecycle survived daemon restart and reattach"
