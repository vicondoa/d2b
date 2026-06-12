#!/usr/bin/env bash
# Guest exec runtime static guard.
#
# Asserts the ATTACHED non-interactive exec runtime stays inside its scope:
# guestd-local process execution only, with no userd runtime call path, no
# TTY/PTY, no detached retained-log writes in the attached path, no extra
# vsock listeners, no CH CONNECT/relay/host-network surface, no readiness
# wiring, and no `nixling exec` CLI.
#
# It also asserts the DETACHED path (W13) is present-and-bounded: the
# detached registry, transient-unit manager, and exec-runner exist; units
# are slot-keyed (`nixling-exec-<NN>.service`) and carry no opaque exec id
# in the unit name or argv; capabilities are advertised conditionally; and
# the retained-log path is truncation-bounded. The narrow relaxation is
# that retained-log file writes are allowed ONLY in the detached log-store
# (detached_registry.rs) and the exec-runner, never in the attached
# exec.rs/exec_linux.rs path.

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

# No host readiness predicate wiring anywhere.
if rg -n 'ReadinessPredicate::Guest|guest-control-health-readiness' \
  "$ROOT/packages" "$ROOT/nixos-modules" --glob '*.rs' --glob '*.nix'; then
  fail "guest-exec-runtime-static: guest exec must not feed host readiness"
fi

# No `nixling exec` CLI subcommand yet.
if rg -n '^\s*Exec(\b|\()' "$ROOT/packages/nixling/src/lib.rs"; then
  fail "guest-exec-runtime-static: nixling exec CLI surface landed before its wave"
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

ok "guest-exec-runtime-static: attached scope held; detached path present-and-bounded"
