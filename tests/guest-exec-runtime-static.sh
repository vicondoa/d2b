#!/usr/bin/env bash
# W12 guest exec runtime static guard.
#
# Asserts the attached non-interactive exec runtime stays inside its scope:
# guestd-local process execution only, with no userd runtime call path, no
# TTY/PTY, no detached retained-log writes, no extra vsock listeners, no CH
# CONNECT/relay/host-network surface, no readiness wiring, and no `nixling
# exec` CLI.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

if ! command -v rg >/dev/null 2>&1; then
  fail "guest-exec-runtime-static: rg is required"
fi

GUESTD_SRC="$ROOT/packages/nixling-guestd/src"
EXEC_SRC="$GUESTD_SRC/exec.rs"
EXEC_LINUX_SRC="$GUESTD_SRC/exec_linux.rs"

# The runtime must exist (this guard is meaningless otherwise).
for required in "$EXEC_SRC" "$EXEC_LINUX_SRC"; do
  if [ ! -f "$required" ]; then
    fail "guest-exec-runtime-static: missing $required"
  fi
done

# No userd runtime call path in the exec runtime.
if rg -n 'userd|nixling-userd' "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: exec runtime must not reference userd"
fi

# No interactive TTY/PTY allocation.
if rg -n 'openpty|forkpty|login_tty|set_controlling|\bpty\b|Pty' "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: attached exec must not allocate a TTY/PTY"
fi

# No detached retained-log file writes from the exec runtime.
if rg -n 'File::create|OpenOptions|fs::write' "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: exec runtime must not write retained log files"
fi

# stdin must be closed (redirected to /dev/null), never piped/open.
if ! rg -q 'Stdio::null' "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: spawned children must redirect stdin to /dev/null"
fi
if rg -n 'stdin\(Stdio::piped' "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: spawned children must not pipe stdin"
fi

# The only vsock listener lives in the service transport, not the exec runtime.
if rg -n 'VsockListener|VsockAddr' "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: exec runtime must not open its own vsock listener"
fi

# No CH CONNECT / relay / host firewall / observability surface in the runtime.
if rg -n 'CONNECT|nftables|iptables|/etc/hosts|otel|exporter|prometheus' \
  "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: exec runtime must not touch host network/observability surfaces"
fi

# No host readiness predicate wiring anywhere.
if rg -n 'ReadinessPredicate::Guest|guest-control-health-readiness' \
  "$ROOT/packages" "$ROOT/nixos-modules" --glob '*.rs' --glob '*.nix'; then
  fail "guest-exec-runtime-static: guest exec must not feed host readiness"
fi

# No `nixling exec` CLI subcommand yet.
if rg -n '^\s*Exec(\b|\()' "$ROOT/packages/nixling/src/lib.rs"; then
  fail "guest-exec-runtime-static: nixling exec CLI surface landed before its wave"
fi

ok "guest-exec-runtime-static: attached exec runtime stays within W12 scope"
