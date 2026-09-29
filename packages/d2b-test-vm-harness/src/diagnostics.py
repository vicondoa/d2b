# The d2b host-integration lane's fixture diagnostics prelude.
#
# This is the one copy of the prelude a check's `testScript` opens with, and
# it is shared deliberately rather than re-provided per surface. The nix lane
# keeps running the fixtures it already has, so the prelude reaches those
# checks by being interpolated into their evaluated `testScript`
# (`tests/host-integration/lib.nix` reads this file); the Bazel lane runs the
# very same evaluated script, so its legacy guest-control surface and the
# ported Rust checks that follow it report through this text rather than
# through a second copy of it that would drift from the first. A check whose
# diagnostics can be read the same way before and after its port is a check
# whose port is reviewable.
#
# The helpers are diagnostics only: no assertion and no timeout declared
# here changes any of them. The lane's guest-control surface
# (`packages/d2b-test-vm-harness/src/legacy.rs`) owns what `machine.*` means, and
# the failure path below is what makes a failed check legible - the stage it
# was in, the rows it was asserting on, the journal lines that explain them,
# and the zone's own account of the row that did not settle.

# ---- d2b fixture diagnostics (issue #513) --------------------------
# The test driver discards machine.execute output and does not re-print
# the output a timed-out wait_until_succeeds last saw, so a failed lane
# used to leave only the command text in the log. These helpers push the
# row set and the daemon explanation lines into the driver log (stdout
# and stderr of the test driver, that is the lane log).
#
# Diagnostics only: no assertion and no timeout is changed here.
import time as _diag_time

_diag_t0 = _diag_time.monotonic()
_diag_stage = "startup"

def _diag_elapsed():
    return f"{_diag_time.monotonic() - _diag_t0:.1f}s"

def _diag_print(*lines):
    for line in lines:
        print(line, flush=True)

def stage(name):
    global _diag_stage
    _diag_stage = name
    _diag_print(f"[d2b] stage={name} t={_diag_elapsed()}")

def diag(command, label="diagnostic output"):
    try:
        status, output = machine.execute(command, timeout=120)
    except Exception as error:
        _diag_print(
            f"[d2b] stage={_diag_stage} t={_diag_elapsed()} {label}: "
            f"diagnostic command failed: {error}"
        )
        return -1
    _diag_print(
        f"[d2b] stage={_diag_stage} t={_diag_elapsed()} {label} "
        f"(exit {status}):"
    )
    _diag_print(command)
    for line in output.rstrip().splitlines():
        _diag_print("    " + line)
    return status

def _diag_journal(unit, token):
    scope = f"-u {unit} " if unit else ""
    select = f"| grep -F -- {token!r} " if token else ""
    return (
        f"journalctl {scope}--no-pager -o cat -b -n 4000 2>/dev/null "
        f"{select}| tail -n 60 || true"
    )

def unit_dumps(unit):
    """Row dumps for a systemd unit waiting to become active."""
    return [
        (
            f"{unit} status",
            f"systemctl status {unit} --no-pager 2>&1 | tail -n 40 "
            "|| true",
        ),
    ]

# Every fixture drives one zone as one linux user through the same
# public socket, so the composed explanation is available without each
# stage listing the rows it asserted on: `d2b debug` reads the whole
# zone and prints the ownership tree, the row that is not settled, and
# the structured failure behind it.
_diag_zone = "work"
_diag_user = "alice"

def diag_debug_zone(label="zone explanation"):
    """The composed `d2b debug` report, always diagnostic and never
    fatal: a failure that happened before the daemon was reachable must
    still print its own stage rather than a diagnostic error. Bounded,
    because a failure can happen before there is anything to explain."""
    status = diag(
        f"runuser -u {_diag_user} -- env "
        f"D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        f"timeout 60 d2b --zone {_diag_zone} debug {_diag_zone} 2>&1 "
        f"|| true",
        label,
    )
    return status

def diag_step(name, action, rows=(), explain=(), wait=None, debug=True):
    stage(name)
    try:
        return action()
    except Exception as error:
        labels = ", ".join(label for label, _ in rows) or "none"
        failing = f" wait={name}" if wait else ""
        _diag_print(
            f"[d2b] FAIL stage={name} t={_diag_elapsed()}{failing} "
            f"rows=[{labels}]: {error}"
        )
        if wait:
            _diag_print(f"[d2b] failing wait: {wait}")
        for label, command in rows:
            diag(command, f"row dump: {label}")
        for unit, token in explain:
            detail = f"journal {unit or 'all'}"
            if token:
                detail += f" lines matching {token!r}"
            diag(_diag_journal(unit, token), detail)
        if debug:
            diag_debug_zone()
        raise

def diag_unit(name, unit, timeout, debug=True):
    """wait_for_unit with the unit status and journal on timeout."""
    return diag_step(
        name,
        lambda: machine.wait_for_unit(unit, timeout=timeout),
        unit_dumps(unit),
        [(unit, None)],
        debug=debug,
    )

def diag_wait(name, command, timeout, rows=(), explain=(), debug=True):
    return diag_step(
        name,
        lambda: machine.wait_until_succeeds(command, timeout=timeout),
        rows,
        explain,
        command,
        debug=debug,
    )

