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
  `ReadStdout`/`ReadStderr` offset reads.
- 16 MiB slow stdin is delivered byte-exact through `WriteStdin`, with
  exact-offset append, duplicate replay acceptance, offset-gap rejection,
  stale-data rejection, and bounded stdin backpressure.
- A 30 second slow-consumer run keeps retained output below the configured
  cap and returns an explicit `SlowConsumer` error instead of allocating
  without bound.
- Four concurrent attached sessions meet the proof's p95/max read-latency
  thresholds and finish with bounded byte-skew fairness.
- Restarted sessions reject stale generation tokens.
- TTY Ctrl-D (`0x04`) is data, while EOF is `CloseStdin` at the next
  stdin offset.
- Resize, signal, and exit events share an ordered control sequence, and
  signal exits map to shell-style `128 + signal` status codes.

SSH compatibility is intentionally design-level: existing running VMs that
lack guest-control capability must continue using the SSH lifecycle path
until replacement/restart. The executable proof models the new protocol's
stale-session/restart behavior, not the legacy SSH transport itself.

## SSH compatibility matrix

| VM state | CLI behavior | Compatibility result |
| --- | --- | --- |
| Old running VM without `guest-control` capability | Keep using the existing SSH lifecycle/exec path. | Compatible; no forced restart. |
| New or restarted VM advertising `guest-control` capability | Use chunked stdio RPCs for exec I/O. | New protocol active. |
| VM restarts while a client holds an old generation token | Reject the next RPC as stale. | Fail closed; client must reconnect/rediscover. |
| Guest-control unavailable but SSH still configured | Fall back only through the documented SSH compatibility path. | Operator-visible old-generation behavior. |
| Future removal gate reached | Remove SSH fallback only after the documented migration gate. | No silent behavior change before the gate. |
