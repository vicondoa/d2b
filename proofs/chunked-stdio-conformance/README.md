# Chunked stdio conformance proof

This crate is executable evidence for the selected Kata-style chunked
stdio guest-control protocol described in
`docs/reference/guest-control-exec-io-chunked-stdio.md`.

Run from the repository root:

```bash
cargo test --manifest-path proofs/chunked-stdio-conformance/Cargo.toml
```

The tests prove:

- 64 MiB stdout plus 64 MiB stderr are byte-exact through
  `ReadStdout`/`ReadStderr` offset reads; zero-length reads and appends
  after output EOF are rejected.
- 16 MiB slow stdin is delivered byte-exact through `WriteStdin`, with
  exact-offset append, same-request duplicate replay acceptance,
  different-request stale replay rejection, offset-gap rejection,
  stale-data rejection, drainable bounded stdin retention, and bounded
  stdin backpressure.
- Simulated pipe and PTY partial child writes drain from the bounded stdin
  queue without exposing duplicate or lost bytes at the RPC offset boundary.
- Atomic `WriteStdin.close_after` succeeds, fails, and replays
  idempotently with endpoint-specific close semantics: pipe-backed stdin
  closes only after queued bytes drain, while TTY close leaves PTY output
  readable and does not synthesize EOF/HUP.
- Per-connection decoded-byte budget and per-exec stdin permits bound
  malicious concurrent `WriteStdin` fan-in.
- A deterministic active slow-consumer stress keeps retained output below
  the configured cap while producers continue attempting stdout/stderr
  writes and receive explicit `SlowConsumer` errors instead of allocating
  without bound.
- Four concurrent attached sessions, including a mixed deterministic
  scheduler with slow-output, blocked-stdin, interactive echo, and
  unary-health load, meet bounded service-turn and fairness thresholds
  without relying on wall-clock timing.
- Restarted sessions reject stale generation tokens.
- TTY Ctrl-D (`0x04`) is data, while EOF is `CloseStdin` at the next
  stdin offset.
- Resize, signal, and cancel events share an ordered client control
  sequence with `request_id` replay for identical retained requests and
  typed rejection for mismatched duplicate IDs.
- Process exit status is recorded separately from client controls, is
  visible only after preceding output is retained, delivered/acknowledged,
  or explicitly dropped with cursor accounting, rejects future
  ACK/accounting cursors, stays hidden after unaccounted loss, and maps
  signal exits to shell-style `128 + signal` status codes.

SSH compatibility is intentionally design-level: existing SSH-backed commands
such as `config sync` and `vm konsole` continue using their current SSH path
for old running VMs until replacement/restart. The new `nixling exec` and
`nixling vm exec run` commands never fall back to SSH. The executable proof
models the new protocol's stale-session/restart behavior, not the legacy SSH
transport itself.

## SSH compatibility matrix

| VM state | CLI behavior | Compatibility result |
| --- | --- | --- |
| Old running VM without `guest-control` capability and existing SSH-backed command (`config sync`, `vm konsole`) | Keep using that command's current SSH path with `transport: "ssh-compat"` and remediation. | Compatible; no forced restart. |
| Old running VM without `guest-control` capability and new generic exec (`nixling exec`, `nixling vm exec run`) | Return typed `guest-control-unavailable-old-generation`; do not use SSH. | Fail closed; no new generic SSH exec surface. |
| New or restarted VM advertising `guest-control` capability | Use chunked stdio RPCs for exec I/O. | New protocol active. |
| VM restarts while a client holds an old generation token | Reject the next RPC as stale. | Fail closed; client must reconnect/rediscover. |
| Guest-control unavailable but SSH still configured | Fall back only through the documented SSH compatibility path. | Operator-visible old-generation behavior. |
| Future removal gate reached | Remove SSH fallback only after the documented migration gate. | No silent behavior change before the gate. |
