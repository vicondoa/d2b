#!/usr/bin/env bash
# Guest exec runtime static guard.
#
# Asserts the ATTACHED non-interactive exec runtime stays inside its scope:
# guestd-local process execution only, with no userd runtime call path, no
# low-level TTY/PTY syscalls, no detached retained-log writes in the attached
# path, no extra vsock listeners, and no CH CONNECT/relay/host-network surface.
#
# It also asserts the DETACHED path (W13) is present-and-bounded: the
# detached registry, transient-unit manager, and exec-runner exist; units
# are slot-keyed (`nixling-exec-<NN>.service`) and carry no opaque exec id
# in the unit name or argv; capabilities are advertised conditionally; and
# the retained-log path is truncation-bounded. The narrow relaxation is
# that retained-log file writes are allowed ONLY in the detached log-store
# (detached_registry.rs) and the exec-runner, never in the attached
# exec.rs/exec_linux.rs path.
#
# Finally it asserts the INTERACTIVE TTY path (W14) is present-and-confined:
# the guestd-side PTY mechanism (master allocation) lives in exec_pty.rs and
# the controlling-terminal handshake (setsid + TIOCSCTTY) lives ONLY in the
# static `--tty-exec` helper of the exec-runner. exec.rs/exec_linux.rs may
# reference the PtyProcessSpawner/SpawnedPtyProcess/TtyState *type names* (the
# runtime drives the spawner trait) but must never perform a PTY syscall or
# the controlling-terminal handshake themselves — guestd NEVER acquires a
# controlling tty (the no-first-party-unsafe crux of the W14 design).

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

# No LOW-LEVEL TTY/PTY syscalls in the ATTACHED exec runtime. The interactive
# (W14) PTY mechanism lives entirely in exec_pty.rs (the guestd-side spawner)
# and the exec-runner `--tty-exec` helper; exec.rs/exec_linux.rs may reference
# the PtyProcessSpawner/SpawnedPtyProcess/TtyState *type names* (the runtime
# drives the spawner trait) but must never allocate a PTY or perform the
# controlling-terminal handshake themselves. The `\bsetsid\(` / `openpt\(` call
# forms keep prose mentions (e.g. the no-orphan limitation comment) from
# tripping the guard.
if rg -n 'openpty|forkpty|login_tty|set_controlling|openpt\(|grantpt\(|unlockpt\(|ptsname\(|ioctl_tiocsctty|\bsetsid\(' "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: attached exec must not perform low-level TTY/PTY syscalls (the PTY mechanism lives in exec_pty.rs + the --tty-exec helper)"
fi

# No detached retained-log file writes from the ATTACHED exec runtime.
# (Detached retained-log writes are allowed only in detached_registry.rs and
# the exec-runner; see the detached present-and-bounded assertions below.)
if rg -n 'File::create|OpenOptions|fs::write' "$EXEC_SRC" "$EXEC_LINUX_SRC"; then
  fail "guest-exec-runtime-static: attached exec runtime must not write retained log files"
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

# --- Detached path (W13): present-and-bounded, not absent. ---

DETACHED_REGISTRY_SRC="$GUESTD_SRC/detached_registry.rs"
DETACHED_UNIT_SRC="$GUESTD_SRC/detached.rs"
RUNNER_SRC="$ROOT/packages/nixling-exec-runner/src"

# The detached runtime must exist (this part of the guard is meaningless
# otherwise).
for required in "$DETACHED_REGISTRY_SRC" "$DETACHED_UNIT_SRC"; do
  if [ ! -f "$required" ]; then
    fail "guest-exec-runtime-static: missing detached source $required"
  fi
done
if [ ! -d "$RUNNER_SRC" ]; then
  fail "guest-exec-runtime-static: missing exec-runner source dir $RUNNER_SRC"
fi

# Transient units are slot-keyed and carry no opaque exec id in the unit
# name. `nixling-exec-<NN>.service` is the only allowed shape.
if ! rg -q 'nixling-exec-\{slot' "$DETACHED_UNIT_SRC"; then
  fail "guest-exec-runtime-static: detached units must be slot-keyed (nixling-exec-<NN>)"
fi

# The opaque exec id must NEVER appear in the unit name or systemd-run argv.
# (It is confined to the spec/status files under the slot dir.) Scope this to
# production code: the test module legitimately asserts the *absence* of the
# token, which would otherwise be a false positive.
detached_unit_prod=$(sed '/^#\[cfg(test)\]/,$d' "$DETACHED_UNIT_SRC")
if printf '%s\n' "$detached_unit_prod" | rg -n 'exec_id'; then
  fail "guest-exec-runtime-static: opaque exec id must not appear in unit name/argv"
fi

# Detached transient units are scoped to the dedicated guest-internal slice.
if ! rg -q 'nixling-exec\.slice' "$DETACHED_UNIT_SRC"; then
  fail "guest-exec-runtime-static: detached units must be scoped to nixling-exec.slice"
fi

# The retained-log path is truncation-bounded (drop-oldest accounting).
if ! rg -q 'truncated|dropped' "$ROOT/packages/nixling-exec-runner/src/filering.rs"; then
  fail "guest-exec-runtime-static: detached retained logs must be truncation-bounded"
fi

# Capabilities are advertised conditionally (usability-aware), not always-on.
if ! rg -q 'EXEC_DETACHED|EXEC_LOGS' "$GUESTD_SRC/service.rs"; then
  fail "guest-exec-runtime-static: detached/logs capabilities must be wired conditionally"
fi

# The detached parent dir + slice are declared in the guest module.
if ! rg -q '/run/nixling-exec' "$ROOT/nixos-modules/guest-control.nix"; then
  fail "guest-exec-runtime-static: guest module must declare /run/nixling-exec parent dir"
fi
if ! rg -q 'nixling-exec' "$ROOT/nixos-modules/guest-control.nix"; then
  fail "guest-exec-runtime-static: guest module must declare the nixling-exec slice"
fi

# --- Interactive TTY path (W14): present-and-confined, not absent. ---

EXEC_PTY_SRC="$GUESTD_SRC/exec_pty.rs"
TTY_HELPER_SRC="$RUNNER_SRC/tty_helper.rs"

for required in "$EXEC_PTY_SRC" "$TTY_HELPER_SRC"; do
  if [ ! -f "$required" ]; then
    fail "guest-exec-runtime-static: missing interactive TTY source $required"
  fi
done

# The guestd-side PTY spawner owns master allocation (openpt/grantpt/unlockpt/
# ptsname). Confining it to exec_pty.rs is what keeps exec.rs PTY-syscall-free.
if ! rg -q 'openpt\(' "$EXEC_PTY_SRC"; then
  fail "guest-exec-runtime-static: exec_pty.rs must own PTY master allocation (openpt)"
fi

# The controlling-terminal handshake (setsid + TIOCSCTTY) lives ONLY in the
# static --tty-exec helper. This is the no-first-party-unsafe crux of W14:
# guestd never acquires a controlling tty.
if ! rg -q '\bsetsid\b' "$TTY_HELPER_SRC" || ! rg -q 'ioctl_tiocsctty' "$TTY_HELPER_SRC"; then
  fail "guest-exec-runtime-static: --tty-exec helper must perform the setsid + TIOCSCTTY handshake"
fi

# guestd (exec_pty.rs) must NOT perform that handshake itself. Strip line
# comments first so the design-rationale prose that *names* setsid/TIOCSCTTY
# does not trip the guard.
exec_pty_code=$(rg -v '^\s*//' "$EXEC_PTY_SRC")
if printf '%s\n' "$exec_pty_code" | rg -n 'ioctl_tiocsctty|\bsetsid\b'; then
  fail "guest-exec-runtime-static: guestd must not perform the controlling-terminal handshake (it routes through the --tty-exec helper)"
fi

# The TTY merged-output contract surfaces a typed stderr-unavailable error;
# the wire mapping must be wired in the service layer.
if ! rg -q 'TtyStderrUnavailable' "$GUESTD_SRC/service.rs"; then
  fail "guest-exec-runtime-static: TTY stderr-unavailable wire mapping missing from service.rs"
fi

ok "guest-exec-runtime-static: attached scope held; detached path present-and-bounded; interactive TTY path present-and-confined"
