"""The Python side of the lane's legacy guest-control surface.

A check that has not been ported yet is a Python script: the `testScript` of
the fixture it used to be, with the shared diagnostics prelude already
interpolated at its top. This module is everything around that script - the
`machine` object its assertions call and the `start_all` they open with - and
deliberately nothing else. Every operation is a request to the lane's
harness, which owns the guest and speaks the legacy driver's own protocol to
it, so the semantics of an operation live in exactly one place: the guest a
command runs under, the retry bounds, the wording of a refused assertion, and
the log lines a reader of the lane sees around them.

The reason it is worth being that thin is the transition itself. Every
unported check has to keep gating the lane with the assertions it already
has, so this surface has to behave like the one it replaces rather than like a
new one: the same operations, the same defaults, the same messages, and the
same exceptions, so a check that fails here fails with what it would have
failed with under the driver that is being retired. When a check ports, its
assertions move into Rust against the same guest-control primitives, and the
diagnostics text it reports through does not move at all.
"""

import json
import os
import socket
import sys
import traceback


class MachineError(Exception):
    """The guest could not be reached, or answered something unreadable.

    Named for the driver's own exception of that name, so a traceback from an
    unported check reads the way it read before the port.
    """


class RequestedAssertionFailed(Exception):
    """An assertion the check made did not hold.

    Named for the driver's own exception of that name. The message is the
    driver's message, word for word, because the lane's diagnostics prelude
    prints it: a check's failure line is that text, and a different wording
    would be a different failure report for the same failure.
    """


class _Machine:
    """The `machine` object an unported check's assertions call.

    Every method is a request: the harness performs the operation against the
    guest, including every retry inside it, and answers with the result or
    with the failure. A check therefore blocks on one guest operation exactly
    as it blocked on one under the driver, and the bounds it declares are the
    bounds the harness waits by.
    """

    # The driver's own defaults, restated rather than defaulted in the
    # harness: a check that omits an argument has to get the bound the
    # driver gave it, and the harness is told which bound was asked for
    # rather than choosing one of its own.
    EXECUTE_TIMEOUT = 900
    WAIT_TIMEOUT = 900

    def __init__(self, control):
        self._control = control

    def _call(self, op, *args, **kwargs):
        return self._control.request(op, args, kwargs)

    def execute(self, command, check_return=True, check_output=True, timeout=EXECUTE_TIMEOUT):
        """Run a shell command, returning `(status, output)`."""
        status, output = self._call(
            "execute",
            command,
            check_return=check_return,
            check_output=check_output,
            timeout=timeout,
        )
        return status, output

    def succeed(self, *commands, timeout=None):
        """Run each command in turn, refusing anything that exits non-zero."""
        return self._call("succeed", *commands, timeout=timeout)

    def fail(self, *commands, timeout=None):
        """Run each command in turn, refusing anything that exits zero."""
        return self._call("fail", *commands, timeout=timeout)

    def wait_until_succeeds(self, command, timeout=WAIT_TIMEOUT):
        """Retry a command with one-second intervals until it succeeds."""
        return self._call("wait_until_succeeds", command, timeout=timeout)

    def wait_for_file(self, filename, timeout=WAIT_TIMEOUT):
        """Wait until a path exists in the guest."""
        return self._call("wait_for_file", filename, timeout=timeout)

    def wait_for_unit(self, unit, user=None, timeout=WAIT_TIMEOUT):
        """Wait until a systemd unit is active, refusing a failed one at once."""
        return self._call("wait_for_unit", unit, user=user, timeout=timeout)

    def sleep(self, secs):
        """Sleep in guest time, the way the driver slept in guest time."""
        return self._call("sleep", secs)

    def __getattr__(self, name):
        # An operation the lane's surface does not carry is named here rather
        # than raising an attribute error about a private attribute: a check
        # that reaches for it is a check whose port has a gap, and the gap
        # should read as the operation that is missing. A dunder is left to
        # raise the plain error, so the interpreter's own lookups are not
        # answered with a message about the lane.
        if name.startswith("__") and name.endswith("__"):
            raise AttributeError(name)
        raise AttributeError(
            "the lane's legacy guest-control surface has no operation "
            f"{name!r}; the legacy driver's surface is the set this lane "
            "re-provides, and an assertion calling anything else needs a port"
        )


class _Control:
    """The request channel to the lane's harness."""

    def __init__(self, path):
        self._socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._socket.connect(path)
        self._reader = self._socket.makefile("rwb")

    def request(self, op, args, kwargs):
        self._reader.write(
            json.dumps({"op": op, "args": args, "kwargs": kwargs}).encode() + b"\n"
        )
        self._reader.flush()
        line = self._reader.readline()
        if not line:
            raise MachineError(
                "the lane's harness closed the guest-control channel while "
                f"the check was waiting for `{op}` to finish"
            )
        answer = json.loads(line)
        if not answer.get("ok"):
            # The harness distinguishes the two failures the driver
            # distinguished: a refused assertion is the check's own verdict
            # and is reported as one, and a guest the surface could not reach
            # is a lane failure the check cannot be blamed for.
            failure = (
                RequestedAssertionFailed
                if answer.get("assertion")
                else MachineError
            )
            raise failure(answer.get("error") or "the operation failed with no message")
        value = answer.get("value")
        # JSON has no tuple, and `execute` returns one: a check that unpacks
        # or indexes the result is reading the driver's `(status, output)`.
        return tuple(value) if isinstance(value, list) else value


def _report_failure():
    """Print a failure the way the driver printed it.

    Every line carries the prefix the driver gave its test errors, and the
    traceback carries the refusal as its last line, so a reader of a lane log
    gets the assertion that failed and what it failed on without the two
    being separated by anything.
    """
    for line in traceback.format_exc().splitlines():
        print("!!! " + line, file=sys.stderr)


def start_all():
    """The fixtures' first call, and a no-op on this surface.

    The lane owns the guest: it booted the guest, waited for the guest's own
    activation contract, and holds the guest for the whole run. There is
    nothing left for a check to start, and the call stays because the
    assertion bodies around it are unchanged until each check ports.
    """


def main():
    if len(sys.argv) != 3:
        print(
            "usage: legacy_bridge.py <control-socket> <check-script>",
            file=sys.stderr,
        )
        return 2
    control_path, script_path = sys.argv[1], sys.argv[2]
    machine = _Machine(_Control(control_path))
    with open(script_path) as script:
        source = script.read()
    symbols = {
        "__name__": "__main__",
        "machine": machine,
        "start_all": start_all,
    }
    try:
        exec(compile(source, "<check script>", "exec"), symbols)
    except Exception:  # noqa: BLE001 - a check's own failure, whatever kind
        # Every failure a check can raise is reported the same way, because a
        # reader of the lane log should not have to know which exception class
        # the assertion behind a red check happened to use.
        _report_failure()
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
