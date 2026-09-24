# U52 d2b-provider-supervisor

Lane audit (fresh pass per U1 ledger; all prior U52 findings refused).

## Ledger verification (refusal-honoring - no new evidence)

Prior lane ledger rows, each **refused**, re-verified against the current
tree (HEAD `3f2664794`); refusal grounds still hold:

- **#G53 [refused] hand-rolled blocking executor (16 threads, deadline
  thread, custom waker)** - `ProviderSupervisor` ordering executor remains
  the crate's blocking process-effect seam; `d2bd` awaits launch with **no
  timeout** (`packages/d2bd/src/process_provider_runtime.rs:59` names the
  generic `ProviderSupervisor<B: ProcessEffectBackend>`); tokio is
  dev-only here. `src/adapter.rs` keeps the executor, `src/broker.rs`
  keeps `MAX_PENDING_OBSERVATIONS`.
- **#G54 [refused] hand-rolled seqpacket broker transport** - broker stub
  `src/broker.rs` (seqpacket envelope carrier, `OpenPidfd`/`ObserveRunner`
  leg) has unambiguously live production callers in
  `packages/d2bd/src/process_provider_runtime.rs` and the daemon's
  `d2bd-runtime` pidfd table, all named through the trusted broker socket
  profile and role seam (`with_socket_and_role`, `BrokerPidfdHandle`).
  Provider crates may not depend on d2bd transport; seam stays.
- **#G55 [refused] generic systemd seam with exactly one production
  implementation** - `SystemdProcessProvider::new(ProviderSupervisor::new(
  SystemdProcessBackend::new(...)))` is constructed in
  `process_provider_runtime.rs:59`, and `BrokerSystemdEffectOwner`
  envelope seam has a live caller in `d2bd/src/process_provider_runtime.rs`
  (constructing `BrokerSystemdEffectOwner::with_socket_and_role`);
  the generic trait surface stays behind the crate boundary as refused.
- **#G60 [refused] suite scrapes broker error-kind string via `include_str`
  (`packages/d2b-provider-supervisor/tests/`)** - no exported constant
  exists for the broker error kind; `d2b-contracts-broker` is out of lane.
  The scrape and cross-crate `compile_data` stay, as recorded.

## Fresh-audit result

No new applied findings in this crate beyond the prior ledger. All
`src/` code is live; the two "competing" helpers the audit considered
(dup `read_proc_start_time`, hand-rolled `matches_peer_process` without
broker-side deadline) were already measured-and-refused in the prior pass
and their callers remain (adapter.rs, broker.rs, d2bd consumer). No new
dead string constants, no new test-only flow, no new zero-caller surface
introduced at HEAD.

net: 0 lines, 0 deps - reuse of prior refusals; nothing new to cut.

## Consistency notes

Not a types/contracts crate; supervisor is a leaf adapter twin (U14
family). No cross-crate duplicate type definitions or wire-shape skew
found in this crate beyond the already-recorded #G54/#G55 seams.

## Checked

- `src/adapter.rs`, `src/broker.rs`, `src/systemd.rs` (full + ranges);
- `src/lib.rs` export surface; `tests/production_adapter.rs` + integration
  seams; workspace-wide callers of `matches_peer_process`,
  `read_proc_start_time_pub`, broker transport, `runner_role_for_process_role`,
  `ProviderSupervisor`, `MAX_PENDING_*` consts (broker.rs, systemd.rs).
- Confirmed no new dead code at HEAD relative to the held refusals.
- Ledger obligations honored: no reopened refusals; no new findings
  beyond refused-lane scope.
