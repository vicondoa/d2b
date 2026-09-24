# U4 d2b-contracts-broker

Lean already. Ship.

Wire-only typed contract crate (~5,338 LOC across broker_wire.rs, host_generation.rs,
kernel_client.rs, generated broker_operation_profiles.rs, lib.rs). Audited the public surface
that could plausibly be dead and verified every candidate against workspace-wide callers:

- `BrokerRequest`/`BrokerResponse` wire envelope + 50+ typed request/response shapes —
  live on both legs: constructed/decoded by `d2b-broker` (runtime.rs, ops/*), `d2bd`,
  `d2bd-runtime`, and the provider crates (supervisor, network-local, process, gpu...).
- `BrokerCapabilities::w3()` — broker_operations list read by `d2b-broker` catalog.rs:751
  and the profile-separation + wire tests; `HelloResponse`/`HelloRequest` exercised by
  `d2bd` composition + daemon hello frames and the socket_activation / profile tests.
- `BrokerCapabilities` / `BrokerProfile` / `BrokerCallerRole` — `allowed_by_profile`,
  `allows_operation`, `allows_request`, `guest_operations`, `operations()` all read at
  runtime.rs:1636,1900 and tests/profile_separation.rs + host/guest profile tests.
- `security_key_authority_binding`, `security_key_authority_binding` fingerprint helper —
  called by d2b-broker security_key op + effects_service; `security_key_authority_binding`
  (hash-bound selector) consumed by d2bd + d2bd-composition.
- `KernelInvocation`/`KernelReply`/`envelope_invoke_kernel` — dialed by supervisor,
  process-systemd, process, network-local, d2bd forward_rendezvous.
- `HandoffCoordinator`/`HandoffState`/`HostGenerationHandoffIntent` (host_generation) —
  consumed by d2b-broker/src/runtime.rs + host_generation_handoff.rs + activation-nixos driver.
- `AuditJoinContext`/`CanonicalAuditDigest`/audit export cursor — read in d2b-broker audit,
  d2bd composition, broker_transport.
- `ForwardOperationRequest`/`Outcome::Result|Refused`, `EnvelopeInvoke` kernel frames,
  `PipeWireAudioRequest`, `QemuMedia*`, `Usbip*` — consumed across the provider family.
- `RunnerRole`/`BrokerProfile::as_str`, `PROTOCOL_VERSION` (wire-tagged; cross-checks in
  d2bd catalog + xtask gen_broker_operations).

Searched: workspace-wide `grep` for each pub item's name outside this crate (default source
roots, then `packages/`), checked `tests/wire.rs` + `wire()` test boundary surface, and used
xtask `catalog.rs` (BrokerProfile::w3 capabilities check) as the trusted admission reference.
No item surfaced with zero real callers. Correctness/wire-shape items are out of scope here.

## Consistency notes

- Types-layer crate (U4). `HelloRequest.supported_features` is read by daemon hello handling
  (`d2b-contracts/src/lib.rs:211-215`, d2bd composition), not dead `[INFERENCE-free; direct
  read sites]`. Wire tags (`kebab-case`/`camelCase`) stable across the contract family.
- `#C4 [not applied]` cross-crate macro/boilerplate consolidation (76 Wire deserialize
  blocks, identity macros, facade copies) is a workspace-family class owned by U97's
  consolidation feed; this crate's `ForwardOperationOutcome`, `BrokerCallerRole`, and
  `RunnerRole::as_str` are all type-carriers with production readers, so the canonical
  consolidation home remains `d2b-contracts` + broker-side (not this crate).
- `BrokerProfile::allows_request` duplicates the daemon's per-profile operation gate across
  the contracts family (host_operations/guest_operations catalog); the crate is the trusted
  catalog owner, so this is the canonical home, not a duplicate.

## Reopened refusals

None. `#C4` stays open as a cross-crate consistency lane (no new evidence).

## Checked

Read broker_wire.rs (full rustdoc/cargo surface), host_generation.rs, kernel_client.rs,
lib.rs, tests/wire.rs, generated broker_operation_profiles.rs, and the crate's BUILD.bazel +
Cargo.toml. Workspace-wide caller greps for every pub item; xtask provider_crate_policy +
crate-layout integration ratchet confirmed (integration/*.rs + README required, no README-only
offenses here). Ledger item #C4 honored ([not applied], no new evidence). No dead code
surfaced.
