# Amendment request: W5 destination and crate-rename drift

| Field | Value |
| --- | --- |
| Scope | Work-item **destination paths** for eleven W5 items plus two already-`Merged` items, across five crate renames and one policy-file family; widened in section 7 to a **normative value** conflict on the configuration-cleanup stall threshold |
| Raised under | FR-046, and the "existing code is canon" rule where FR-046 does not reach |
| Affected member specs | `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-zone-control`, `ADR-046-resources-volume`, `ADR-046-provider-state`, `ADR-046-security-and-threat-model` (section 7 only) |
| Affected manifests | `ADR-046-work-items.json`, `ADR-046-implementation-graph.json` |
| Status | Recorded and raised to the integrator; awaiting a separate specification amendment |
| Snapshot verified against | `a7f4a6a4` on `adr046-w5-audit-docs` |
| Runtime behaviour changed by this document | **None.** No code is edited; section 7.6 is a standing instruction not to edit it |

## 1. Why this is a separate file

[`amendment-w2-destination-drift.md`](./amendment-w2-destination-drift.md)
section 6 says any further **section 3.2 wave-table** drift should be appended
there rather than opening a second amendment, and draws the boundary
explicitly: the batching covers drifts in that one table of
`ADR-046-validation-and-delivery`, and does not extend to other member
specifications, which carry their own evidence.

This drift is on the other side of that boundary in both respects. It is not in
the section 3.2 wave table, it is not about wave assignment, and it touches four
different member specifications. It is about **destination paths**: the exact
files and crates a work item may write. Appending it to the W2 file would
silently widen a batch whose author scoped it to one table and one re-opening.

## 2. The deciding rule, because FR-046 does not settle this class

FR-046 resolves disagreements between the specification set's **prose** and the
**generated manifests** by making the manifests authoritative. That rule does
not apply here, and it is worth being precise about why: in every case below,
both sides of the disagreement are the generated manifest.

The clearest example. `ADR046-exec-016` and `ADR046-exec-017` name
`packages/d2b-bus-session/` and `packages/d2b-bus-session-unix/` as
destinations. `ADR046-session-001` and `ADR046-session-002`, both `Merged` in
W1, name `packages/d2b-session/` and `packages/d2b-session-unix/`. Both pairs
are rows in `ADR-046-work-items.json`. Two member specifications were authored
against different naming proposals and the generator faithfully carried both
forward. Consulting "the manifest" returns two answers.

**The rule that decides it is "existing code is canon."** Where a destination
names a path that does not exist and a committed, passing crate covers the same
obligation under a different name, the committed crate is the destination and
the manifest cell is drift. Two corollaries, and the second is the one that
prevents this document from becoming a completion claim:

- Where the member specification's own detailed-design text explicitly permits
  the current name, there is no drift at all; the destination cell is one of two
  sanctioned spellings and the code chose the other. Section 3.1 is that case,
  and it is decided by reading the spec rather than by overruling it.
- **A mapped destination is not a discharged obligation.** Mapping says where
  the work belongs, not that it is done. Every item below stays `Planned` unless
  the manifest already records otherwise, and section 5 states what would be
  needed to change that.

Per FR-046 none of these cells is corrected in place. Editing a member spec
re-opens its validation and panel evidence and re-triggers Gate 0 across the
whole manifest under FR-056. Four member specs are affected here, so the cost of
correcting in place is four re-openings for a set of path spellings.

## 3. The renames

### 3.1 Session crates - not drift; the specification permits the current name

| Item | Destination cell | On disk |
| --- | --- | --- |
| `ADR046-exec-016` | `packages/d2b-bus-session/src/`, `.../tests/` | `packages/d2b-session/` |
| `ADR046-exec-017` | `packages/d2b-bus-session-unix/src/`, `.../tests/` | `packages/d2b-session-unix/` |

The destination cells propose a rename. The **same rows'** detailed-design text
makes it optional, in as many words:

> `ADR046-exec-016`: "Preserved source-plan detail: copy verbatim; rename crate
> from `d2b-session` to `d2b-bus-session` **or retain name**."

> `ADR046-exec-017`: "Preserved source-plan detail: copy verbatim; rename crate
> from `d2b-session-unix` to `d2b-bus-session-unix` **or retain name**."

`ADR046-exec-017`'s Removal proof closes it: "If the crate is renamed, the
superseded `packages/d2b-session-unix/` owner is removed or reduced to a
compatibility wrapper ...; **if the name is retained, no prior owner is
removed**."

**Verdict: conformant, not drift.** The specification offered two spellings, the
destination cell records one, and the tree carries the other. `d2b-session` and
`d2b-session-unix` are the destinations for these two items and no rename is
owed. This is decided by reading the spec, not by overruling it, and it is the
one row in this document where the manifest needs no amendment at all - only the
destination cell's misleading singularity is worth a footnote if the integrator
amends the others.

`ADR046-exec-017`'s own `currentSource` already names
`packages/d2b-session-unix/src/` at baseline `b5ddbed6` as the partial
equivalent, so the manifest is internally aware that the retained-name crate is
the subject.

### 3.2 Bus wire and bus session modules - `packages/d2b-bus/`

| Item | Destination cell | On disk |
| --- | --- | --- |
| `ADR046-exec-018` | `packages/d2b-bus-wire/src/session.rs` | `packages/d2b-bus/src/wire.rs` and `packages/d2b-bus/src/session/` |

`packages/d2b-bus/` is owned by `ADR046-bus-001`, `Merged` in W1, whose
destination reads
`packages/d2b-bus/src/{router,registry,authorization,streams,operations}.rs`.
The crate also carries `wire.rs`, a `session/` directory
(`contract.rs`, `enrollment.rs`, `prologue.rs`, `noise_vectors.rs`,
`zone_link.rs`) and a `transport/` directory. There is no `d2b-bus-wire` crate
and no plan that produces one, because the surface it names already lives inside
the merged bus crate.

**Verdict: genuine equivalent.** `ADR046-exec-018`'s destination is
`packages/d2b-bus/src/wire.rs` and `packages/d2b-bus/src/session/`. The item is
still `Planned` and its own validation obligations - compile-time assertions on
every copied numeric constant, an `EndpointPolicyIdentity` golden vector under
v3 zone-name encoding, a `LimitProfile::local_default()` round trip - are not
claimed here.

**A named hazard.** `packages/d2b-bus/` is a critical subsystem: AGENTS.md lists
the zone message bus boundary and authoritative subject resolution, and
`ZoneRegistrar` exclusively owns subject resolution from registrar-private state.
Redirecting a `Planned` item's destination into that crate means its
implementer inherits those invariants. An implementer who reads only the
manifest cell would have created a fresh `d2b-bus-wire` crate and could have
introduced a second, unsealed copy of the session-wire constants beside the
sealed one. That is the concrete failure this mapping prevents, and it is why
the mapping is worth recording rather than leaving to be rediscovered.

### 3.3 Zone routing - `packages/d2b-zone-routing/`

| Item | Destination cell | On disk |
| --- | --- | --- |
| `ADR046-exec-023` | `packages/d2b-zone-router/src/{router,service,resolver}.rs` | `packages/d2b-zone-routing/src/{router,service,resolver}.rs` |

The module filenames match exactly; only the crate name differs by one suffix.
The crate is owned by `ADR-046-zone-routing`'s W2 items, several already
`Merged`: `ADR046-routing-002` (`engine.rs`), `ADR046-routing-003`
(`resolver.rs`, `ZoneEntrypointResolver`), `ADR046-routing-016` (`service.rs`),
`ADR046-routing-006` (tests and benches).

The symbols `ADR046-exec-023` names are present:
`router.rs` defines `ZoneOperationRouter`, `DedupKey`, `DedupDecision` and
`DurableExecTable`; `service.rs` defines `ZoneServiceLimits`,
`ZoneServiceServer`, `ZoneServiceMethod` and `ZoneServiceAuditEvent`;
`resolver.rs` defines `ZoneEntrypointResolver`.

**Verdict: genuine equivalent, with a residual.** The destination is
`packages/d2b-zone-routing/`. The item stays `Planned`, and the reason is
specific rather than procedural: `ADR046-exec-023` requires the five-tuple dedup
namespace to be `(zone, resource-type, resource-name, verb, idempotency-key)`
with a golden-vector test, tombstone expiry, `MAX_DISPATCH_IN_FLIGHT`
back-pressure, and principal-binding enforcement. The W2 items delivered the
crate; whether they delivered *those* obligations is a per-obligation check that
this adjudication does not perform and must not be read as having performed.
The manifest also spells the audit type `ZoneAuditEvent` where the code has
`ZoneServiceAuditEvent`; code is canon.

### 3.4 Resource client - `packages/d2b-resource-client/`

| Item | Destination cell | On disk |
| --- | --- | --- |
| `ADR046-client-001` | `packages/d2b-client/src/` | `packages/d2b-resource-client/src/` |
| `ADR046-exec-022` | `packages/d2b-bus-client/src/` | `packages/d2b-resource-client/src/` |

Two items in two different member specs name two different nonexistent crates
for the same surface. `packages/d2b-resource-client/` is that surface, and it
carries the exact renamed symbols `ADR046-exec-022`'s destination prescribes:
`DaemonClient` to `ZoneClient`, `HostSocketConnector` to `ZoneSocketConnector`,
and `LocalDaemonSession` to `LocalZoneSession` all appear in
`packages/d2b-resource-client/src/zone_client.rs`, alongside `ResourceWatch`,
`ZoneSessionConnector` and `ConnectedZoneClient`.

**Verdict: genuine equivalent, with one named absence.** The destination for
both items is `packages/d2b-resource-client/`. `ADR046-exec-022`'s fourth
rename, `GuestClient` to `ProcessAttachClient`, has **no** counterpart:

```
$ git grep -l 'ProcessAttachClient' -- packages
(no match)
```

That obligation is genuinely outstanding, not relocated, and both items stay
`Planned`.

### 3.5 Zone service and generated v3 stubs - `packages/d2b-resource-api/`

| Item | Destination cell | On disk |
| --- | --- | --- |
| `ADR046-exec-021` | `packages/d2b-bus-contracts/src/generated_v3_services/` | `packages/d2b-contracts/src/generated/d2b_resource_v3.rs`, `packages/d2b-resource-api/src/generated/d2b_resource_v3_ttrpc.rs` |
| `ADR046-exec-021` | `packages/d2b-zone-service/src/{admission,handler,routing}.rs` | `packages/d2b-resource-api/src/{zone_service,admission,service}.rs` |
| `ADR046-wire-001` | `packages/d2b-contracts/src/v3/state.rs` | `packages/d2b-contracts/src/v3/{volume_state,storage,limits}.rs` |

`packages/d2b-resource-api/src/zone_service.rs` opens with "Authenticated v3
Zone service dispatch seam ... the native Rust equivalent of the generated Zone
service boundary" and defines `ZoneMethod`, `ZoneCallContext`, `ZoneService` and
the `StrictWireMessage` trait - the four surfaces `ADR046-exec-021`'s detailed
design names. The generated ttrpc stubs it also names are emitted by
`xtask gen-resource-ttrpc` into `packages/d2b-resource-api/src/generated/`,
which is `ADR046-api-001`'s destination and is `Merged`.

`ADR046-wire-001`'s `v3/state.rs` is the one split rather than renamed
destination: `StateEnvelope` and the state constants are re-exported through
`packages/d2b-contracts/src/v3/mod.rs` from `volume_state.rs`, with bounds in
`limits.rs` and storage types in `storage.rs`. Its three sibling destinations
`v3/{services,identity,provider}.rs` all exist under their manifest names, so
this is a one-module divergence inside an otherwise-conformant row.

**Verdict: genuine equivalents.** Neither item's validation obligations are
claimed; both stay `Planned`. `ADR046-exec-021` in particular still owes the
deny-unknown-field decode assertions across every v3 message type and a
build-stable `service_schema_fingerprint`.

### 3.6 Provider runtime and provider agent - `packages/d2b-provider/`

| Item | Destination cell | On disk |
| --- | --- | --- |
| `ADR046-exec-019` | `packages/d2b-provider-runtime/src/{registry,rpc,instance,context,error}.rs` | `packages/d2b-provider/src/{registry,rpc,instance,context,error}.rs` |
| `ADR046-exec-020` | `packages/d2b-provider-agent/src/` | `packages/d2b-provider/src/agent.rs` |

All five module filenames `ADR046-exec-019` names exist in
`packages/d2b-provider/src/`, and the crate additionally carries `agent.rs`,
`descriptor.rs`, `forwarding.rs`, `identity.rs`, `installation.rs`, `session.rs`
and `share_adapter.rs`. `ADR046-provider-agent-001` at T158 independently names
`packages/d2b-provider/src/agent.rs` as its destination, so the manifest already
places the provider agent in that crate under a different item.

`ADR046-exec-019` also says "provider trait objects moved to `d2b-bus-wire` or
`d2b-provider-contracts`". Neither crate exists; the wire-safe publication shape
is at `packages/d2b-contracts/src/v3/provider_registry.rs`, whose own doc
comment states the division: "The runtime registry in `d2b-provider` owns
instances and in-flight permits. This module contains only the signed,
identity-safe publication shape that can cross the v3 Provider service
boundary."

**Verdict: genuine equivalents.** Destinations are `packages/d2b-provider/` and
`packages/d2b-contracts/src/v3/provider_registry.rs`. Both items stay `Planned`.

**A second named hazard, and the reason this row matters most.**
`packages/d2b-contract-tests/tests/policy_provider_crates.rs` pins
`ALLOWED_WORKSPACE_DEPS` for Provider crates to exactly six entries. Creating
`d2b-provider-runtime` or `d2b-provider-agent` as new `d2b-provider-*` crates
would put them under the provider-crate-layout policy, which requires `src/`,
`tests/`, `integration/` and `README.md`, and under the naming policy's
`d2b-provider-<base>-<implementation>` rule that neither name satisfies. An
implementer following the manifest cell literally would have created two crates
that the enforcing `test-policy` lane refuses, and the natural next move -
adding two more names to the exemption list that
`the_two_recorded_exemptions_are_exactly_the_naming_mismatches` pins at exactly
two - would have widened a deliberately closed allowlist to accommodate a stale
path spelling.

### 3.7 Provider layout policy - three locations, and the W3 record is now stale

`implementation-debt.md` section 10.3 records that `ADR046-pkg-001` named
`packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` while the W3
slice shipped `policy_provider_crates.rs`, and that the named file was routed
through the "advisory `test-fixture-contracts` lane". Both halves of that record
have since been overtaken by committed changes, so restating it unchanged would
propagate a stale claim.

Measured at `a7f4a6a4`:

| Location | Present | Lane | Owner |
| --- | --- | --- | --- |
| `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` | yes, 533 lines, landed at `2232c8c1` | `test-fixture-contracts`, which runs `cargo nextest run -p d2b-contract-tests` over the whole crate | `ADR046-pkg-001`, W5, `Planned` |
| `packages/d2b-contract-tests/tests/policy_provider_crates.rs` | yes | `test-policy` as a standalone binary, and the fixture lane | W3 slice, per section 10.3 |
| `packages/xtask/src/provider_crate_policy.rs` | yes | `test-policy`, as `cargo xtask check-provider-crate-layout` and `check-provider-layout` | `ADR046-pstate-011`, W4, `Merged` |
| `tests/unit/gates/provider-crate-layout-check.sh` | **no** | n/a | named by `ADR046-pstate-011`; ruled out by debt section 14.4 |

Two corrections to the section 10.3 record:

1. **The named destination file now exists.** `2232c8c1` added it, so the drift
   that section described - a policy shipped under a different name - is
   resolved for `ADR046-pkg-001`'s destination, though the item remains
   `Planned` on its validation obligations.
2. **`test-fixture-contracts` is no longer advisory.**
   `tests/layer1-jobs.json` classifies exactly one job advisory today,
   `test-performance-budgets`; `test-fixture-contracts` is enforcing. Section
   10.3's reasoning - that a hermetic check belongs in an enforcing lane rather
   than an advisory one - was sound and is unaffected, but the premise it rested
   on has changed and citing it now would understate the coverage.

**Verdict.** Three live locations, deliberately, with distinct roles:
`provider_crate_policy.rs` is the xtask entrypoint the item's own `integration`
field always described; `policy_provider_crates.rs` is the dependency-allowlist
and naming-exemption policy; `policy_provider_crate_layout.rs` is the
Cargo-metadata-driven layout policy that also scans `packages/` on disk so a
crate omitted from the workspace cannot escape coverage. The shell gate at
`tests/unit/gates/provider-crate-layout-check.sh` is **not** owed and must not
be created - the drift and meta gate set is closed, and debt section 14.4
already ruled on it.

## 4. The unmapped destinations

Each row records what the manifest names, what exists, and the verdict.
`mapped` means a committed equivalent covers the destination and the item's
implementer should write there. `absent` means no equivalent exists and the
obligation is genuinely outstanding.

| Task | Item | Destination not found | Verdict |
| --- | --- | --- | --- |
| T128 | `ADR046-exec-001` | `packages/d2b-contracts/src/v3/ephemeral_process.rs` | **mapped** - `EphemeralProcess` is modelled in `v3/process.rs`; the row's six other destinations (`host`, `guest`, `execution_policy`, `process`, `user`, `endpoint`) all exist under their manifest names |
| T129 | `ADR046-exec-002` | `packages/d2b-contracts/src/v3/process_provider.rs` | **mapped** - `packages/d2b-process-conformance/src/process_provider.rs`, whose doc comment calls itself "the destination-compatible boundary", re-exports all eight named types; `ExitClass` is spelled `ProcessExitClass` |
| T139 | `ADR046-exec-012` | `nixos-modules/zone-bundle.nix` | **mapped** - `nixos-modules/bundle-zones.nix` is the per-Zone v3 resource bundle emitter; `options-zones.nix` and `resource-schemas/` exist under their manifest names |
| T140 | `ADR046-exec-014` | `nixos-modules/zone-bundle.nix` | **mapped** - same; `v3/resource_bundle.rs` and `xtask/src/gen_resource_schemas.rs` exist under their manifest names |
| T144 | `ADR046-exec-019` | `packages/d2b-provider-runtime/` | **mapped** - `packages/d2b-provider/`; see 3.6 |
| T145 | `ADR046-exec-020` | `packages/d2b-provider-agent/` | **mapped** - `packages/d2b-provider/src/agent.rs`; see 3.6 |
| T146 | `ADR046-exec-021` | `packages/d2b-bus-contracts/`, `packages/d2b-zone-service/` | **mapped** - `packages/d2b-resource-api/`; see 3.5 |
| T147 | `ADR046-exec-022` | `packages/d2b-bus-client/` | **mapped**, except `ProcessAttachClient`, which is **absent**; see 3.4 |
| T153 | `ADR046-volume-004` | `nixos-modules/options-volumes.nix` | **absent** - no file declares user-facing volume or attachment options. `resources-volume.nix` exists and holds the eval-time Volume contract over the Zone resource attrset in `options-zones.nix`; the separate option module does not exist and is not covered by another file |
| T159 | `ADR046-wire-001` | `packages/d2b-contracts/src/v3/state.rs` | **mapped** - split across `v3/{volume_state,storage,limits}.rs` and re-exported from `v3/mod.rs`; see 3.5 |
| T166 | `ADR046-zone-control-007` | `nixos-modules/resources-zone-control.nix` | **absent as a file**, obligation partly covered - Zone-control resource handling is spread across `options-zones.nix`, `options-zones-resources.nix`, `resources-zones-processes.nix`, `resources-zones-volumes.nix` and `generated/`, and `index.nix` already computes `declaredZones` and `zoneRows`. No single module answers to the destination name, so the integrator must either bind the destination to that set or accept a new file |

`ADR046-volume-004` at T153 deserves the emphasis. It is the only row here where
the check might have looked satisfied and is not: `resources-volume.nix`, its
other destination, exists and is substantial, and a scan that stopped at "one of
two destinations is present" would have marked the row covered. The absent half
is a distinct obligation - user-facing volume and attachment options - and
`ADR046-vvfs-006` names the same absent file, so two `Planned` items depend on
it.

## 5. What this document does not do

**It marks nothing complete.** Every item named here that the manifest records
as `Planned` remains `Planned`. Presence of a file at a mapped path is evidence
about *where* the work belongs, and no evidence at all about whether the item's
`validation` column has been satisfied. Several of these items carry obligations
that a directory listing cannot speak to - golden vectors, deny-unknown-field
decode assertions, fingerprint stability across builds, tombstone expiry - and
none of them is asserted here.

**It changes no runtime behaviour.** No code is edited by this document. Section
7 records a conflict over a committed numeric default and explicitly instructs
W5 to leave that default, and the test that pins it, alone.

**It edits no manifest and no member specification.** The mappings are
instructions to implementers and to reviewers, standing until a dedicated
amendment carries the prose change with its own validation and panel evidence.

**It does not touch the amendments under panel.**
`amendment-spike-01-rerun.md` and `gate0-reevaluation-spike-01-rss-rerun.md` are
untouched; nothing here needed a reference into them.

## 6. Disposition

- Sections 3.2 through 3.7 and section 4 are **recorded and raised to the
  integrator**, to be carried by a separate specification amendment against the
  four affected member specs, scheduled outside any implementation wave.
- Section 3.1 needs no amendment: the specification already permits the retained
  crate names, so there is nothing to correct beyond the destination cell's
  choice of one of two sanctioned spellings.
- Until that amendment lands, this document is the standing instruction:
  implementers write to the mapped path, reviewers reject a candidate that
  creates a crate named in a stale destination cell, and the two absent rows
  (`ProcessAttachClient`, `nixos-modules/options-volumes.nix`) plus the partly
  covered `resources-zone-control.nix` remain outstanding obligations against
  their `Planned` items.
- Section 7's numeric conflict is raised on the same amendment, which acquires
  `ADR-046-security-and-threat-model` as a fifth affected member spec. Until it
  lands, `CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT` stays at 600,000 ms
  and `configuration_stall_clock_is_bounded_and_clock_injected` keeps its pinned
  ten-minute boundary; a W5 candidate that moves either is out of scope and
  should be rejected at review.

## 7. Scope widened: a normative numeric conflict in the same member specs

This document was opened for destination paths. A second drift of a different
kind was found in the same wave, in two of the same member specifications, and
is absorbed here on the precedent
[`amendment-w2-destination-drift.md`](./amendment-w2-destination-drift.md)
section 6 set: batch when the batching is genuinely free, and say where the
boundary now sits.

The batching is close to free here. This amendment already re-opens
`ADR-046-resources-host-guest-process-user` and `ADR-046-resources-zone-control`;
absorbing this conflict adds exactly one further member spec,
`ADR-046-security-and-threat-model`. Filing it separately would pay three
re-openings where batching pays one.

**The boundary.** Section 7 covers a **normative value** conflict - the same
named default given two different numbers by different members of the set - as
distinct from sections 3 and 4, which cover destination paths. A third class,
such as a frozen-contract amendment, still files separately.

### 7.1 The conflict, with all five sources quoted

The configuration-cleanup stall threshold has two different published defaults.

| Source | Kind | Value |
| --- | --- | --- |
| `packages/d2b-core-controller/src/cleanup.rs:257` | committed, passing code | `600_000` ms (10 min) |
| `ADR-046-resources-host-guest-process-user.md:2604`, carried into `ADR046-exec-015`'s manifest `detailedDesign` | normative prose plus generated manifest | 10 min |
| `ADR-046-resources-zone-control.md:247`, `:3024`, `:4046` | normative prose, three separate places | 5 minutes |
| `ADR-046-resources-zone-control.md:4939`, carried into `ADR046-zone-control-016`'s manifest `detailedDesign` | normative prose plus generated manifest | 5 min |
| `ADR-046-security-and-threat-model.md:1502` | normative prose | 5 minutes |

Verbatim, so no side is paraphrased into agreement:

```
$ sed -n '257p' packages/d2b-core-controller/src/cleanup.rs
pub const CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT: u64 = 600_000;
```

> `ADR046-exec-015` detailed design: "Cleanup-stuck threshold: **10 min
> default**; configurable; stuck resources remain Degraded without blocking
> later activations."

> `ADR-046-resources-zone-control` section 2.5: "`GenerationCleanupFailed=True`
> is additionally set when a candidate is stuck beyond `cleanupStuckThreshold`
> (**default 5 minutes**) with no controller progress."

> `ADR046-zone-control-016` detailed design: "stuck-cleanup
> `GenerationCleanupFailed=True` at `cleanupStuckThreshold` (**default 5 min**)
> with exponential backoff retry".

> `ADR-046-security-and-threat-model`: "if a cleanup candidate exceeds
> `cleanupStuckThreshold` (**default 5 minutes**), a `GenerationCleanupFailed`
> condition is set - the runtime never force-removes finalizers to clear it".

Both `ADR046-exec-015` and `ADR046-zone-control-016` are **W5** and **Planned**,
and both name the `d2b-core-controller` configuration and cleanup surfaces as
destinations. The wave that must reconcile this is the wave now in flight.

### 7.2 FR-046 does not decide this either, for the same reason as section 2

The two published values are not prose against a manifest. `ADR046-exec-015`
carries "10 min default" and `ADR046-zone-control-016` carries "default 5 min",
both inside `detailedDesign` in `ADR-046-work-items.json`:

```
$ jq -r '.items[] | select(.workItemId=="ADR046-exec-015") | .detailedDesign' \
    docs/specs/ADR-046-work-items.json | grep -o 'Cleanup-stuck threshold: [^;]*;'
Cleanup-stuck threshold: 10 min default;

$ jq -r '.items[] | select(.workItemId=="ADR046-zone-control-016") | .detailedDesign' \
    docs/specs/ADR-046-work-items.json | grep -o 'cleanupStuckThreshold` (default [^)]*)'
cleanupStuckThreshold` (default 5 min)
```

Consulting "the manifest" returns both answers, exactly as in section 2. The
generator is not at fault; it faithfully carried two member specs that were
authored against different numbers.

### 7.3 Ruling: code is canon for this wave; 600,000 ms stands

**`CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT` is not changed, and this
record changes no runtime behaviour.** The committed constant is 600,000 ms, it
is covered by a passing test, and existing passing code is canon.

That the code happens to match one of the two normative values is a coincidence
worth naming rather than leaning on. Code wins here because it is committed and
passing, not because `ADR-046-resources-host-guest-process-user` outranks
`ADR-046-resources-zone-control`. Nothing in the specification set gives one
member standing over another on a shared default, which is precisely why this
needs an amendment rather than a reading.

### 7.4 What the constant actually reaches today, measured

This is the fact that makes the conflict cheap now and expensive later, and it
is not visible from the specifications:

```
$ git grep -n 'CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT' -- packages
packages/d2b-core-controller/src/cleanup.rs:257:pub const CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT: u64 = 600_000;
packages/d2b-core-controller/src/cleanup.rs:1095:                CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT,
packages/d2b-core-controller/src/cleanup.rs:1103:                CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT,

$ git grep -l 'cleanupStuckThreshold' -- . | grep -v '^docs/specs/'
(no match)
```

Both call sites are inside one `#[cfg(test)]` unit test,
`configuration_stall_clock_is_bounded_and_clock_injected`, which pins the
boundary at exactly ten minutes: `00:09:59.999` is not stalled and
`00:10:00.000` is. There is **no production caller**. `cleanup_stall_due` takes
the threshold as a parameter and is clock-injected and side-effect free, and the
controller's own stall path is caller-driven through `mark_cleanup_stalled`
rather than derived from this constant. `cleanupStuckThreshold` exists nowhere
outside `docs/specs/`: no Nix option, no schema field, no CLI surface, no
reference page.

Three consequences follow, and they should be read together:

1. **No operator is affected today**, in either direction. Neither value is
   reachable from a running system, so neither the record nor a future
   correction is currently a behaviour change.
2. **The constant is a default, not a policy.** Because the threshold is a
   parameter, the eventual disagreement is about what default the Zone
   configuration surface publishes, not about what the predicate computes. An
   amendment that only edits a number has answered the smaller question.
3. **The window closes when the surface is wired.** Once `cleanupStuckThreshold`
   becomes an operator-visible option with a published default, changing it is a
   consumer-facing default change under the deprecation policy rather than a
   specification correction. It is cheap in W5 and expensive after.

### 7.5 The specific failure this record exists to prevent

`ADR046-zone-control-016`'s destination is:

> `packages/d2b-core-controller/src/configuration/{mod,bundle_apply,generation_transition}.rs`
> (Phase 3 activation, diff, delete dispatch);
> **`packages/d2b-core-controller/src/cleanup.rs`** (pending tracking, status,
> stuck detection, rollback verb handler)

It names the exact file holding the 600,000 ms constant, and its detailed design
says the default is 5 min. A W5 implementer working that item literally will
edit `600_000` to `300_000`, and
`configuration_stall_clock_is_bounded_and_clock_injected` will fail, because it
asserts the transition at `00:10:00.000` rather than tolerating a range.

**The failing test is the guard, and it must not be retuned to accommodate the
edit.** The tempting repair - move the test's timestamps from ten minutes to
five and carry on - halves the stall window that reaches every future operator,
inside a slice whose stated scope was cleanup wiring, with a green gate and no
record. That is the concrete, specific way this drift ships silently, and it is
the reason this section names the file rather than describing the conflict in
the abstract.

The test is not incidental coverage. It is the only artifact in the tree that
asserts what the default is.

### 7.6 Standing instruction for W5

Fail closed on the constant:

- **Do not change `CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT` in this
  wave.** A W5 candidate snapshot that alters it is out of scope and should be
  rejected at review, in the same way section 2's corollary rejects a candidate
  that creates a crate named in a stale destination cell.
- **Do not retune
  `configuration_stall_clock_is_bounded_and_clock_injected`.** If a slice finds
  itself editing that test's timestamps, the slice has changed a default, and
  that is an amendment, not an implementation choice, per FR-047.
- **A slice implementing `ADR046-zone-control-016` writes the rest of its
  destination** - pending tracking, status, stuck detection, the rollback verb
  handler - and passes the threshold as the parameter it already is, leaving the
  default where it stands.
- **Neither wiring the option nor publishing a default is authorised here**,
  because doing so would pick a winner between two member specs by
  implementation.

### 7.7 Recommendation to the amendment, explicitly not applied

The amendment must choose one value; it cannot ship both. A recommendation is
recorded so the decision starts from a position rather than from a fresh
argument, and it is a recommendation only.

**Recommend 5 minutes, and change the code to match as a deliberate, separate
change.** Two reasons, neither of them a document headcount:

- **The failure is asymmetric in the safe direction.** Crossing the threshold
  sets `GenerationCleanupFailed=True` and holds the Zone `Degraded`. The
  specifications are explicit that this reports rather than refuses: stuck
  resources "remain Degraded without blocking later activations", and "the
  runtime never force-removes finalizers to clear it". A false positive
  therefore costs an operator one investigation of a cleanup that was merely
  slow; a false negative hides a genuinely stuck finalizer for twice as long.
  Where a threshold governs surfacing rather than denying, the shorter window is
  the fail-closed choice.
- **The security and threat model is one of the three sources saying 5
  minutes**, and it states the value while explaining why the runtime never
  force-clears finalizers. That is the document whose numbers should not be
  quietly relaxed by a controller default drifting the other way.

Against that: 10 min is what is committed and tested, and the amendment may
reasonably prefer the value the code already carries. Either choice is
defensible; what is not defensible is leaving both published.

**If the amendment selects 5 minutes, one change must land all of:** the
constant at `cleanup.rs:257`, the pinned boundary in
`configuration_stall_clock_is_bounded_and_clock_injected`, and a changelog entry
naming the halved default - because the moment the option is wired, this is a
consumer-visible default change, and FR-042's rule that nothing changes silently
applies to a default just as it does to a capability.

**If the amendment selects 10 minutes**, three normative statements in
`ADR-046-resources-zone-control` plus one in
`ADR-046-security-and-threat-model` change, and `ADR046-zone-control-016`'s
`detailedDesign` changes with them. No code changes.

### 7.8 One more destination drift found in passing

`ADR046-exec-015`'s destination names
`packages/d2b-core-controller/src/configuration.rs`. That path is a **directory
module** at `packages/d2b-core-controller/src/configuration/`, per the ruling
already recorded in
[`implementation-debt.md`](./implementation-debt.md) section 14.3.
`ADR046-zone-control-016`'s destination already spells it as
`configuration/{mod,bundle_apply,generation_transition}.rs`, so the two items
disagree on the module shape as well as on the number. Same resolution as
section 3: existing code is canon, the destination is the directory module, and
the cell is drift. Recorded here rather than in section 4 because it was found
through this conflict and shares its amendment.

## 8. Four rows from the final audit

Appended on the same reasoning as section 4: these are work-item destination
paths that do not resolve, checked against the tree at `f70046fc`. Two map to
committed equivalents. **Two do not, and are recorded as active implementation
gaps rather than adjudicated as mapped**, because in both cases something with
a matching name exists and would pass a presence-only scan.

No implementation state is changed by this section. Every one of the four items
is `Planned` in `ADR-046-work-items.json` and stays `Planned`.

| Task item | Destination not found | Verdict |
| --- | --- | --- |
| `ADR046-device-008` | `packages/d2b-contract-tests/tests/workspace_policy.rs` | **mapped** |
| `ADR046-zone-control-014` | `nixos-modules/resource-type-validators.nix` | **mapped** |
| `ADR046-zone-control-015` | `packages/d2b-resource-compiler/src/{bundle,schema,validator,digest,sort,secret_lint,generation}.rs`; `nixos-modules/resource-compiler.nix` | **active gap** |
| `ADR046-telem-003` | `packages/d2b-bus/src/metrics.rs` | **active gap** |

### 8.1 `ADR046-device-008` - mapped

Destination: `packages/xtask/src/main.rs` (`check-provider-layout` subcommand)
and `packages/d2b-contract-tests/tests/workspace_policy.rs` (provider-layout
policy assertions).

The first half resolves exactly. `packages/xtask/src/main.rs` dispatches both
subcommands:

```
415:        [command] if command == "check-provider-crate-layout" => run_provider_crate_layout(),
416:        [command] if command == "check-provider-layout" => run_provider_layout(),
```

`workspace_policy.rs` does not exist. The provider-layout policy assertions it
names live in `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs`
and `policy_provider_crates.rs`, the same file family already adjudicated in
section 3.7. The destination is those files; the item stays `Planned` on its
own validation obligations.

### 8.2 `ADR046-zone-control-014` - mapped

Destination: `nixos-modules/options-zones.nix`,
`nixos-modules/generated/resource-types.nix`,
`nixos-modules/generated/options-zones-<ResourceType>.nix`, and
`nixos-modules/resource-type-validators.nix`.

Three of four resolve under their manifest names. `options-zones.nix` and
`generated/resource-types.nix` are present, and `generated/` carries per-type
option modules matching the pattern.

`resource-type-validators.nix` does not exist. Per-type validation is committed
under two other names: `nixos-modules/resource-schema-validation.nix`
("build-time validation of every emitted standard ResourceSpec") for the
schema-driven half, and the per-type compiler seams such as
`resources-zone-control.nix` and `resources-volume.nix` for the closed
type-specific constraints. The destination is that pair.

**One coverage observation that is not a destination question.** The
`options-zones-<ResourceType>.nix` pattern currently resolves to two files,
`options-zones-Zone.nix` and `options-zones-ZoneLink.nix`, against a
nineteen-member standard ResourceType set. That is generator coverage, not path
drift, and it is exactly the distinction section 5 draws: mapping a destination
says where the work belongs and never that it is complete.

### 8.3 `ADR046-zone-control-015` - active gap, and the crate name is the trap

Destination:
`packages/d2b-resource-compiler/src/{main,bundle,schema,validator,digest,sort,secret_lint,generation}.rs`,
exposed as `pkgs.d2b-resource-compiler`, called from
`nixos-modules/resource-compiler.nix`.

A crate named `d2b-resource-compiler` exists, is substantial, and is exposed as
a package. That is the whole trap:

```
$ ls packages/d2b-resource-compiler/src/
lib.rs  linux.rs  main.rs

$ ls nixos-modules/resource-compiler.nix
ls: cannot access 'nixos-modules/resource-compiler.nix': No such file or directory
```

Seven of the eight named modules are absent, and the Nix seam that would call
the compiler is absent. More decisively, the committed crate has a **different
subject**. Its own documentation opens: "Build-time validation for one selected
Provider artifact output," and its entry point is `compile_artifact`. The work
item's compiler is the Zone resource bundle compiler - bundle assembly, schema
validation, digesting, canonical sorting, secret linting, and generation
identity.

So the crate name matches, the package output matches, and the obligation does
not. A scan keyed on `pkgs.d2b-resource-compiler` or on the crate directory
would report this row satisfied. **It is an active implementation gap**, and it
is recorded here rather than mapped so that the name collision is on the record
before someone resolves it the other way.

### 8.4 `ADR046-telem-003` - active gap

Destination: `packages/d2b-resource-api/src/metrics.rs`,
`packages/d2b-session/src/metrics.rs`, `packages/d2b-bus/src/metrics.rs`.

Two of three exist under their exact manifest names. The third does not, and
`d2b-bus` has no metrics surface under another name either - `lib.rs` declares
seventeen modules and none of them is `metrics`. The matches on "metric" inside
`audit.rs`, `routing.rs`, and `session/` are incidental prose, not a relocated
module.

This is the same shape as the `options-volumes.nix` row in section 4 and is
worth the same emphasis: **a presence-only scan on the first two destinations
would mark the row covered.** The bus half of the metric surface is genuinely
outstanding, and the item stays `Planned`.

### 8.5 What section 8 does not do

It does not flip any implementation state, and it does not claim any of the
four items is complete. Two destinations are mapped so an implementer writes to
the committed path instead of creating a duplicate; two are named as gaps so a
later reader does not infer coverage from a matching crate name or from two
destinations out of three. All four remain `Planned`, and their `validation`
columns are untouched and unassessed here.
