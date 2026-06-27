# ADR 0038: Persistent named guest shell sessions

- Status: Accepted
- Date: 2026-06-21
- Related: ADR 0015 (daemon-only clean break), ADR 0026 (native SigNoz
  observability backend), ADR 0028 (guest control plane over virtio-vsock),
  ADR 0029 (framework SSH to typed guest-control RPCs), ADR 0030 (guest exec
  runs as the workload user), ADR 0031 (bare commands and detached
  workload-user exec), ADR 0033 (host collector parity and hostname identity),
  ADR 0034 (storage lifecycle, restart adoption, and synchronization), ADR 0035
  (efficiency and simplification roadmap), ADR 0037 (local hypervisor runtime
  seam)

## Context

`d2b vm exec -it <vm> -- <cmd>` gives operators a workload-user terminal
inside a VM, but the terminal is intentionally connection-owned. If the local
client disappears, the command and its terminal lifetime end according to the
exec contract. That is correct for ephemeral commands, but it is a poor fit for
long-lived interactive administration where the operator expects to detach,
reconnect, and resume the same shell state later.

The desired UX is closer to a single-session tmux workflow: named persistent
shells per VM, a default shell name when no name is supplied, list/detach/kill
management verbs, and resume-on-reattach. It is not a terminal multiplexer: one
CLI invocation attaches to one named shell, and the MVP does not expose panes,
windows, shpool templates, shpool variables, or custom start commands.

shpool already provides the core persistent shell behavior. It maintains a
daemon per user, named shell sessions, a single attached client per session,
detach/reattach, force attach, and bounded redraw/spool semantics. Its Rust
library, `libshpool`, is CLI-shaped and process-global: `libshpool::run` is
`unsafe`, can initialize global tracing, can daemonize, and exits the process for
normal CLI-like control flow. Its socket trust model is same-UID: clients are
expected to run as the same workload user as the daemon. D2b should reuse
shpool's persistence model without turning shpool's internal protocol or process
behavior into d2b's public contract.

Existing exec already spent design and implementation effort on terminal
streaming: stdin chunk offsets, output cursors and long-polling, resize control,
raw-mode guards, daemon owner workers, caps, and guest PTY primitives. Adding a
separate shell streaming protocol would duplicate risk and likely drift from
exec's already-validated terminal semantics.

## Decision

D2b will add default-off persistent named guest shell sessions exposed as the
top-level command `d2b shell`.

### Public UX

The public command family is:

```text
d2b shell <vm> [--name NAME] [--force]
d2b shell <vm> attach [--name NAME] [--force]
d2b shell <vm> list [--json]
d2b shell <vm> detach [--name NAME] [--json]
d2b shell <vm> kill --name NAME [--json]
```

`guest.shell.defaultName` defaults to `default`. When the CLI omits `--name`,
d2bd resolves the configured default name from the host manifest for
admission and force-slot decisions, and guestd resolves it again
authoritatively before starting work. `list` includes the default name and marks
it in human output. `detach` may omit `--name` because it is non-destructive and
targets the default. `kill` always requires explicit `--name` because it
terminates a persistent session.

The default name is intentionally a VM configuration value, not a host-username
derivation. That keeps the session identity visible in the manifest, avoids
coupling guest state to host-local account names, and lets operators choose a
different default through Nix when shared-admin behavior is undesirable. Multiple
operators can still avoid collisions with explicit `--name`.

`guest.shell.maxSessions` bounds all persistent shell sessions in the VM,
attached and detached. `guest.shell.maxAttached` bounds live attached shell
clients independently from total persistent sessions and independently from the
generic public connection admission budget.

Session names are plain bounded identifiers, not shpool templates. The accepted
validator is non-empty, at most 64 bytes, first byte `[A-Za-z0-9_]`, remaining
bytes `[A-Za-z0-9._-]`, not `.` or `..`, no whitespace, no `/`, no leading `-`,
and no shpool template markers such as `{` or `}`. CLI, d2bd, and guestd
validate independently.

`--force` is a same-session slot swap: it may evict the currently attached
client for the target session and attach the caller in its place. Attaching to a
new or currently detached session consumes a normal slot from `maxAttached`;
forcing an already-attached session reuses the victim's slot. Victims of force
attach, admin detach, and kill receive distinct terminal results and human
messages.

`d2b shell` is top-level because persistent interactive shells are an
operator-entry command rather than a one-off process execution submode. Its
grammar keeps the VM target first, matching the detached-management pattern used
by `d2b vm exec`: `d2b shell <vm>` implicitly attaches, and management
verbs are subcommands after the VM. A VM named `list`, `detach`, or `kill` is not
ambiguous because the first positional is always the VM. Trailing command-like
arguments after the VM are rejected with guidance to use
`d2b vm exec <vm> -- <cmd>`.

### Shared terminal-v1 substrate

Interactive exec and persistent shell share one terminal streaming substrate.
The implementation will extract terminal-v1 from current exec machinery:

- host raw-mode and nonblocking terminal guards;
- chunked stdin with offsets/retry;
- output cursors and long-polling;
- resize sequencing and terminal status/wait;
- daemon owner-worker admission and teardown;
- guest PTY I/O primitives and bounded buffering.

Exec remains an adapter over this substrate. `d2b vm exec -it` does not
silently become shpool-backed, because exec has a different public contract:
explicit argv, connection-owned lifetime, command exit status, and current
signal behavior. Reconnectable command sessions require a separate future
contract.

Shell is a second adapter over the same substrate. It uses in-band terminal
bytes for Ctrl-C/Ctrl-D/detach keybindings, merged PTY output, and shpool's
redraw/spool model. If guestd's per-attach output ring detects an overflow or
cursor discontinuity, the attach is closed with a typed slow-reader/output-gap
result so the user can reconnect and let shpool redraw a clean terminal state.

### Guest-side helper and shpool daemon

`libshpool` is isolated behind a new guest helper crate and binary named
`d2b-guest-shell-runner`. The crate is intentionally excluded from the main
workspace because the main workspace forbids unsafe code and because
`libshpool::run` is `unsafe` and process-global. The helper has its own
standalone `[workspace]`, direct lint declarations, lockfile, `deny.toml`, and
explicit fmt/clippy/test/cargo-deny/cargo-audit gate wiring through the existing
`make check`/Rust gate orchestrators. It does not inherit main-workspace lints
by implication, and the new workspace does not add a top-level test runner
script.

The helper is built as a fully static musl guest binary through the pkgsStatic
path. The helper and libshpool are compiled without internal PAM support. The
static packaging gates must prove no ELF interpreter, no `NEEDED` dynamic
dependencies, no dynamic PAM/dlopen dependency path, and no accidental
`pam-sys`, `dlopen2`, or libpam closure. Build-time native inputs such as
bindgen/libclang may be allowed only through a narrow documented policy; the
runtime binary remains static.
If libshpool's `libproc` dependency pulls in procps, the static package uses a
procps build without systemd support so libsystemd and dlopen paths cannot enter
the static helper closure.

The helper modes are shpool-shaped but d2b-owned:

- `daemon --socket <path> --home <path>`;
- `attach --socket <path> --name <name> [--force]`;
- `list --socket <path> --json`;
- `detach --socket <path> --name <name>`;
- `kill --socket <path> --name <name>`.

The helper translates shpool output and errors into d2b-owned JSON for
guestd. It does not expose shpool's CLI output or internal protocol as a d2b
contract. Management helper JSON uses bounded pipes or a direct AF_UNIX stream
owned by guestd, not a systemd pipe proxy. Streamed JSON is explicitly framed,
for example with a fixed-size length prefix followed by a bounded JSON payload;
flushing alone is not a message boundary. guestd applies strict byte caps to
helper stdout, stderr, and log streams before buffering or parsing them.

The long-lived shpool daemon runs as the VM workload user, not root. It is a
dormant declarative guest NixOS systemd service, started/adopted/stopped by
guestd on demand. The daemon service owns the workload-user environment for
persistent shells and uses a custom PAM service:

```nix
security.pam.services.d2b-shpool-daemon = {
  startSession = false;
  setEnvironment = true;
  setLoginUid = true;
};
```

The daemon service sets `serviceConfig.User = <workload-user>` and
`serviceConfig.PAMName = "d2b-shpool-daemon"`. The PAM service deliberately
does not start a `pam_systemd` session: session creation would migrate the daemon
out of the delegated system service cgroup and make service-cgroup teardown
unreliable. systemd, running as the dynamic root service manager, owns PAM module
loading and loginuid setup before executing the fully static helper as the
workload user. Because `pam_systemd` is not creating the environment block, the service
`ExecStart` runs a small workload-user shell wrapper that derives the effective
UID with `id -u`, reads the workload user's `HOME`, exports
`XDG_RUNTIME_DIR=/run/user/<uid>` and
`DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/<uid>/bus`, then `exec`s the daemon
helper. The static helper never invokes PAM itself.
guestd does not treat `systemctl start` completion as socket readiness; after
starting or adopting the daemon service, it performs bounded workload-UID
readiness probes before spawning attach or management helpers.

The guest module also enables workload-user linger while `guest.shell.enable`
is true, so `/run/user/<uid>` and the user manager can outlive attached clients
without moving the daemon out of its service cgroup. The daemon uses
`/run/user/<uid>` as
`XDG_RUNTIME_DIR`, and the shpool socket is a filesystem-backed UNIX socket under
that permissioned runtime directory. Abstract namespace sockets are rejected.
shpool's command-less attach path must spawn the workload shell as a login shell,
with a generated NixOS-aware initial `PATH`. D2b
injects `SHELL`, `HOME`, and `USER` explicitly so the static musl helper is not
forced to rely on dynamic NSS lookups for workload users. Per-attach helpers do
not create their own PAM sessions and do not source profiles before shpool
redraws terminal state. They may start as root-owned child processes only long
enough to run the helper's own privileged prelude; before touching the
shpool socket or calling libshpool they must drop to the workload UID and become
workload-UID shpool clients. guestd spawns the helper with a cleared root
environment. Workload or terminal environment values such as `HOME`, `USER`,
`LOGNAME`, `SHELL`, a NixOS-aware `PATH` (`/run/wrappers/bin`,
`/run/current-system/sw/bin`, `/nix/var/nix/profiles/default/bin`,
`/etc/profiles/per-user/$USER/bin`, and absolute `$HOME/.nix-profile/bin`),
`XDG_RUNTIME_DIR=/run/user/<uid>`, `WAYLAND_DISPLAY` when configured, `TERM`,
locale, target cwd, correlation ids, and the explicit shpool socket path are sent
through a root-created pipe/socket or equivalent trusted side channel and are
applied by the helper only after the privilege-drop prelude verifies it is
non-root. Host `TERMINFO_DIRS` is not forwarded into the guest; the guest uses
its own terminfo search path, or a guest-local `TERMINFO_DIRS` such as
`/run/current-system/sw/share/terminfo` if one must be set explicitly. The payload must contain concrete `HOME`, `USER`, and
`LOGNAME` values, not placeholders.

The same-UID socket model is an explicit trust boundary. Code already running as
the workload user may be able to reach the shpool socket. D2b provides
admin-visible reconciliation and typed control, not cryptographic prevention
against same-UID clients.
All filesystem-backed UNIX socket paths used by this feature, including test
socket paths, are preflighted against the platform `sockaddr_un.sun_path` limit
before bind or connect. Tests use deliberately short XDG runtime paths so CI temp
directory nesting cannot produce false `ENAMETOOLONG` failures.

### Async, filesystem, and process-safety rules

Guestd must not call `libshpool` in-process and must not perform blocking file
or process I/O on async executor paths. Helper process spawning uses
`std::process::Command` so guestd can hold the unreaped `Child`, acquire a
race-free pidfd before any async reaper can release the numeric pid, and wrap the
pidfd in `AsyncFd` for readiness. Guestd does not depend on
`tokio::process::Child` for helper lifecycle authority. Long-lived streaming I/O
uses nonblocking pipes or framed AF_UNIX streams; `tokio::fs`,
`tokio::task::spawn_blocking`, or equivalent blocking pools are reserved for
filesystem probes, short bounded file reads, and cleanup.
The helper binary uses a synchronous `fn main()` and explicitly does not use
`#[tokio::main]`. The single-threaded prelude performs environment
sanitization/mutation, including applying the trusted post-drop environment,
before any Tokio runtime, background thread, async logger, libshpool
initialization, shpool socket access, or child process spawn occurs.

Helper-private diagnostics use bounded pipes or framed sockets with
application-level byte accounting and truncation. Regular log files, if kept for
post-mortem debugging, are written by guestd from bounded streams or by
guestd-owned rotation/truncation logic; the helper does not rely on
process-wide `RLIMIT_FSIZE` for log bounding because such limits can leak into
daemon or workload-shell execution. guestd never follows workload-controlled
symlinks as root. Cleanup is an explicit awaited
`cleanup().await`/`shutdown().await` step. Any `Drop` fallback is best-effort
only, cannot spawn detached blocking cleanup that races resource reuse, and is
not authoritative.

When a helper stdout, stderr, log, or framed JSON stream exceeds its byte cap,
guestd immediately cancels or aborts the concurrent stream read futures and
closes pipe read ends or shuts down/closes AF_UNIX streams. Authoritative teardown
uses PID-reuse-safe authority: guestd holds the unreaped `std::process::Child`,
prefers atomic `CLONE_PIDFD` through a stable crate/syscall path, or immediately
calls `pidfd_open(child.id())` while no other waiter can reap the child. guestd
does not convert the held child into `tokio::process::Child`; process readiness
and reaping are driven by the pidfd plus the held standard-library child. An
`ESRCH` from pidfd acquisition is an invariant violation because unreaped zombies
should still have pidfds. The pidfd is put in nonblocking mode before wrapping in
`AsyncFd`. The reaper uses a double-check pattern around readiness clearing:
after readiness, call `try_wait()`; if it returns `None`, clear readiness and
call `try_wait()` again before awaiting another event. Each helper is placed in a
dedicated cgroup or systemd scope before untrusted work begins. The helper
prelude blocks on a trusted root-created isolation-ready byte before dropping
privileges, applying the post-drop environment, connecting to shpool, or spawning
descendants. On cap exceedance, owner teardown, or prelude timeout, guestd kills
the helper cgroup/scope for descendants, sends a pidfd-backed kill to the direct
child if needed, and reaps through the pidfd readiness path. It never
synchronously waits on a helper that may be stuck in D-state. Dropping a child
handle or abandoning a full pipe is not cleanup.

Attach/list/detach/kill helpers run with the workload UID before touching the
shpool socket. The long-lived shpool daemon is different: systemd starts it
directly as the workload user with the PAM/logind session above, and it does not
run the guestd-spawned helper capability-bounding prelude. guestd does not use an
unsafe `CommandExt::pre_exec` closure; the unsafe/syscall privilege setup for
guestd-spawned helpers lives in the excluded `d2b-guest-shell-runner` crate
before libshpool is initialized. In that
single-threaded helper prelude, the helper immediately re-applies `FD_CLOEXEC` to
inherited control/liveness fds, redirects unused standard streams to `/dev/null`,
closes all other unintended fds rather than only marking them close-on-exec,
starts from a safe cwd such as `/`, reads only fixed-size identity data needed for the drop (uid, gid,
length-prefixed supplementary gid list bounded by `NGROUPS_MAX`, and mode flags),
waits for the isolation-ready byte, and only then drops privileges. The drop
order is strict: perform process-group/session isolation (`setpgid(0, 0)` for
non-interactive helpers or `setsid()` for interactive PTY helpers), clear the
capability bounding set and inheritable/ambient capabilities while `CAP_SETPCAP`
is still available, apply the precomputed supplementary groups or an empty list,
reset `oom_score_adj` to the workload/default policy while
it is still writable, then use libc's process-wide credential wrappers directly, or a safe crate such
as `nix::unistd` that delegates to libc, for `setresgid(gid, gid, gid)` and
`setresuid(uid, uid, uid)`, and then set `PR_SET_DUMPABLE = 0` after the uid
change so the kernel cannot reset it during `setresuid`. Do not use rustix raw thread-level credential
syscalls for the process privilege boundary. Target uid 0 is invalid.
Non-daemon helpers set
`PR_SET_NO_NEW_PRIVS`; the long-lived shpool daemon remains exempt so persistent
shells can use `sudo` and other setuid/fcap tools. The helper verifies real,
effective, and saved uid/gid, supplementary groups, no-new-privs where required,
and all capability sets before reading complex/string side-channel frames. It
then applies the trusted post-drop environment, `chdir()`s to the absolute target
cwd, and reports success only after `chdir()` succeeds. If `chdir()` fails, the
helper reports the error and aborts; it never continues from `/`. After the
prelude, an active liveness watcher terminates the helper on supervisor-channel
EOF. Interactive attach
helpers acquire the PTY slave as their controlling terminal (`TIOCSCTTY`) and
configure the foreground process group so kernel terminal signals such as
`SIGWINCH` route to the helper/shpool side. Interactive attach mode uses the
guestd/exec PTY primitives for terminal bytes only; structured JSON for
management verbs and setup/close metadata never shares that PTY. Helper process
identifiers include nonces and never include VM or session names. Long-lived
attach helper child processes are owned and stopped by guestd through the child
handle/pid plus any registered user manager scope cleanup. D2b does not rely
on a `systemd-run` client process or on PTY SIGHUP propagation for authoritative
teardown.

### Lifecycle and persistence

Persistence is live-process persistence. Sessions survive dropped d2b/client
connections and guestd restart when guestd can adopt the still-running shpool
daemon. Sessions do not survive VM reboot or shpool daemon restart/crash. A
shpool daemon epoch is tracked so daemon-only loss cannot silently recreate
empty sessions under old identities.

Detached sessions have no idle TTL in the MVP. That is an accepted operational
risk because `maxSessions` is the hard resource bound for total shell count and
operators can list/kill abandoned sessions. A later idle-timeout policy can be
added only with its own UX and lifecycle contract.

guestd records guest boot id, guestd instance id, shpool daemon instance id, and
opaque shell session instance ids for attach handles and audit correlation.
Session-instance metadata is boot-scoped and root-owned. If a stable shpool
session fingerprint cannot be proven during adoption, d2b emits an
observable reconciliation gap and downgrades exact lifecycle invariants until
state is resynchronized.

### Audit, metrics, and redaction

Admin-initiated events such as list, create/attach, attach close, force detach,
admin detach, and kill are synchronously audited by d2bd with actor
`peer_uid`, the validated 64-byte session name, bounded result enums, and opaque
correlation ids. Involuntary detach events record the acting admin and, when
known, the victim `peer_uid` and `attach_id`. Raw terminal bytes, argv,
process env, cwd/current working directory, raw or unbounded helper stderr, and
unbounded paths never enter audit, logs, spans, or metrics.
On abnormal helper exit, guestd may include a bounded, sanitized helper stderr
snippet or a helper-emitted JSON panic record in structured error logs for
debuggability. Such snippets are byte-capped, control-character escaped, and
never used as metric labels or audit authority.

Guest-observed lifecycle events drain through a cursor-based shell event channel
with sequence numbers, bounded batches, gap markers, deduplication by
`(vm, guestd_instance_id, seq)`, and forced reconciliation on gaps. A guestd
instance change is itself an event-gap boundary because the previous in-memory
queue may have been lost. The guestd event queue has a hard capacity; when it is
full, guestd drops oldest events, records the dropped count, and emits a gap
marker so d2bd can audit degraded state and force reconciliation.

System-induced disconnects and losses are distinct from admin actions. guestd
and d2bd record bounded system/no-actor causes for network loss, owner drop,
daemon OOM/signal/exit, resource kill, orphan reap, and reconciliation gaps so
operators can distinguish expected admin detaches from unexpected session loss.

Daemon startup or runtime loss is not diagnosed solely from socket closure.
guestd queries the systemd unit state and result fields, including OOM, signal,
exit-code, and timeout outcomes where available, and may fetch bounded sanitized
journal excerpts on daemon readiness failure or abnormal daemon exit. The daemon
service clamps verbosity and prevents raw terminal I/O from being written to the
guest journal; startup diagnostics are bounded and sanitized before surfacing to
d2bd.

Metrics use bounded labels only. Session names, attach ids, shell session
instance ids, terminal stream ids, provider resource ids, raw output, stdin,
helper stderr, process env, cwd/current working directory, and paths are not
metric labels. Opaque correlation ids may appear only in redaction-safe structured
logs/spans and audit fields where they are needed for debugging and lifecycle
correlation. Core metrics include gauges for current persistent sessions and
current attached clients, a shell-pool-up gauge, counters for shell operation
outcomes and capacity failures, daemon restart/loss counters, helper failure
counters, output-loss counters, reconciliation counters, and event-drop counters
that increment when the bounded guestd event queue drops events.

Shell management RPCs propagate trace context across the host/guest boundary
using the repository's OpenTelemetry/W3C trace-context conventions so host
`d2bd` spans and guestd spans share one trace root. Metric labels remain
strictly low-cardinality. Structured logs and trace spans may carry
high-cardinality opaque ids or validated session names when needed for debugging,
subject to the same redaction rules above.

### Test and delivery process

This decision record does not ship runtime code. The helper workspace,
orchestrator wiring, and host-integration tests are implementation requirements
for later phases.

Tests follow `tests/AGENTS.md`: Layer 1 first, no new top-level `tests/*.sh`,
and Layer 2 only with justification. Nix module defaults, option values, and
eval rejections for `guest.shell.*` are Type 1 eval cases in
`tests/unit/nix/cases/*.nix`. Rendered manifest/bundle fields for shell
configuration are Type 4 contract tests in `packages/d2b-contract-tests/`.
Static helper packaging assertions, including no ELF interpreter and no dynamic
dependencies, are Type 6 flake checks or existing static derivation checks wired
into the flake; no new top-level shell gate is added for them. The guest helper's
shpool-to-JSON translation, helper CLI parsing, and error mapping get Type 2 unit
tests and/or Type 3 binary integration tests under the helper workspace.
Implementation tests also cover oversized, malformed, fragmented, and partial
control frames; supervisor-channel EOF; inherited nonblocking fds; control-fd
CLOEXEC leakage to descendants; `env_clear`, cwd, and chdir-failure behavior;
prelude timeout kill and pidfd reaping; pidfd nonblocking setup; injected pidfd
fallbacks where applicable; stderr capture for early prelude panics; concurrent
helper caps under stalled helpers; and capability/group verification. Tests that
exercise real uid/gid/capability syscalls use isolated child-process test
binaries and either skip without required root/capabilities, use a user namespace
where appropriate, or run as Type 10 VM tests.
Host-side terminal behavior that does not need a booted VM is Layer 1: add a
Rust integration test (Type 3) that runs the host attach client or CLI in a real
PTY, using a Rust PTY harness, against a mock daemon socket so raw-mode guards,
stdin/stdout handling, and SIGWINCH source are covered in PR CI. The standalone
excluded helper workspace is documented in root `AGENTS.md` and `tests/AGENTS.md`
when it is introduced, including its test paths, so future agents do not assume
one unified workspace. The contributor guide is also updated with local
multi-workspace validation instructions for humans. The initial implementation
phase also includes a justified Type 10 VM test (`runNixOSTest`) under
`tests/host-integration/*.nix` for the load-bearing Linux boundaries that
Layer 1 cannot prove: guest shpool daemon PAM/logind session creation/adoption,
workload-UID helper privilege drop, and real guest helper/PTY teardown. That
phase does not add production guestd runtime integration, user-visible shell UX,
or premature observability hooks such as metrics, audit events, or structured
spans. If a host-side PTY wrapper such as `pexpect` is used in a runNixOSTest,
the test must declare the dependency explicitly through the test's Python package
inputs.

Any future compatibility or migration machinery introduced for this feature uses
ADR 0035 keys shaped like
`compat-ADR<NNNN>-added-<YYYYMMDD>-<surface>-<slug>`.

## Consequences

- Operators get a first-class persistent shell UX without reintroducing SSH or a
  host broker operation.
- The terminal streaming path is reused instead of duplicated, reducing protocol
  and flow-control drift between exec and shell.
- shpool remains an implementation detail behind a static guest helper and a
  d2b-owned protocol.
- The same-UID socket trust boundary is a real limitation: untrusted workload-UID
  services should not be co-located with persistent admin shells.
- The MVP does not provide per-shell cgroup isolation. A pool-level resource
  kill can affect the persistent-shell pool and must surface as a typed pool
  resource-kill/daemon-loss cause.
- The design adds a new excluded Rust workspace that must be kept in fmt,
  clippy, test, deny, audit, static ELF, and dependency-policy gates explicitly.

## Alternatives considered

- **Use tmux or screen.** Rejected because d2b needs typed lifecycle,
  audit/metrics, guest-control integration, and a minimal one-session-per-command
  UX rather than exposing a general terminal multiplexer.
- **Call libshpool inside guestd.** Rejected because `libshpool::run` is unsafe
  and process-global, can daemonize, initializes process state, and exits like a
  CLI.
- **Implement a new persistent shell protocol from scratch.** Rejected because
  shpool already owns the hard persistence/redraw behavior, and exec already owns
  the terminal streaming substrate d2b should reuse.
- **Make interactive exec implicitly persistent.** Rejected because it would
  break exec's command exit-status and connection-owned lifetime contract.
- **Place shell under `d2b vm shell`.** Rejected because persistent shells
  are a first-class operator entry point with their own lifecycle and management
  verbs, not a one-off exec submode. The chosen top-level command still keeps the
  VM target first (`d2b shell <vm> ...`) to match the detached-management
  grammar operators already use with exec.
- **Derive the default shell name from the host operator username.** Rejected
  because host usernames are not a stable guest configuration contract and would
  hide session identity from the manifest. Operators who want per-admin defaults
  can set `guest.shell.defaultName` or pass `--name`.
- **Expose shpool templates, variables, config, or start commands in MVP.**
  Rejected because those expand the trust and UX surface before the basic named
  persistent shell contract is proven.
- **Let the initial implementation phase skip real systemd/PAM/PTY proof.**
  Rejected because later phases depend on those Linux boundaries being true.

## References

- [ADR 0015](0015-daemon-only-clean-break.md)
- [ADR 0026](0026-native-signoz-observability.md)
- [ADR 0028](0028-guest-control-plane-over-vsock.md)
- [ADR 0029](0029-framework-ssh-to-typed-guest-rpc.md)
- [ADR 0030](0030-guest-exec-as-workload-user.md)
- [ADR 0031](0031-bare-command-and-detached-exec.md)
- [ADR 0033](0033-host-collector-parity.md)
- [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md)
- [ADR 0035](0035-efficiency-and-simplification-roadmap.md)
- [ADR 0037](0037-local-hypervisor-runtime-seam.md)
- [../reference/cli-contract.md](../reference/cli-contract.md)
- [../../AGENTS.md](../../AGENTS.md)
- [../../tests/AGENTS.md](../../tests/AGENTS.md)
