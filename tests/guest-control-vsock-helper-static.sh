#!/usr/bin/env bash
# Static guard for the W7 guest-control CH CONNECT helper.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if rg -n "connect_guest_control_vsock" "$ROOT/packages/nixlingd/src" \
  --glob '*.rs' \
  | grep -v 'packages/nixlingd/src/guest_control_vsock.rs:'; then
  fail "guest-control-vsock-helper-static: helper is wired outside its transport module"
fi

if rg -n "ReadinessPredicate::Guest|guest-control-health-readiness" \
  "$ROOT/packages" "$ROOT/nixos-modules" \
  --glob '*.rs' --glob '*.nix'; then
  fail "guest-control-vsock-helper-static: guest-control readiness predicate emitted before health runtime"
fi

ok "guest-control-vsock-helper-static: helper remains transport-only"
