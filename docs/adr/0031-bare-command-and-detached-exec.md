# ADR 0031: Bare commands and detached workload-user exec

- Status: Accepted (Unreleased)
- Date: 2026-06-15
- Related: ADR 0015 (daemon-only clean break), ADR 0028 (guest-control
  plane over vsock), ADR 0029 (framework SSH to typed guest-control RPCs),
  ADR 0030 (guest exec as the workload user)

## Context

ADR 0030 moved generic guest exec to the VM's configured workload user
inside a real PAM login session. That made `d2b vm exec -it <vm> --
<shell>` the replacement for the retired SSH-backed console surface, but
operators still had two mismatches with normal login-shell workflows:

1. Command validation required an inconvenient path-oriented invocation
   even though the command ultimately runs behind a login-shell
   `exec "$@"` wrapper.
2. The guest-control wire shape already described detached exec, but the
   shipped daemon and guest intentionally failed closed rather than
   allowing a long-running command to outlive the owner connection before
   the workload-user, quota, and orphan-cleanup model was complete.

The intended operator model is simpler: `d2b vm exec -it <vm> -- bash`
should open a workload-user login shell, `d2b vm exec <vm> -- id`
should resolve `id` through the workload user's login `PATH`, and
long-running non-interactive commands should be startable and manageable
without weakening the never-root guarantee.

## Decision

1. **Bare command names are valid exec programs.** `guestd` accepts a
   non-empty, NUL-free, bounded `argv[0]` that does not begin with `-`.
   Bare names and relative paths are passed positionally to the
   workload user's login shell, whose `exec "$@"` wrapper resolves them
   using the login `PATH`. The command separator remains mandatory in the
   CLI: operators run commands with `d2b vm exec <vm> -- <cmd>
   [args…]`, not by relying on positional guessing.

2. **Invalid program names are a distinct typed error.** Empty
   `argv[0]`, a leading `-`, or another guest-side program-validation
   failure returns the closed `INVALID_PROGRAM` / `invalid-program` kind.
   The CLI maps it to the same usage-class exit as local argument
   mistakes and tells the operator to pass a non-empty command after `--`.

3. **Detached exec is enabled for non-interactive workload-user jobs.**
   `d2b vm exec -d <vm> -- <cmd> [args…]` creates a detached
   guest-control exec and returns an opaque `exec_id` plus its initial
   state. Detached jobs are never TTY or stdin-forwarding sessions:
   `-d` is mutually exclusive with `-i` and `-t`. The workload child runs
   as the configured workload user in a PAM login session, never as root.

4. **The detached runner remains a trusted supervisor, not the workload.**
   The in-guest detached runner keeps the root-owned slot and retained-log
   files trustworthy for quota and reconciliation, but it launches only a
   nested workload unit with the resolved non-root uid. The runner
   re-checks that uid immediately before spawn and fails terminally rather
   than falling back to a direct root command.

5. **Detached management uses a VM-first grammar.** Management verbs live
   after the VM name: `d2b vm exec <vm> list`, `logs <exec_id>`,
   `status <exec_id>`, and `kill <exec_id>`. Management words remain
   valid VM names: because every command form requires `--`, a VM
   literally named `list` still works (`d2b vm exec list -- bash`
   runs a command in that VM; `d2b vm exec list status <id>` asks for
   a detached status in that VM).

6. **Detached state is reconciled and bounded.** Guestd reconciles the
   detached registry before advertising detached capability, re-adopts
   structurally valid runner/workload units, cleans orphaned workload
   units, and runs a periodic reaper for terminal records and retained-log
   slots. Retained stdout/stderr are bounded ring buffers with dropped and
   truncated accounting plus per-stream offsets for resume.

7. **Cancel is idempotent and two-phase.** `kill` is the public CLI name
   for `ExecCancel`: guestd requests graceful termination, waits a bounded
   grace window, then force-kills the detached workload if needed. A
   repeated cancel on a terminal record succeeds with `already-terminal`
   rather than inventing an error.

8. **Audit stays redacted and daemon-local.** Detached create and kill are
   guest-control operations handled by guestd through `d2bd`; they add
   no privileged broker operation. The daemon writes bounded audit events
   containing only `vm`, `peer_uid`, closed action/result enums, and the
   opaque `exec_id`. argv, env, cwd, retained output, and raw logs never
   appear in audit, traces, metrics, or debug output.

## Consequences

- The canonical console replacement is `d2b vm exec -it <vm> -- bash`.
  Scripts can use ordinary commands such as `d2b vm exec <vm> -- id`
  or `d2b vm exec -d <vm> -- long-task` without spelling a store path.
- Detached exec survives host client disconnect and can be inspected,
  logged, or cancelled later through the VM-first management verbs.
- The never-root exec guarantee from ADR 0030 remains intact for both
  attached and detached modes. Operators who need elevation use `sudo`
  inside the workload-user login session.
- No SSH fallback and no new broker audit surface are introduced; the
  feature remains inside the authenticated guest-control plane.
