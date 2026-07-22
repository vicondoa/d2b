# Verify provider parity

**Diataxis category:** how-to.

`packages/d2b-realm-provider` declares 18 typed provider trait families
(`HostSubstrateProvider`, `RuntimeProvider`, `WorkloadProvider`,
`DurableExecutionProvider`, `GuestControlEndpointProvider`,
`PersistentShellProvider`, `DisplayProvider`, `TransportListener`,
`TransportProvider`, `StreamMux`, `ProtocolCodec`, `DaemonAccessTransport`,
`DaemonAccessApi`, `InfrastructureProvider`, `CredentialProvider`,
`ObservabilitySinkProvider`, `RelayProvider`, `NodeProvider`). The crate's
`mock::parity` module (`packages/d2b-realm-provider/src/parity.rs`, wired in
from `packages/d2b-realm-provider/src/mock.rs`) is the typed inventory and
proof that every one of those trait families' operations is classified for
capability gating, idempotency, retry safety, cancellation, and
attachment/stream behavior — and that the crate's in-memory stand-ins behave
the way that classification says they must.

This how-to explains how to run that proof, how to read a failure, and
exactly what it does and does not establish.

## Run the proof

From `packages/`:

```bash
cargo test -p d2b-realm-provider mock::parity
cargo clippy -p d2b-realm-provider --all-targets -- -D warnings
cargo fmt -p d2b-realm-provider -- --check
```

`cargo test -p d2b-realm-provider` (the whole crate) also runs the proof; the
narrower `mock::parity` filter is faster while iterating on this module
specifically. The proof takes no network access and dials no real host,
relay, or provider API — every check runs against in-memory stand-ins
already defined in `mock.rs` (`MockWorkloadProvider`, `HeadlessDisplayProvider`,
`NoGuestControlEndpointProvider`, `HeadlessPersistentShellProvider`,
`StrictPersistentShellProvider`, `LoopbackStreamMux`, and the 13 additional
stand-ins added for the previously-unmocked trait families:
`MockHostSubstrateProvider`, `MockRuntimeProvider`,
`MockDurableExecutionProvider`, `NoAcceptTransportListener`,
`LoopbackTransportProvider`, `NoOpProtocolCodec`,
`LocalUnixDaemonAccessTransport`, `UnimplementedDaemonAccessTransport`,
`MockDaemonAccessApi`, `MockInfrastructureProvider`, `FixedCredentialProvider`,
`FixedObservabilitySinkProvider`, `MockRelayProvider`, `MockNodeProvider`).

## What the proof actually checks

`mock::parity::build_inventory()` fails closed (returns a
[`ParityViolation`]) the moment any of the following is not true:

- Every one of the 37 [`CanonicalOperation`] variants — one per non-identity
  ("verb") method across the 18 trait families — appears in `ALL_OPERATIONS`
  exactly once (no missing, no duplicate).
- Every one of the 18 [`ProviderFamily`] variants owns at least one
  classified canonical operation.
- Every one of the 22 [`d2b_realm_core::Capability`] variants and every one
  of the 19 [`d2b_realm_core::ErrorKind`] variants appears in this crate's
  `ALL_CAPABILITIES`/`ALL_ERROR_KINDS` tables exactly once.
- Every canonical operation classified `CapabilityGate::SelfAdvertised(cap)`
  is listed back by `capability_role(cap)`, and vice versa (a two-way
  cross-check between the operation table and the capability table).

`mock::parity::verify_provider_parity()` then runs two probe suites against
the stand-ins above and cross-checks their **observed** behavior against the
**declared** classification:

- `run_capability_denial_probes()` calls every capability-gated operation
  against a stand-in that advertises nothing (or advertises a different
  capability), and asserts each one fails closed with a typed
  `ErrorKind::CapabilityDenied` naming exactly the capability the
  operation's classification declares — never a generic error, never a
  silent alternate path.
- `run_stand_in_smoke_probes()` exercises every remaining stand-in
  (including all 13 newly-added ones) and asserts none of them ever
  construct a router-layer-owned `ErrorKind` (`NoRealmEntrypoint`,
  `GatewayUnavailable`, `AuthenticationFailed`, `VersionSkew`,
  `AuditUnavailable`; see `error_kind_owner`) — a provider stand-in must
  never impersonate the layer above it.

Both probe suites, `build_inventory`, and every value returned along the way
carry only bounded, low-cardinality data: family/operation/capability/error
codes and booleans/counts — never a raw provider message, endpoint, path, or
credential. See [`ProbeOutcome`] and [`ParityViolation`].

## Compile-time exhaustiveness, and its one honest gap

[`Capability`] and [`ErrorKind`] are **not** `#[non_exhaustive]` upstream in
`d2b-realm-core`, so `capability_role()` and `error_kind_owner()` are
genuinely exhaustive `match` statements across the crate boundary: adding a
new variant to either enum is a compile error in this crate until someone
classifies it. The same is true of `CanonicalOperation::classification()`,
since that enum is declared in this crate.

[`StreamKind`] **is** `#[non_exhaustive]` upstream. `stream_kind_usage()`
therefore cannot be a compile-time-exhaustive match; it can only assert, at
test time, that every kind in the hand-maintained `KNOWN_STREAM_KINDS` table
(13 entries, matching every `StreamKind` variant that exists in the
`d2b-realm-core` version this crate currently depends on) is explicitly
classified rather than falling through to the `UnknownToThisCrateVersion`
wildcard arm. A new `StreamKind` variant added upstream will **not** break
this crate's build; it will silently fall through to that wildcard arm until
a human adds it to `KNOWN_STREAM_KINDS` and gives it an explicit
classification. Treat any `d2b-realm-core` dependency bump that touches
`StreamKind` as a required trigger to re-run `cargo test -p d2b-realm-provider
mock::parity` and manually diff `KNOWN_STREAM_KINDS` against the upstream enum.

`ALL_OPERATIONS`, `ALL_FAMILIES`, `ALL_CAPABILITIES`, and `ALL_ERROR_KINDS`
are hand-maintained `const` arrays, not derived by reflection or a macro over
`provider.rs`'s trait declarations (Rust has no such reflection). The
self-consistency tests prove the tables are internally coherent — no
duplicate, no gap versus each enum's own variant list — but they cannot by
themselves prove a *new* trait method added to `provider.rs` was also added
to `CanonicalOperation`. Anyone adding, removing, or resignaturing a trait
method in `provider.rs` must manually add or update the matching
`CanonicalOperation` variant, `family()`/`method_name()`/`ordinal()`/
`classification()` arm, and any probe that exercises it; the compiler will
only catch a *missing arm on an existing variant*, not a missing variant.

## Reading a failure

- `ParityViolation::DuplicateOrMissingOperation` /
  `DuplicateOrMissingFamily` / `DuplicateOrMissingCapability` /
  `DuplicateOrMissingErrorKind`: one of the hand-maintained `const` arrays
  drifted from its enum. Fix the array, not the enum.
- `ParityViolation::UnclassifiedFamily(family)`: a trait family has zero
  canonical operations recorded against it — likely a new trait added to
  `provider.rs` with no matching `CanonicalOperation` variants yet.
- `ParityViolation::CapabilityGateCrossCheckFailed(op)`: an operation's
  declared gate and `capability_role`'s reverse listing disagree. Fix
  whichever side is stale.
- `ParityViolation::ProbeMismatch(op)`: a stand-in's **observed** behavior
  disagreed with `op`'s **declared** classification (for example, a
  capability-denial probe unexpectedly succeeded, or returned the wrong
  `ErrorKind`/missing capability, or a smoke probe surfaced a router-owned
  `ErrorKind`). Fix the stand-in in `mock.rs` or the classification in
  `parity.rs`, whichever is actually wrong.

## Scope: what this proof does not establish

- It never dials a real host, relay, or external provider — it only proves
  the *trait and stand-in* shape holds together. It says nothing about
  whether any real (non-mock) implementation of these traits, once written,
  will honor the same classification.
- It does not inspect or exercise the daemon/router call sites that are
  supposed to route every capability-gated operation through these traits.
  A caller that bypasses the trait boundary entirely (for example, invoking
  a legacy shell fallback instead of `PersistentShellProvider::attach_shell`)
  is invisible to this proof, because the proof only ever calls the trait
  methods directly.
- It does not cover capabilities that currently have no canonical operation
  in this crate at all (`Vsock`, `Virtiofs`, `DisplayStreaming`, `GpuAccel`,
  `Snapshots`, `Hotplug`, `EphemeralSessions`, `ProviderManagedIsolation`,
  `ConfiguredLaunch`): `capability_role()` records these explicitly with an
  empty operation list rather than omitting them, but an empty list is not a
  proof of anything about how those capabilities are gated elsewhere.

## Exact fallback-removal prerequisites

`FallbackPolicy` is deliberately a single-variant enum
(`FallbackPolicy::NeverFallback`): the type system itself makes it
impossible for `CanonicalOperation::classification()` to record an
operation as "falls back to an alternate transport or provider." That is a
proof about the classification table in *this* crate, not a proof that
every real call site already honors it. Removing any remaining fallback
path elsewhere in the system (legacy CLI, SSH, an undocumented alternate
provider, a bypass of a capability-gated trait method) requires all of the
following to hold, in addition to (not instead of) this proof passing:

1. **Every capability-gated operation this proof covers must have a real,
   non-mock provider implementation that this same proof's *shape* has been
   re-run against**, i.e. the future real implementation must fail closed
   with the exact typed `CapabilityDenied`/`UnsupportedFeature` this proof
   requires of the stand-in, not a generic error or a silent alternate path.
   This proof only establishes that shape for the 18 families' stand-ins
   today; it is a template for verifying the real implementations, not a
   substitute for doing so.
2. **The daemon/router code calling into these providers must be audited
   (outside this crate) to prove every capability-gated call site actually
   goes through the trait method this proof classifies**, rather than
   around it. This proof cannot see caller wiring; it can only prove the
   callee side is fail-closed if reached.
3. **`KNOWN_STREAM_KINDS` in `parity.rs` must be re-verified against the
   exact `d2b-realm-core::StreamKind` version in use** before removing any
   fallback tied to stream-kind-gated operations, because `StreamKind` is
   `#[non_exhaustive]` and a version bump will not fail this crate's build
   on its own (see "Compile-time exhaustiveness, and its one honest gap"
   above).
4. **`ALL_OPERATIONS`/`ALL_FAMILIES` must be confirmed in sync with
   `provider.rs`'s actual trait method list** at the specific commit being
   cut over, since that sync is manual (no reflection). A stale operation
   table can pass every test in this module while missing a newly-added
   trait method entirely.
5. **The empty-operation capabilities (`Vsock`, `Virtiofs`,
   `DisplayStreaming`, `GpuAccel`, `Snapshots`, `Hotplug`,
   `EphemeralSessions`, `ProviderManagedIsolation`, `ConfiguredLaunch`)
   must either gain a canonical operation and classification here, or have
   their gating proven by an equivalent typed proof in whichever component
   does gate them**, before any fallback tied to those capabilities is
   removed — this crate currently records them as ungated by construction,
   not as verified-safe.

Until all five hold for the specific fallback path in question, treat this
proof as necessary evidence toward removing that fallback, not sufficient
evidence on its own.

[`ParityViolation`]: ../../packages/d2b-realm-provider/src/parity.rs
[`CanonicalOperation`]: ../../packages/d2b-realm-provider/src/parity.rs
[`ProviderFamily`]: ../../packages/d2b-realm-provider/src/parity.rs
[`Capability`]: ../../packages/d2b-realm-core/src/capability.rs
[`ErrorKind`]: ../../packages/d2b-realm-core/src/error.rs
[`StreamKind`]: ../../packages/d2b-realm-core/src/stream.rs
[`ProbeOutcome`]: ../../packages/d2b-realm-provider/src/parity.rs
