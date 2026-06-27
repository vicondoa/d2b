# Guest control exec I/O: interactive TTY exec

This document specifies the interactive (PTY-backed) exec path of the
guest-control plane. It is the design follow-up to the interactive-exec
section of [ADR 0028](../adr/0028-guest-control-plane-over-vsock.md) and
builds on the non-interactive contract in the
[chunked stdio reference](./guest-control-exec-io-chunked-stdio.md).

> **Scope note.** This document specifies the interactive TTY exec
> *RPC/service* surface served by `d2b-guestd`. The operator-facing
> `d2b vm exec -it` / `--tty` **CLI** front-end is shipped and
> drives this contract (admin-only, over the authenticated
> guest-control vsock; see
> [`cli-contract.md`](./cli-contract.md) for the verb surface).

## Mode selection

`exec_create` selects the interactive path from two request fields:

| `tty` | `detach` | Path |
| ----- | -------- | ---- |
| `false` | `false` | Non-interactive **attached** exec (separate stdout/stderr, 6 h ceiling). |
| `false` | `true`  | Non-interactive **detached** exec (slot-keyed, retained logs). |
| `true`  | `false` | Interactive **TTY** exec (this document). |
| `true`  | `true`  | Rejected: typed `ProtocolError` (unsupported mode). |

An interactive TTY exec is **connection-owned and non-durable**: it lives only
for the originating ttRPC connection, has no retained-log or registry record,
and is torn down when the connection drops. It is served entirely by
`d2b-guestd`; there is no per-user `d2b-userd` involvement.

## Spawn model: helper-exec, no first-party unsafe

guestd never performs the controlling-terminal handshake and never acquires a
controlling terminal. The full guest stack keeps `unsafe_code = "forbid"`.

1. guestd allocates the PTY master with `posix_openpt(O_RDWR|O_NOCTTY|
   O_CLOEXEC)`, then `grantpt` / `unlockpt` / `ptsname`, and opens the slave
   `O_RDWR|O_NOCTTY|O_CLOEXEC`. The slave's `O_CLOEXEC` is intentional: a
   concurrent fork/exec elsewhere in guestd cannot inherit the slave and keep
   the PTY alive (preserving the HUP/EOF contract). `Stdio::from(slave)` still
   hands fd 0 to the helper because `Command`'s `dup2` clears `CLOEXEC` on the
   duplicate.
2. guestd spawns the static `d2b-exec-runner` in its `--tty-exec` mode via
   `Command`, with the slave on **stdin** (`Stdio::from(OwnedFd)`) and an
   `O_CLOEXEC` status pipe on **stdout**. There are no arbitrary `pass_fds`,
   no `process_group(0)`, and no `pre_exec` closure.
3. The helper (safe `rustix`) `F_DUPFD_CLOEXEC`s the status fd to a high
   number (so the later `dup2` onto fd 1 cannot clobber it), `setsid()`s to
   become a session leader with no controlling terminal, acquires the slave as
   its controlling terminal (`TIOCSCTTY`), applies the initial geometry
   (`tcsetwinsize`), `dup2`s the slave onto fds 0/1/2, and `execve`s the
   target argv.
4. On success the `O_CLOEXEC` status fd closes during `execve`; guestd
   observes EOF on the status pipe (success handshake). On any setup or `exec`
   failure the helper writes one typed status byte and exits; guestd maps that
   byte to a typed `ExecCreate` error.

Because the helper is a session leader and `TIOCSCTTY`'d the slave, the
spawned process is the session leader (`sid == pid`) and the foreground
process group of the terminal.

## Output: merged, stderr disabled

A PTY exposes a single output side, so stdout and stderr are **merged** onto
the stdout stream and read through the normal `ReadOutput(stream=stdout)`
cursor model. The stderr ring is pre-marked EOF at create, so
`ReadOutput(stream=stderr)` on a TTY exec returns a typed
**stderr-unavailable** error rather than blocking or returning data.

## Initial terminal geometry

`initial_terminal_size` is optional:

- **absent** ⇒ the helper applies a default of **24 rows × 80 cols**;
- **present** ⇒ both dimensions must be in `1..=65535`; `0` or out-of-range
  is rejected (validation / protocol error).

## Stdin and CloseStdin (VEOF)

`WriteStdin` uses the same machine as non-interactive exec: a monotonic byte
offset (duplicate / out-of-order offsets are rejected), serialized writes, and
bounded backpressure (non-blocking writes, `WouldBlock`/partial-write handling,
a bounded queue, and per-connection handler + decoded-byte budgets). No PTY fd
lock is held across an `await`.

`CloseStdin` **injects VEOF (`0x04`)** into the PTY line discipline and keeps
the master open. It is **not** a master-close half-close (the master is closed
only on disconnect, cancel, or terminal teardown). `CloseStdin` is idempotent
(a duplicate `CloseStdin` at the same final offset is a no-op success that does
**not** inject a second VEOF), and any subsequent `WriteStdin` is rejected with
a typed **stdin-closed** error. `WriteStdin` with `close_after=true` writes the
data, then injects VEOF.

A `0x04` byte sent through `WriteStdin` is **ordinary stdin data**, not a
protocol close: guestd forwards it to the PTY like any other input. In raw mode
the foreground program receives the literal `0x04`; under canonical mode the
line discipline interprets it (as the `VEOF` control char) exactly as a real
terminal would. Only the explicit `CloseStdin` RPC (or `close_after=true`)
performs the protocol-level stdin close.

## Resize and signal ordering

`TtyWinResize` and `ExecSignal` are serialized through the same per-exec,
**strictly-increasing** `control_seq` dispatcher. A sequence number ≤ the last
accepted value (stale, duplicate, or out-of-order) is rejected; gaps are
allowed.

`ExecSignal` is **TTY-only** and enforces two allowlists:

- **Target allowlist.** Only `FOREGROUND_PROCESS_GROUP` is accepted; any other
  `SignalTarget` is rejected with a `ProtocolError` **before** the sequence is
  consumed. The foreground process group is resolved via `tcgetpgrp(master)`
  **at delivery time** (so it tracks job-control changes inside the session).
  Non-TTY `PROCESS_TREE` signalling is **deferred** — it is not implemented in
  this surface, so a `PROCESS_TREE` (or unspecified/unknown) target is rejected
  with the same typed error.
- **Signal allowlist.** The delivered signal must be one of
  `INT`, `TERM`, `HUP`, `QUIT`, `WINCH`, `USR1`, `USR2`, `KILL`, `TSTP`,
  `CONT`. An out-of-allowlist signal number is rejected **before** the
  sequence is consumed — the `control_seq` is not advanced, so it stays
  available for a subsequent valid control message — and maps to a typed
  `ProtocolError`. There is no dedicated invalid-signal wire kind.

## Runtime ceiling: indefinite, scoped to TTY

A TTY exec runs **indefinitely by default** (`interactiveMaxRuntimeSec = 0` ⇒
unlimited). Setting `d2b.vms.<vm>.guest.exec.interactiveMaxRuntimeSec > 0`
installs an optional ceiling for interactive sessions only. The 6-hour
non-interactive attached ceiling (`MAX_EXEC_RUNTIME_MS`) is unchanged — only
the interactive path opts into unlimited runtime.

## Teardown: Running → Closing → Terminal

Teardown is a three-state machine driven by child exit, ceiling expiry,
explicit cancel, or host disconnect, and is idempotent (the first caller to
enter `Closing` wins the race):

1. **Running → Closing.** Atomically reject new stdin/control RPCs (typed
   no-op/error, no side effect), drop pending accepted writes, stop issuing
   new master clones, and release handles.
2. Drop the last master reference, which delivers `SIGHUP` to the session
   leader.
3. Wait a bounded grace period.
4. `SIGKILL` the whole TTY **session**: enumerate the session's processes by
   `sid` via `/proc` and signal them, repeating until the session is empty.
5. Reap and set the terminal `ExecState` (cancelled vs exited/signaled), then
   notify waiters.

### In-session no-orphan limitation

The no-orphan guarantee is **scoped to processes that remain in the exec's
session**. A child that deliberately escapes the session via `setsid()` or a
double-fork is **not** reaped by the session sweep. This is a documented
trusted-root limitation: the interactive exec target already runs with the
guest user's privileges, so escaping the session is not a privilege boundary.

## Capabilities

`ExecTty`, `TtyResize`, and `Signals` are advertised in the capabilities
response **only when the interactive path is usable** — i.e. the PTY spawner
is wired, which requires the `d2b-exec-runner` helper to be present
(the same gate as the detached exec surface).

## Conformance

The interactive contract is validated by:

- a fake-driven runtime matrix in `d2b-guestd` (fake PTY duplex, fake
  session reaper, and fake clock) covering offset/sequence rejection, VEOF
  injection, target/signal allowlists, the runtime ceiling, and the
  `Running → Closing → Terminal` teardown; and
- a real-PTY, Linux-only integration test in `d2b-exec-runner` that drives
  the `--tty-exec` helper exactly as guestd does and asserts session
  leadership + controlling terminal, the initial winsize, `SIGWINCH`
  delivery on resize, `SIGHUP` on master hangup, status-pipe EOF (no fd
  leak), and in-session reap.
