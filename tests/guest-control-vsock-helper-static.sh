#!/usr/bin/env bash
# Static guard for the guest-control CH CONNECT helper: the vsock transport
# helper stays confined to the transport module and its sanctioned consumers
# (the W15 guest-control bridge + the W16 exec connector), and is not sprinkled
# across unrelated nixlingd code.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

# `connect_guest_control_vsock*` may appear only in the transport module itself
# and its sanctioned consumers: the guest-control bridge (W15) and the exec
# connector (W16). Any other reference is an unsanctioned wiring of the raw
# transport helper.
if rg -n "connect_guest_control_vsock" "$ROOT/packages/nixlingd/src" \
  --glob '*.rs' \
  | grep -vE 'packages/nixlingd/src/(guest_control_vsock|guest_control_bridge|exec_session_real)\.rs:'; then
  fail "guest-control-vsock-helper-static: helper is wired outside its transport module + sanctioned consumers"
fi

ok "guest-control-vsock-helper-static: helper stays transport-confined"
