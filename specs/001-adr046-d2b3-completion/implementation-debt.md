# Implementation Debt Register

**Feature**: `001-adr046-d2b3-completion` | **Opened**: 2026-07-30

Durable record of debt accumulated while implementing waves: stubs, unlanded
dependencies, semantics inferred where a specification was silent, code that
exists but is unreachable, and gate coverage that is not yet enforcing.

## What belongs here, and why it is separate

This register is **not** [`deferred-findings.md`](./deferred-findings.md). That
one records panel findings deferred under the bounded-deferral rule, and only
after a wave's eighth round. This one records debt created by *building*
against a specification set that is not uniformly complete, which is a
different thing with a different owner and a different closing condition.

Every entry names the wave that must close it. An entry with no owning wave is
an integrator decision, not a scheduling one, and is marked as such.

The rule that produced most of this register is worth stating, because it is
what made the debt visible rather than silent: an implementer who cannot
determine a semantic from the specification **reports the gap instead of
guessing**. A guess is indistinguishable from a decision once it is committed,
and a guess that lands in a contract propagates into every slice that consumes
it. Several entries below exist because an implementer stopped rather than
invent a wire contract, and that was the correct outcome each time.

---

## 0. Wave 2 delivery claim, stated precisely

Wave 2 is **17 of 19 items complete, 2 partial**. Saying "all nineteen
implemented" overstates it, and the difference matters to a reviewer deciding
what to hold to a completion standard.

- `ADR046-routing-014` is partial. The Provider registry, admission, lifecycle
  and forwarding are delivered; the eleven ProviderInstance trait objects and
  the whole RPC proxy family are not.
- `ADR046-routing-015` is partial. The conformance kit, bootstrap admission,
  audit ring, dispatch limiter and redaction are delivered; the generated
  service dispatch, the agent adapter, one-shot registration and the agent
  process are not.

Both are blocked on the same missing thing: a v3 Provider-method DTO catalogue
that exists in no crate. That catalogue is now owned: **Wave 3, inside
`ADR046-provider-001`**, whose destination already names
`packages/d2b-contracts/src/v3/provider.rs`, `packages/d2b-provider/src/lib.rs`
and `packages/d2b-provider-toolkit/` - exactly where the catalogue belongs. It
is scoped into that existing item rather than raised as a fifth work item,
because a fifth item would contradict `ADR-046-implementation-graph.json`,
whose `.waves[]` entry for W3 pins `workItemCount: 4`.

**That ruling is confirmed as to ownership and corrected as to expectation.**
It was made on the expectation that Wave 3 could deliver the catalogue.
Wave 3 delivered part of it and stopped, correctly, at the point where
continuing would have meant inventing a wire contract. The ownership is still
right and the item is still the right home; the remainder is not scheduling
debt but a **specification hole**, and it cannot be discharged by any wave
until that hole is filled. What is delivered and what is not is stated exactly
in section 10.1; discharging the remainder still closes the Destination
caveats recorded against `ADR046-routing-014` and `ADR046-routing-015` below,
and nothing else in this register does.

The Evidence rows for those two items in `docs/specs/ADR-046-zone-routing.md`
are deliberately left alone: they record what Wave 2 actually delivered and are
already panel-attested. This register is the forward-looking record.

## 1. Blocked and unlanded dependencies

These prevent a work item from being completed at all, or force it to ship a
hole. Each must be closed by the wave named.

| Item | Debt | Owning wave |
| --- | --- | --- |
| `ADR046-routing-007` | CLOSED. The dependency declarations landed as integrator prep and the contract module landed first, on the reading that the recorded edge is inverted for the contract. | Closed in W2 |
| `ADR046-routing-009` | Both landed, 009 first. The recorded edge remains wrong in the graph: it says 009 depends on 007, but 007 imports 009's contract. The manifest should be corrected as a separate amendment under the drift rule. | Amendment, not blocking |
| `ADR046-routing-016` service | Still no handler for `zone-bootstrap` and `zone-enroll`, but the blocker moved: the session contract and enrollment machine now exist in the bus, so wiring the service to them is ordinary work rather than a missing contract. The four enrollment obligations are met in the bus session module, not in the service. **Not W3**: the destination is `packages/d2b-zone-routing/src/service.rs`, which no W3 work item owns. Every work item whose destination names `packages/d2b-zone-routing/` - `routing-002`, `routing-003`, `routing-006`, `routing-016` - is W2, so the artifacts name no later owning wave. **Ruled: W5, alongside `ADR046-store-004`.** See the ruling note below the table. | W5, `ADR046-store-004` |
| `ADR046-primitives-002` providers | `ProcessLaunchEffectPort` has no production adapter, so both process Providers are complete but unwired. The adapter is `ADR046-process-001`, destination `packages/d2b-provider-supervisor/`. | W4 |
| `ADR046-routing-014` | `ProviderInstance`'s eleven trait objects and the whole `RpcProviderProxy` family are not delivered. Wave 3 delivered the method catalogue only for the methods the specification actually names, so the registry stays generic over the runtime's own opaque instance handle. The remainder is blocked on absent specification content, not on scheduling; see section 10.1. | W3 ownership stands, but blocked on a specification hole |
| `ADR046-routing-015` | `GeneratedProviderServiceServer` ttrpc dispatch not implemented: no v3 Provider proto, no service-name freeze, no generated bindings exist, and none can be written without a frozen service name and field numbering. `ProviderAgentAdapter`, `register_exact_instances`, and `ProviderAgentProcess` all depend on routing-014 surfaces that are themselves incomplete. | W3 ownership stands, but blocked on a specification hole |

### Ruling: the bootstrap and enroll handler is W5 work

The `ADR046-routing-016` row above had no natural owner, because every work
item naming `packages/d2b-zone-routing/` sits in the sealed W2. It is assigned
to **W5, alongside `ADR046-store-004`**, on a dependency rather than on a
file-ownership argument.

Two sibling rows in the wave-close table below - "Sealed enrollment record does
not bind the child uid" and "No durable persistence for enrollment" - were
already resolved to W5 `ADR046-store-004`, whose destination is
`packages/d2b-resource-store-redb/src/{lib,actor,transaction}.rs`. A
`zone-enroll` handler cannot be completed without the durable enrollment
persistence those two rows describe, so scheduling the handler in any earlier
wave would schedule work that cannot finish.

That dependency was verified rather than assumed.
`packages/d2b-bus/src/session/enrollment.rs` states in its own module
documentation that the durable store transaction which seals or invalidates an
enrollment is not part of the module, that recovery re-derives a restarting
handler's state "from what the durable store holds" and "takes the persisted
facts rather than a prior in-memory state", and that "the caller performs the
single durable store transaction that seals the record". The enroll handler is
that caller. Its correctness on the crash boundary is defined entirely in terms
of a persisted record and a persisted invalidation marker, neither of which
exists before `ADR046-store-004`.

One correction to the shipped code's own account, recorded rather than
silently reconciled: the `service.rs` module documentation still names the
absent Zone session contract as the blocker and says
`d2b_contracts::v3::zone_session` "is still empty". The session contract and
the enrollment state machine have since landed in `packages/d2b-bus/src/session/`.
The stale comment is not corrected here, because `service.rs` belongs to the
sealed W2; the W5 slice that lands the handler should correct it.

## 2. Semantics inferred where the specification is silent

Each of these is a defensible reading that a reviewer should confirm or
correct. They are recorded because a silent inference is indistinguishable
from a specified rule six months later.

| Where | Inference | Owning wave |
| --- | --- | --- |
| `v3/volume.rs` | `volumeAttachmentDefaults` entry shape is undefined everywhere in the spec set; every occurrence is an empty list. Typed as an opaque object rather than invented as `{volumeRef, view, access}`. This is the one non-strict spot in the primitive surface. | W5 |
| `v3/volume.rs` | `SensitivityClass` admitted as `public | private | secret`; only `private` is attested by spec text. | W5 |
| `v3/volume.rs`, `v3/process.rs` | `RepairPolicy`, `CleanupPolicy`, `AdoptionPolicy`, `EntryRestartPolicy`, `LeaseClass` and `Invariant` value sets are only exemplified in YAML samples, never enumerated. Only attested values are admitted. The Volume providers had to map onto this narrower set, using `exact-mode` for the TPM root and carrying `same-filesystem` as a drift class rather than an invariant. | W5 |
| `v3/network.rs` | MTU bound `576..=9216` inferred; spec states the default but no range. | W5 |
| `v3/credential.rs` | `ExpirySpec.hardDeadlineMs` additionally capped at the maximum lease lifetime, since a deadline beyond the longest possible lease is unreachable. Not a stated rule. | W5 |
| `v3/zone_routing.rs` | The v3 capability catalogue is not enumerated anywhere. Modelled as a bounded token set with subset ordering rather than a frozen enum, because freezing it would have invented a wire contract four slices consume. | W2, before the routing contract is treated as frozen |
| `v3/zone_routing.rs` | Zone label bound narrowed from the baseline 128 bytes to 63, on the reading that a Zone label is a Zone resource name. Deliberate narrowing. | W2 |
| `v3/zone_routing.rs` | The private allocated-capability field shape is an interpretation: modelled as non-serialized state attachable only through a narrowing constructor. | W2 panel |
| `zone_routing.rs` resolver | The local root may not absorb an unmatched target. Keeping the baseline's ancestor-coverage rule verbatim would make the resolver unconditionally permissive, since the local root is a suffix of every in-scope path. Non-root ancestors still cover descendants. | W2 panel |
| `zone_links.rs` | Three refusal labels are not spelled in any spec and were named locally: `bootstrap-psk-invalidated`, `zone-link-not-ready`, `reconnect-budget-exhausted`. Need a register decision if they are to be wire-stable. | W2 |
| `zone-resources-json.nix` | Canonical bytes come from the Nix JSON builtin, which coincides with the canonical form for the ASCII-constrained data here. A full canonicalization implementation is absent. | W5 |

## 3. Code that exists but is unreachable

Landed, tested, and currently wired to nothing. Each is intended, but each is
also invisible until wired, so it must not be mistaken for working behaviour.

- `nixos-modules/resources-volume.nix` - Volume assertions are imported by no
  module, so they do not run. Needs an import from `index.nix` or
  `default.nix`, both owned by other slices.
- `nixos-modules/options-zones.nix` - imported by `index.nix` only, not by
  `default.nix`, which the work item requires.
- `nixos-modules/generated/options-zones-*.nix` and `zone-resources-json.nix` -
  generated and correct, imported by nothing.
- Both process Providers and both Volume Providers - complete but unwired,
  pending their effect-port adapters.
- The controller engines remain deliberately unwired from the absent production
  store and watch dispatcher, per the standing boundary.

## 4. Gate and test debt

| Debt | Detail | Owner |
| --- | --- | --- |
| `flake.nix` zone-schema-drift check | The work item asks for `checks.<system>.zone-schema-drift` plus a matrix pin refresh. Not added. | W2 |
| `public_mint_surface` runtime | Renders rustdoc for every workspace member sequentially into isolated target dirs; roughly 30 minutes and growing with every crate added. Its earlier characterization here - that the render phase is parallelizable and the dependency ordering is only needed for the analysis phase - is corrected below; the ordering is needed by the render itself. | **Ruled: W3.** See section 9 |
| Unknown-spec-field rejection | Cannot be enforced while the shared `spec` type injects execution-policy defaults into every resource. Needs the generated per-type submodule to replace the freeform type, which requires editing a file the generator slice does not own. `nix-unit: zone-link-closed-spec` cannot pass until then. | W2 |
| Two engine refusal branches unreachable from outside | The contract constructors already reject the shapes that would trip them, so they guard only the deserialization path. Exercising them needs a deserialization-based vector, a different surface than the vector suite owns. | W2 panel to rule |
| Enrollment validation obligations | Four obligations unmet, blocked on the session contract. | W2, with routing-007/009 |
| `UNIMPLEMENTED_SCAFFOLD` markers | Still present in several crates, deliberately, because the capability gate fails closed on a crate advertising no public item. Each must be deleted by the slice that fills its crate. | Per slice |


### Discharged during Wave 2

Recorded as closed rather than deleted, so a reader can tell the difference
between debt that was paid and debt that was never real.

- The capability and public-API snapshots were regenerated and reviewed, and
  the widening was approved with a stated reason.
- The drift gate now runs both Zone generators and compares
  `nixos-modules/generated/`, so the header those artifacts carry promising
  byte-for-byte comparison is now true rather than aspirational.

## 5. Specification drift found while implementing

Recorded per the standing rule that drift is raised as a separate amendment
and never corrected inside an implementation wave.

- **`transportProviderRef` pattern conflict.** The normative zone-control
  schema requires a `transport-` prefixed Provider reference; the zone-routing
  document's illustrative excerpt permits any Provider name. The generator
  follows the normative one.
- **Assertion message wording.** The specification's assertion table restates
  two key-format checks that already exist in shipped code with different
  wording, pinned by an existing eval case. Shipped code was kept.
- **`metadata.labels` and `annotations`** are described as authored fields but
  the metadata submodule declares only an owner reference, so they are
  currently unauthorable.
- **`spec.updatePolicy`** is described as part of the universal base spec for
  every zone-control type, but the ZoneLink spec is frozen at exactly six
  fields. Not emitted.
- **Frozen Provider method taxonomy** has no v3 re-freeze, so the capability
  set is a bounded token set rather than the specified taxonomy.
- **W3 destination set disagrees with the section 3.2 wave table.**
  `docs/specs/ADR-046-validation-and-delivery.md` section 3.2 gives W3's
  destinations as only `packages/d2b-provider/`, `packages/d2b-provider-toolkit/`
  and a `packages/d2b-provider-<base>-<implementation>/` skeleton generator. It
  names neither `packages/d2b-contracts/src/v3/provider.rs` (destination of
  `ADR046-provider-001`), nor
  `packages/d2b-contracts/src/v3/semantic_services/` (destination of
  `ADR046-provider-004`), nor `packages/d2b-provider-system-core/` (destination
  of `ADR046-provider-003`). The same table lists
  `packages/d2b-provider-system-{core,systemd,minijail}/` in its **W5** row,
  while `ADR046-provider-003` is a W3 item in
  `ADR-046-implementation-graph.json`. `ADR-046-work-items.json` is canon:
  FR-046 makes the generated manifests authoritative over prose on wave
  assignment, destination paths, and work-item identity, and `tasks.md` states
  that each task is a pointer to a manifest entry and that those manifest
  fields are the task. Implementers follow the manifest; the section 3.2 W3 and
  W5 rows are treated as stale prose for those entries only. Not corrected in
  place, per FR-046 - `ADR-046-validation-and-delivery.md` is untouched, and
  this is the same shape as
  [`amendment-w2-destination-drift.md`](./amendment-w2-destination-drift.md),
  which should carry the prose change in its own amendment with its own
  validation and panel round.
- **Zone and Volume assertion messages rewritten for FR-017 actionability.**
  A panel finding established that the assertion messages pinned verbatim by
  the specification were not FR-017 compliant: each named the violated rule
  but no concrete operator action. The shipped code in
  `nixos-modules/assertions.nix` and `nixos-modules/resources-volume.nix` was
  rewritten so every message names the option path to edit and the edit to
  make. Code wins, so the pinned strings in the specification validation
  tables were updated to match the emitted text byte-for-byte: the Zone
  assertion table in `docs/specs/ADR-046-zone-routing.md` (reserved zone key,
  `parentZone` topology, Zone self-resource, ZoneLink sole-uplink, unresolved
  `transportProviderRef`, zone count, per-zone resource count, and forbidden
  `transportSettings` keys) and the transport-settings row of the assertion
  table in `docs/specs/providers/ADR-046-provider-transport-unix.md`. Two
  shapes could not be pinned as a single literal. The sole-uplink message now
  branches on whether the Zone is `local-root`, so its single table row was
  split into one row per case. The forbidden-key message now interpolates the
  offending key names from a framework constant list, so the table uses a
  `<forbidden-keys>` placeholder and states what it stands for. The
  transport-unix row also carried a pre-existing defect independent of this
  rewrite: it pinned a `spec.transportSettings` message prefix where the code
  has always emitted `transportSettings`. That prefix was corrected in the
  same edit. Prose that describes these rules rather than pinning their exact
  text, such as the Zone self-resource line in
  `docs/specs/ADR-046-resources-zone-control.md`, was left unchanged.

## 6. Corrections to this program's own process

- **The `FR-047` false alarm.** Four independent implementers reported that
  `FR-047` does not exist. It does - in this feature's `spec.md`. They searched
  `docs/specs/` because the dispatch prompt cited the decision register and the
  requirement in the same breath. The requirement is real and was met; the
  prompt was wrong. Future dispatch prompts must cite a requirement by its
  file, not only by its number.

---

## Added at Wave 2 close

New debt discovered while completing the wave's last five items.

| Debt | Detail | Owning wave |
| --- | --- | --- |
| Appended Zone tags cannot reach the wire | The session contract appends six Zone members at new tags, but the canonical handshake offer encoder types its fields with the un-extended enums and lives in a file no W2 slice owned. Widening it in place would have invalidated the committed golden vectors. Enrolled links and bootstrap use preserved tags, so ZoneLink is unaffected; carrying an appended tag needs an owned decision on the offer encoding. **Not W3**: the encoder is `packages/d2b-session/src/handshake.rs`, and no W3 work item names `packages/d2b-session/`. `ADR046-exec-018` (W5) owns the v3 session wire types and re-types `EndpointPolicy` - `encode_offer`'s sole input - onto v3 `ZoneId`/`ProviderId`, and `ADR046-reuse-001` (W5) owns the v3 contract extension to that crate. | W5, `ADR046-exec-018` |
| Session tag values are an inference | No specification fixes the numeric tags for the six appended members, nor the ZoneLink service wire string. The scheme chosen is append-only, next unused tag, never renumber, with two tags permanently reserved rather than reused. These are wire-visible and need panel confirmation before anything depends on them. | W2 panel |
| Subject-digest prologue field is a choice | The specs name no field for the subject-context digest. It is folded into the existing channel binding, which is already inside the canonical offer and therefore inside the handshake prologue, so no wire change was needed. Worth confirming. | W2 panel |
| Sealed enrollment record does not bind the child uid | The spec says the record binds the child static key pin to the child Zone uid. The session module holds the fingerprint and an opaque allocator-binding digest; the uid binding belongs to the durable store transaction owned by the ZoneLink controller. **Not W3**: no W3 work item owns `packages/d2b-session/` or `packages/d2b-core-controller/`. The transaction is `ADR046-store-004`, destination `packages/d2b-resource-store-redb/src/transaction.rs`, which the graph places in W5; the controller-side write lands in `zone_links.rs` behind it. | W5, `ADR046-store-004` |
| No durable persistence for enrollment | Recovery takes the persisted facts as arguments. The store transaction that seals or invalidates a record is the controller's and is not implemented. **Not W3**: same determination as the row above - the durable store backend and its transaction module are `ADR046-store-004` in W5, and no W3 destination is a store or controller path. | W5, `ADR046-store-004` |
| `component_session` runtime tests not duplicated | The bus re-exports the session runtime rather than forking it, so its 2,121 lines of tests were deliberately not copied; they run in the owning crate. The ported golden vectors are the port evidence. A scope judgement worth a reviewer's confirmation. | W2 panel |
| Principal digest has no frozen domain tag | The cross-Zone idempotency key needs a subject digest, but the frozen digest-tag list has no principal or subject tag, so the digest is currently undomained. If a tag is later frozen, the computation changes. **Not W3**: the digest site is `packages/d2b-bus/src/zone_route.rs` (`ADR046-routing-005`, W2) and the frozen tag list is decision D101, landed by `ADR046-object-001` in W0. Both waves are sealed, and no W3 destination is either file. **Ruled: amendment-shaped, batched with the row below.** | Amendment, not wave work: [`amendment-frozen-cross-zone-contracts.md`](./amendment-frozen-cross-zone-contracts.md), before W6 |
| No closed reason for a multi-Zone batch | The routing reason enum has no variant for a batch spanning Zones, so a structural error is returned rather than misusing an unrelated routing reason. **Not W3**: `ZoneRouteFailClosedReason` lives in `packages/d2b-contracts/src/v3/zone_routing.rs`, whose sole owning item is `ADR046-routing-001` in the sealed W2, and the refusal site `ZoneRouteError` is in `packages/d2b-bus/src/zone_route.rs` (`ADR046-routing-005`, W2). No post-W2 item names either file. **Ruled: amendment-shaped, batched with the row above.** | Amendment, not wave work: [`amendment-frozen-cross-zone-contracts.md`](./amendment-frozen-cross-zone-contracts.md), before W6 |
| Unix session tests delegated rather than ported | The manifest asked to port the unix session tests verbatim, but the integrator wired the owning crate as a dependency instead, so copying them would fork the audited substrate. Zone-level semantics were ported instead. Needs a ruling: accept delegation, or add the syscall dev-dependency and port literally. | W2 panel |
| Listener portal transport variant | One spec describes a pre-bound socket handed over a portal call while another describes an inherited connected socket. Only the connected form is implemented; the portal wire contract belongs to a transport Provider crate and is unspecified for the bus. | W6 |

## Wave 3 provider-crate layout and naming policy: scope and exemptions

The Wave 3 crate-layout and naming policy - the mandatory `src/`, `tests/`,
`integration/`, `README.md` shape and the naming rule that `ADR046-provider-002`
carries - scopes to crates matching `d2b-provider-<base>-<implementation>`.

Two existing crates are **exempt**:

| Crate | Reason for exemption | Owner of retiring the exemption |
| --- | --- | --- |
| `packages/d2b-provider-aca` | Pre-ADR-046 crate. Its name carries a single segment after `d2b-provider-`, so it does not match `<base>-<implementation>` at all. `ADR-046-current-code-migration-map.md` dispositions `AcaWorkloadProvider` and its `GuestControlEndpointProvider` impl as REPLACE, superseded by `Provider/runtime-azure-container-apps`. Forcing it to conform in W3 would reshape a crate scheduled for deletion. | W6, `ADR046-aca-001`, whose removal proof is "`packages/d2b-provider-aca/` removed only after conformance suite green" |
| `packages/d2b-provider-relay` | Pre-ADR-046 crate, same single-segment naming mismatch. The migration map dispositions `AzureRelayTransportProvider` as REPLACE, superseded by `Provider/transport-azure-relay`. | W6, `ADR046-aca-004`, whose removal proof is "`packages/d2b-provider-relay/` removed after `transport-azure-relay` Provider conformance". `ADR046-transport-relay-001` (also W6) retains the relay plumbing until ACA display migration completes, so the exemption cannot retire before both land |

Every other `packages/d2b-provider-*` crate in the tree already matches the
`<base>-<implementation>` form and is in scope:
`d2b-provider-system-systemd`, `d2b-provider-system-minijail`,
`d2b-provider-volume-local`, `d2b-provider-volume-virtiofs`.
`packages/d2b-provider-toolkit` is the shared toolkit named by
`ADR046-provider-001`, not a Provider crate, and is out of the naming rule's
scope by construction.

## 7. Known flakes observed during Wave 2

Recorded so a later run does not rediscover them as regressions.

| Test | Observation | Assessment |
| --- | --- | --- |
| `d2b-unsafe-local-helper::shell_supervisor real_supervisor_preserves_pty_across_reconnect_and_kills_exact_scope` | Failed once with "supervisor did not exit" during a full parallel run, passed 3 of 3 in isolation, and passed on the next full run of all 4499 tests. | Environment-sensitive, not a wave regression. The wave never touched that crate, and the test spawns a real supervisor in a transient scope and waits for it to exit, which is timing-sensitive under heavy parallel load. Worth a bounded wait rather than an unbounded one if it recurs. |
| Capability seal fixtures | Two seal fixtures reported a downstream compile failure that read as a trust-boundary regression. The real cause was a stale fixture lock after the bus gained dependencies. | Fixed. Worth knowing that this failure mode is indistinguishable from a genuine seal break in its message, so a stale lock should be ruled out first. |
| Compile-fail tests under a caching compiler wrapper | A capability seal failed once with a wrapper client exiting nonzero under concurrent cargo invocations. | Mitigated earlier in the program by clearing every wrapper spelling for those spawned compilers. The original failure was never reproduced on demand, so that remains a reasoned mitigation rather than a demonstrated fix. |

## 8. Validation obligations not met by Wave 2

The panel found these silent rather than recorded, which is the defect: an
unmet obligation is acceptable when it is written down and a scheduling
problem when it is not. Each names the work item that owes it.

This section was first written by transcribing the three items a reviewer
happened to name, which is not an audit and produced exactly the gap the
panel then caught. It has since been rebuilt by reading the `validation`
field of every one of the wave's nineteen work items against the tests that
actually exist. The three Zone configuration entries below are unchanged;
what follows them is the remainder of that audit.

The three Zone configuration items carry obligations at test layers this wave
did not reach. The Rust behaviour they describe is covered by in-crate tests;
what is missing is the declarative and integration layer that proves the same
behaviour through the module system and against a booted host.

| Work item | Obligation | Where it belongs |
| --- | --- | --- |
| `ADR046-routing-011` | Eval cases for the zone name grammar, the parent topology rules, the uplink placement rules, the credential reference shape, and the transport-settings secret rejection. Also a drift case pinning the standard resource-type registry. | `tests/unit/nix/cases/`, plus the drift gate |
| `ADR046-routing-012` | Build-level flake checks for bundle determinism, sealed parent topology, the child-local uplink bundle, the exactly-six-field uplink spec, unknown transport-settings fields, the transport credential reference, and a missing transport provider. | `flake.checks.<system>` |
| `ADR046-routing-013` | Host-integration checks for cleanup of a removed uplink, rollback restoring one, a dynamic child surviving cleanup, and the absence of a reciprocal row in the parent store. | `tests/host-integration/` |

Two things make this smaller than it looks, and one makes it larger.

Smaller: the eval-case obligations were exercised out of tree during
implementation, with a conformant Zone yielding no assertion and thirteen
distinct misconfigurations each producing their intended message. That
evidence exists in the implementation record but is not committed as a case
file, so it does not run in the gate. Landing it is transcription rather than
design.

Also smaller: the host-integration obligations cannot be met until the modules
are imported and a production store exists, both of which belong to later
waves. Writing them now would produce checks that cannot execute.

Larger: the flake checks are the layer that would catch a generator emitting a
non-deterministic bundle, and nothing else in the wave covers that. The
generators are deterministic by construction and the drift gate now compares
their output, but neither proves determinism across two independent
evaluations of the same input.

### The rest of the audit

Six further obligations are owed by four other items. Each is recorded with
the honest reason it is not met and the thing that would discharge it.

| Work item | Obligation | Why it is not met | What would discharge it |
| --- | --- | --- | --- |
| `ADR046-primitives-002` | Host/Guest/user integration, alongside the shared conformance suite | Both Provider conformance suites run only against `ScriptedEffectPort`, a hermetic mock. A scripted collaborator proves the Provider's own decision logic and proves nothing about a real Host, Guest, or user domain, and the crates are imported by no production caller. | The `ProcessLaunchEffectPort` production adapter, then the same conformance obligations re-run against it across the three domains. The adapter is `ADR046-process-001` in W4. |
| `ADR046-primitives-003` | virtiofs host/guest mount tests | `export_lifecycle.rs` drives `VirtiofsExportController` through a `ScriptedPort`, so "the host serves and the guest mounts" is asserted as a controller state transition, not as a mount. Nothing in the wave mounts anything. | A host-integration check that exports a Volume and mounts it in a booted guest. Blocked on the same absent effect adapter. |
| `ADR046-primitives-003` | ACL, no-follow, and marker tests | `packages/d2b-provider-volume-local/integration/README.md` names the five fixtures that need a real filesystem boundary - Host-path access, the `st_dev` store-view boundary, marker durability across restart, quota enforcement, and the TPM marker - and states plainly that none is wired, because without the adapter they would assert against a stub. The hermetic suite proves these at the policy layer only. | Wiring those fixtures behind the landed effect adapter. |
| `ADR046-primitives-003` | Eval cases for the Volume assertions in `resources-volume.nix` | The module is imported by nothing (section 3), and `tests/unit/nix/cases/` holds no Volume case, so the assertions neither run in a configuration nor in the gate. | An import from `index.nix` or `default.nix`, then eval cases plus `make nix-unit-pin`. Both owned by later slices. |
| `ADR046-routing-001` | Golden advertisement, path, and failure vectors **shared by Rust and Nix** | The Rust half is met: the vectors are pinned in `v3/zone_routing.rs` and render canonical bytes. The Nix half does not exist, because no Nix code consumes a routing vector and no Zone eval case is committed. A vector that only one side reads cannot detect the two sides diverging, which is the entire point of sharing it. | A Nix-side consumer of the same vector bytes, landing with the `ADR046-routing-011` eval cases. |
| `ADR046-routing-006` | p95 benchmark gate | `benches/route_decision.rs` exists and is declared `harness = false`, but no target runs it: it appears in no `Makefile` target, no `tests/layer1-jobs.json` job, and `tests/unit/gates/performance-budgets.sh` names no Zone routing budget. An unrun benchmark enforces no latency, and that gate is in any case classified advisory. | A budget entry naming the route-decision measurement, and an entrypoint that runs the benchmark. |

Two more deserve stating precisely rather than being counted as met.

`ADR046-routing-005` owes an end-to-end K0 to K1 to K2 resource call and
`ADR046-routing-010` owes a cross-Zone K0 to K1 end-to-end test. Both have a
test of that name, and both are in-crate simulations: the bus test seals an
envelope and pins a reverse path against an engine decision it is handed, and
the client test walks a route table. The routing, hop-budget, and
idempotency semantics they assert are genuinely covered. What is not covered
is any hop crossing a transport, because the bus stays deliberately unwired
from production listeners. Recorded as covered-at-the-logic-layer rather
than as end-to-end.

`ADR046-routing-016` owes bootstrap, enroll, resolve-route and shortcut
integration tests against a child-local fake ZoneLink. Resolve-route and
shortcut are met in `service.rs`. Bootstrap and enroll are not, and cannot
be: the service has no handler for them, which its own
`a_method_without_a_landed_handler_is_refused_at_admission` test pins. The
blocker and its W3 owner are already recorded in section 1; the validation
obligation is named here so it is not lost when that row closes.

Two port obligations - `ADR046-routing-007`'s `component_session.rs` tests
and `ADR046-routing-008`'s `unix_session.rs` tests - are unmet by delegation
rather than by omission, and are already recorded with their reasoning in
the wave-close table above. They are not repeated here.

The remaining ten items were audited and their obligations are discharged:
`primitives-001` (schema vectors across the nine v3 primitive modules, and
the folded-field and duplicate-type checks, which are inline Rust tests in
`packages/d2b-contracts/src/v3/execution_policy.rs` and so run enforcing
under `make test-rust`, not under `make test-policy`);
`routing-002` (the route-engine suite is present with
relay, hop-count and capability-narrowing cells, and every reason the engine
can produce is proved covered by a closed-set test); `routing-003`
(longest-suffix vectors over a sealed topology, and the stale, withdrawn,
unauthenticated and no-parent-row cells); `routing-004` (the state-machine
transitions, all three crash windows, the key-lifecycle cells, and the
structural metric-descriptor test asserting no identity reaches a label);
`routing-005` and `routing-010` as qualified above; `routing-006`'s
baseline vector cases, separately from its unwired benchmark; `routing-008`'s
allocator FD handoff and its no-socket-activation cell; `routing-009`
(round-trip, canonical encoding stability, and closed-enum exhaustiveness);
`routing-012`'s two drift obligations, which the drift gate now runs;
and `routing-016` other than bootstrap and enroll.

## 9. What implementation debt Wave 3 takes on, and what it does not

Recorded before the wave's slices open, so its scope is settled rather than
argued at review. Three items are in scope and one named group is explicitly
not.

### In scope

**1. The v3 Provider-method DTO catalogue.** Already ruled as owned by
`ADR046-provider-001` and stated in section 0. Discharging it closes the
Destination caveats recorded against `ADR046-routing-014` and
`ADR046-routing-015` in section 1, which are the two partial items in the
Wave 2 delivery claim. Nothing else in the register turns those two rows from
partial to complete.

**2. The `public_mint_surface` gate runtime.** Wave 3 pays this rather than
inheriting it, because Wave 3 is the wave that makes it worse. The gate's cost
scales with workspace member count, and Wave 3 adds at least
`packages/d2b-provider-system-core/` (destination of `ADR046-provider-003`,
absent from `packages/Cargo.toml` today) and possibly several more through
`ADR046-provider-002`'s skeleton generator, which emits one
`packages/d2b-provider-<base>-<implementation>/` crate per Provider. A wave
that adds crates to a per-crate-linear gate is the wave that should pay for the
gate's shape.

The register's earlier characterization was verified against
`packages/d2b-bus/tests/public_mint_surface.rs` and is **partly wrong**, which
matters because the wrong half was the argument that the fix is cheap:

- Confirmed: `render_workspace_docs` loops over `dependency_order(packages)`
  and runs one `cargo doc --no-deps -p <package>` per workspace member, in
  sequence, into a per-crate isolated render directory.
- Corrected: the dependency ordering is **not** needed only by the analysis
  phase. Before rendering a package, the loop calls
  `plant_dependency_doc_link` for every already-rendered crate
  (`external_docs.iter().chain(docs.iter())`), symlinking those render roots
  into the package's own doc root. A render therefore consumes the output of
  the renders before it, and a flat parallel render would not have them.
- Consequence for the fix: a render/analyze split is **not** available in the
  form the register claimed. What is genuinely available is (a) hoisting the
  pure source scans - `hidden_public_api` and
  `source_capability_inventory_with_externals` - out of the render loop, since
  neither reads rendered output, and (b) parallelising the render *within* each
  level of the dependency order rather than across the whole workspace. Both
  are real wins; neither is the flat parallelisation the old wording implied.
  A Wave 3 slice must scope to the corrected shape.

**3. A build-level determinism flake check for Wave 3's own generated Nix
catalog.** Section 8 records that the flake-check layer - proving a generator
emits identical output across two independent evaluations - is the one thing
neither construction-time tests nor the drift gate cover. The drift gate
compares a generator's output against the committed tree, which catches a
generator that changed; it does not catch a generator that emits different
bytes on two runs of the same input. Wave 3 ships a new generator
(`ADR046-provider-002`'s Provider package and catalog emitter), so it must not
repeat the gap it can see in the wave before it.

### Not in scope

**4. The missing nix-unit eval cases and host-integration checks recorded
against `ADR046-routing-011`, `ADR046-routing-012`, `ADR046-routing-013` and
`ADR046-primitives-003`.** These stay where section 8 puts them.

Two reasons, and the first is sufficient on its own. Every one of those
obligations is owed by a work item in the sealed Wave 2 and already carries a
recorded owner; transcribing them into Wave 3 would give the same obligation
two wave attributions and make the wave that discharges it ambiguous, for no
gain in when it actually lands. Second, several cannot execute yet regardless
of who owns them: section 3 records that `resources-volume.nix` and
`options-zones.nix` are imported by no module, and section 8 records that the
host-integration obligations need both those imports and a production store,
all owned by later waves. Writing them in Wave 3 would produce checks that do
not run.

What Wave 3 does owe them is the pattern. Item 3 above establishes the
build-level determinism flake check on Wave 3's own generator; the
`ADR046-routing-012` determinism obligations follow that shape when their wave
lands, rather than each wave re-deciding what a determinism check looks like.

---

## 10. Added at Wave 3 close

Debt reported by the wave's three slices, each verified against the tree
before it was recorded rather than transcribed from the slice's own account.
Where verification disagreed with a slice, the tree is recorded and the
disagreement is stated.

The three categories the register already separates are kept apart here,
because the distinction is what makes the entry actionable: an **unmet
obligation** names work someone still owes, an **inference** names a reading a
reviewer must confirm or correct, and a **specification correction** names a
place where shipped code and written specification disagree and code was kept.

### 10.1 Unmet obligations

| Debt | Detail | Owning wave |
| --- | --- | --- |
| v3 Provider-method DTO catalogue, remainder | Partly delivered, and the rest is blocked on specification content that does not exist. Verified in `packages/d2b-contracts/src/v3/provider.rs`: method names exist for **one** of the eleven Provider families - the Transport triple `openTransport`/`closeTransport`/`observeTransport` and the controller currency triple `assessUpdate`/`planUpgrade`/`executeUpgrade` - and nothing is named for display, clipboard, notification, shell-terminal, credential, device, volume or network. **No request or response payload is written anywhere for any method**, including the six that are named: `openTransport` is described as returning an opaque handle and observations whose shape appears in no document. There is no proto, no frozen service name and no field numbering, which is exactly what `ADR046-routing-015` needs to freeze. Consequence, confirmed in the tree: `packages/d2b-provider/src/identity.rs` and `src/lib.rs` name the `ProviderInstance` sum type and the `RpcProviderProxy` payload only in prose, no such type exists, and the registry stays generic over the runtime's own opaque instance handle. | `ADR046-provider-001` retains ownership. **What would unblock it**: a specification amendment that (a) names the methods of the remaining ten families, (b) writes the request and response payload for every method including the six already named, and (c) freezes a service name and field numbering. Until that lands, no wave can discharge it, and scheduling it into a later wave would only move the same hole |
| Catalog / Provider-manifest parity test | The offline Nix catalog (`nixos-modules/generated/provider-catalog-shape.nix`, 25 fields) and the `ProviderManifest` DTO (`packages/d2b-contracts/src/v3/provider.rs`) describe the same Provider facts in two places, with nothing comparing them. The packaging slice deferred this to "whichever lands second". **Verification corrects that deferral**: both landed inside Wave 3, in `56196815` and `753e1e63`, so there is no later second lander. The parity test is owed now, and it is additionally named by `ADR046-provider-002`'s own `validation` field as "catalog parity policy" | W3, `ADR046-provider-002` |
| Two conformance cells duplicated per Provider crate | `packages/d2b-provider-system-{systemd,minijail}/tests/execution_parents.rs` are near-identical files, differing only in the provider type and one test name. Two cells belong in the shared suite: execution-parent neutrality and the disagreeing wait owner. The slice named them `assert_execution_parent_is_neutral` and `assert_a_disagreeing_wait_owner_quarantines`; **those are proposed suite-helper names, not names in the tree** - the cells are currently spelled `a_non_host_execution_parent_yields_the_same_status_shape` and `a_candidate_whose_wait_owner_disagrees_is_quarantined` in both crates. The suite is `packages/d2b-process-conformance/src/suite.rs`, which no Wave 3 slice owned | The wave that next owns `packages/d2b-process-conformance/` |
| No Provider dossier parity check | See section 11; recorded there with the rest of the audit | W3, `ADR046-provider-002` |
| `d2b-provider-toolkit` has no `[dev-dependencies]` | Confirmed: its `Cargo.toml` declares one dependency and no dev-dependency table, so its integration tests cannot use `serde_json`. Malformed-wire rejection is therefore proved in the contracts module rather than against the toolkit's own fakes. Not wrong, but the coverage sits one crate away from the surface it describes | W3 or any later slice touching the toolkit; a two-line manifest change |
| `SchemaVersion` has no component accessor | Confirmed: `packages/d2b-contracts/src/v3/resource_schema.rs` exposes no `major()` or `minor()`, so `schema_version_parts` in `provider.rs` renders the type's own canonical string and parses it back. The comment in that function states the reasoning, and it is sound - the canonical spelling is the contract's own round trip, so the parse is exact rather than lossy. It remains indirect, and a one-line accessor would remove the parse and its two `expect` calls | Cosmetic. Any later slice owning `resource_schema.rs` |

### 10.2 Semantics inferred where the specification is silent

| Where | Inference | Owning wave |
| --- | --- | --- |
| `v3/provider.rs` | The D089 standard capability matrix is modelled as a bounded token map (`StandardCapabilityMatrix(BTreeMap<BoundedToken, CapabilitySupport>)`) rather than a frozen enum, because the specification mandates the matrix and names the classes it covers but never enumerates the optional-capability identifiers. Absence fails closed: a capability the matrix does not list is unsupported rather than assumed. This is the same shape, and the same reason, as the v3 capability catalogue already recorded against `v3/zone_routing.rs` in section 2 | W3, before the Provider capability surface is treated as frozen |
| `nixos-modules/generated/provider-catalog-shape.nix` | The catalog's **25 field names** are an inference. The specification's bullets name concepts, not identifiers, so the field **set** is the specification's and the **names** are not. The file records the concept-to-field mapping explicitly in its `fieldGroups`, which is the right shape for a later reviewer to check the inference rather than rediscover it | W3, confirmed by the parity test recorded in 10.1 |

### 10.3 Specification drift found while implementing

Recorded per the standing rule that drift is raised as a separate amendment
and never corrected inside an implementation wave.

- **Provider crate-layout policy file name and lane.**
  `docs/specs/ADR-046-resources-zone-control.md` section 4.8.2, the
  `ADR046-pkg-001` Destination, and the matching entries in
  `ADR-046-work-items.json`, `ADR-046-implementation-graph.json` and
  `docs/specs/providers/ADR-046-provider-activation-nixos.md` all name
  `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs`, routed
  through the **advisory** `test-fixture-contracts` lane. The slice shipped
  `packages/d2b-contract-tests/tests/policy_provider_crates.rs`, wired into the
  **enforcing** hermetic `test-policy` lane at `tests/test-policy.sh`. The
  reasoning is sound and is kept: the check is filesystem-only and compiles
  nothing, so it is hermetic, and a hermetic check belongs in an enforcing lane
  rather than one that is advisory until fixture delivery is wired. Code is
  canon. Not corrected in place, per FR-046; the prose change belongs in its
  own amendment with its own validation and panel round, alongside the
  destination drift already recorded in section 5.
- **Two provider-level identity checks are unreachable through valid input.**
  In `packages/d2b-provider-system-{systemd,minijail}/tests/execution_parents.rs`
  the user-domain identity requirement cannot be reached through the provider,
  because the launch ticket refuses to be constructed without an exact user
  under a Host and under a Guest alike. The assertions are therefore pinned
  where the rule is genuinely enforced, in the ticket constructor, and both
  tests say so in their own comments: the controller's check is defence in
  depth rather than the only guard. This is worth recording rather than
  silently accepting, because the visible effect is a provider-level rule with
  no provider-level test, which reads like missing coverage. Relaxing either
  primitive now fails visibly instead of quietly removing a guarantee.

### 10.4 Code that exists but is unreachable

Extending section 3 rather than restating it.

- `packages/d2b-provider-system-{core,systemd,minijail}` have **no production
  caller**. Verified: no workspace `Cargo.toml` outside those three crates
  depends on any of them, and no `nixos-modules/` file names them.
- All three crates' `integration/` directories hold only a `README.md`. That is
  deliberate, not an oversight: without the production launch adapter a fixture
  written there could only drive a fake, which would assert nothing about a
  real system while looking as though it did.
- Everything the system Provider slice delivered is therefore **hermetic**. The
  core's User reconciler is proven only over a scripted discovery port
  (`ScriptedDiscoveryPort` in `src/testing.rs`), and both Process Providers only
  over a scripted effect port. Specifically unproven: that a real NSS lookup
  produces the bindings claimed; that a real transient unit's invocation-id,
  cgroup, main-pid and start-time verification behaves as the profile assumes;
  that a real pidfd spawn does; and that the pid-reuse guard fires against an
  actually reused pid. **Owner: `ADR046-process-001`, which
  `ADR-046-implementation-graph.json` places in `W4`** - read from that item's
  node, whose `wave` field is `W4` and whose `exitGate` names the W4 exit
  criteria. This agrees with the `ADR046-primitives-002` row already in
  section 1.
- `UserDiscoveryEffectPort` (`packages/d2b-provider-system-core/src/user.rs`) is
  **new surface introduced by that crate**, not something the Provider
  catalogue defined. If the catalogue later defines an equivalent discovery
  port, it should supersede this one rather than sit beside it, or the same
  concept acquires two incompatible spellings.

### 10.5 Already recorded, verified not duplicated

`packages/d2b-provider-aca` and `packages/d2b-provider-relay` remain
non-conformant to the crate layout and exempt by name. This was already ruled
and recorded above under "Wave 3 provider-crate layout and naming policy",
with a retirement owner for each. Verified still accurate against
`packages/d2b-contract-tests/tests/policy_provider_crates.rs`, whose
`the_two_recorded_exemptions_are_exactly_the_naming_mismatches` case pins the
exemption set to exactly those two. No new entry.

### 10.6 One honest limit on the determinism check

The build-level determinism check
(`tests/unit/smoke/provider-catalog-determinism-eval.nix`) discharges item 3 of
section 9, and its limit should be stated so a later reader does not
overclaim it.

Nix attribute sets are key-ordered by construction, so consumer authoring order
could **not** have leaked through the `lib.attrNames` calls in
`nixos-modules/provider-catalog.nix` even without the explicit `lib.sort`. The
sort is correct and worth keeping as a statement of intent, but it is not what
the check proves. What the check genuinely proves is that **nothing else
evaluation-order-dependent reaches the bytes**: it compiles the catalog twice
from separately constructed module lists that reach the same declared value by
different routes, and its negative control requires a third, different
evaluation to produce different bytes, so the comparison cannot have
degenerated into comparing a constant with itself.

## 11. Validation obligations not met by Wave 3

Built the way section 8 was rebuilt: by reading the `validation` field of every
one of the wave's four work items against the tests that actually exist, rather
than by transcribing what a slice happened to report. Two obligations below
were named by no slice.

| Work item | Validation obligation | State | What would discharge it |
| --- | --- | --- | --- |
| `ADR046-provider-001` | Contract vectors | **Partial.** `provider.rs` carries `schema_vector_pins_the_minimal_provider_base_spec` and `manifest_vector_round_trips_through_canonical_bytes`, which pin the Provider base spec and the manifest. There is no vector for any Provider **method** DTO, because per 10.1 no method payload is specified | The specification amendment in 10.1, then one vector per method |
| `ADR046-provider-001` | Fake / malicious Provider | **Met.** `packages/d2b-provider-toolkit/tests/{fake_provider,malicious_provider}.rs` | - |
| `ADR046-provider-001` | One-crate / one-identity policy | **Met, but delivered outside this item's Destination.** The rule is enforced by `one_crate_is_exactly_one_provider_identity` in `policy_provider_crates.rs`, which the packaging slice owns. Recorded so a later reader does not look for it under `ADR046-provider-001`'s three destination paths and conclude it is missing | - |
| `ADR046-provider-002` | Workspace **naming** policy | **Met.** `the_naming_convention_reads_base_before_implementation`, plus the pinned exemption case | - |
| `ADR046-provider-002` | Workspace **dependency** policy | **Met.** `every_provider_crate_respects_the_dependency_direction`, with negative cases for the daemon, broker, store and a sibling Provider | - |
| `ADR046-provider-002` | Workspace **output** policy | **Not met, but this row mislocates the obligation. Corrected in section 12.1; read that first.** The finding as originally written asserted that `nixos-modules/provider-catalog.nix` should have asserted something about a derivation's shape. It should not, and could not. What is genuinely unmet is the crate-to-package-output cardinality rule | Section 12.1 |
| `ADR046-provider-002` | Workspace **dossier** parity policy | **Not met, and named by no slice.** `docs/specs/providers/` holds a dossier per Provider, and `policy_provider_crates.rs` contains no occurrence of "dossier". Nothing ties a Provider crate to its dossier, checks that every dossier has a crate or every crate a dossier, or compares the identity the crate declares with the identity the dossier names. This is the one obligation of the four items that has no partial coverage at all | A policy case pairing `packages/d2b-provider-<base>-<implementation>/` with `docs/specs/providers/ADR-046-provider-<base>-<implementation>.md` and comparing the declared identity, with the same two exemptions already pinned |
| `ADR046-provider-002` | **Catalog parity** policy | **Not met.** Recorded in 10.1; repeated here because it is this item's own named validation and would otherwise look like a slice observation rather than an obligation | The parity test in 10.1 |
| `ADR046-provider-003` | Shared conformance tests | **Met at the logic layer, unproven at every real boundary.** The shared suite in `packages/d2b-process-conformance/src/suite.rs` runs against both Providers, and two further cells are duplicated per crate rather than shared (10.1). Every cell runs over `ScriptedEffectPort` | `ADR046-process-001` in W4, then the same suite re-run against the production adapter |
| `ADR046-provider-003` | Host / user / non-Host tests | **Met at the logic layer.** `tests/host_reconciliation.rs`, `tests/user_discovery.rs` and both `tests/execution_parents.rs` cover the three cases, over injected ports only | As above |
| `ADR046-provider-004` | Shared semantic Service / Binding contract tests, and generated schema artifacts for the eight exact qualified ResourceTypes | **Not assessable at this writing.** `packages/d2b-contracts/src/v3/semantic_services/` and `docs/reference/schemas/v3/` are being written concurrently and are not part of the three slices this section audits. `docs/reference/schemas/v3/` currently holds `Zone.schema.json` and `ZoneLink.schema.json` and no semantic-service schema. This row is a placeholder for that slice's own audit, not a finding against it | The semantic-services slice's audit against its own `validation` field, which is long and enumerates roughly fourteen distinct obligations |

Two further observations from the audit, neither of which is an obligation.

The wave's four items are unusually asymmetric in how much their `validation`
fields commit to. `ADR046-provider-003` names two things in six words;
`ADR046-provider-004` enumerates around fourteen. A short validation field is
not a weaker obligation - it is a less legible one, and 10.4 is what a
six-word field looks like when it is discharged only over scripted ports and
nothing in the field says it must not be.

The single largest gap this wave leaves is not any one of the entries above.
It is that **three crates, one Nix catalog and one Provider contract are all
proven only against each other**. Every Wave 3 deliverable is hermetic by
construction, every one is unwired, and the first evidence that any of it
matches a real system arrives with `ADR046-process-001` in W4. That is the
correct sequencing and it was chosen deliberately, but it means a green
Wave 3 gate is evidence about internal consistency and not about behaviour.

## 12. Corrections and new findings on the `ADR046-provider-002` output and parity obligations

Section 11 audited `ADR046-provider-002`'s five-term validation phrase -
"Workspace naming/dependency/output/dossier/catalog parity policy" - and got the
**output** term wrong. The correction is recorded here rather than silently
rewritten in place, because a mislocated obligation that is quietly moved leaves
no trace that the earlier reading was ever held.

### 12.1 Correction: the output obligation was located in the wrong actor

**What section 11 claimed.** That the output term was unmet because
`nixos-modules/provider-catalog.nix` "asserts nothing about that derivation's
shape", and that discharging it needed "a check on the emitted Provider
package's outputs".

**Why that is wrong.** Two independent reasons, either sufficient.

First, the sentence the term comes from is a **cardinality** rule, not a
contents rule. The crate/package boundary section of
`docs/specs/ADR-046-provider-model-and-packaging.md` reads
`- has one Nix package/conformance output;` inside a bullet list whose other
members map one-to-one onto the remaining four terms of the same validation
phrase: `declares one Provider identity` (naming), `depends only on public
neutral contracts/toolkit/SDK crates` and `does not import d2bd, broker,
Zone-store, Nix-emitter, or another Provider's implementation internals`
(dependency), `has one ADR-046-provider-<provider-name>.md dossier` (dossier),
and the "Package catalog" section the same document carries (catalog parity).
Every one of those four is a filesystem or manifest scan. Reading the fifth as a
derivation-contents check makes it the only member of a homogeneous list that
means something structurally different.

Second, the derivation-contents requirement exists, but it is stated in a
**different specification and assigned to a different actor**.
`docs/specs/ADR-046-resources-zone-control.md` section 14.10, "Phase 2 - Nix
build", carries the row:

> Artifact catalog entry has required derivation outputs (manifest, config
> schema, executable) - Provider only | Resource compiler | build failure

The mechanism column names the **resource compiler**, and the phase is Nix
**build**. `provider-catalog.nix` is Phase 1 NixOS eval. A pure eval cannot read
the contents of a derivation it has only declared, so that check could never
have lived there, and the section 11 remediation was unimplementable as written.

**Where the derivation-contents rule actually belongs.** Work item
**`ADR046-zone-control-015`**, wave **W5**. Determined by reading the manifests
rather than inferring: the item's `destination` is
`packages/d2b-resource-compiler/src/{main,bundle,schema,validator,digest,sort,secret_lint,generation}.rs`
exposed as `pkgs.d2b-resource-compiler`; its `detailedDesign` opens "Implement
all Phase 2 build-time checks (§14.10 Phase 2 table)" and states explicitly that
for each `d2b.artifacts.*` entry the compiler must "extract and hash manifest and
config schema files"; its `validation` field enumerates the section 15.8 Phase 2
build tests. The wave is read from that item's node in
`ADR-046-implementation-graph.json`, whose `wave` field is `W5`. The
`implementationState` is `Planned` and no `packages/d2b-resource-compiler/`
exists in the tree, so nothing about this obligation is discharged anywhere
today.

**What the output term therefore is, and its state.** A cardinality rule over
the workspace: one Provider crate yields one Nix package output, not several.
Its state is recorded in 12.2 immediately below, because attempting to check it
is what exposed the gap.

### 12.2 Specification gap: the required derivation outputs have no path, name, or layout

**Unimplementable as specified.** The Phase 2 row above names three required
derivation outputs - manifest, config schema, executable - and specifies no
path, no filename, no Nix output name, and no directory layout for any of them.
`docs/specs/` was searched for `manifest.json`, `provider-manifest`,
`config-schema`, `$out` and `/bin/`; none appears in any Provider packaging
context. Nor can "existing code is canon" resolve it: no Provider crate has a
Nix package output at all (`packages.x86_64-linux` exposes thirteen attributes,
none naming any of the nine `packages/d2b-provider*` crates), no Provider crate
carries a `.nix` file, and nothing anywhere in this tree asserts a derivation's
internal shape, so there is no precedent to follow either.

Consequently an implementer of `ADR046-zone-control-015` cannot write that check
without **inventing the layout contract every Provider package must satisfy**,
and an invented layout would be indistinguishable from a specified one the moment
it is committed and the first Provider package is built against it.

**This needs a specification amendment, not a wave.** The amendment must fix,
for a Provider derivation: the output name or names, the exact relative path of
the signed manifest, the exact relative path of the root config JSON Schema, and
how the executable set is located - which must be consistent with the artifact
catalog's `executableDigests` being a `map[name]sha256` with one entry per built
binary. Until it lands, `ADR046-zone-control-015` carries a hole in the middle of
its own Phase 2 table.

**Consequence for the output term, stated separately so the two are not
conflated.** The cardinality rule of 12.1 is likewise not checkable from this
source tree today, for a related but distinct reason: with zero Provider crates
carrying any package output, there is no relation in the tree between a Provider
crate and "its" Nix package output, so counting that relation would require
inventing the naming convention that maps one to the other. No such check was
written. Writing one against an invented mapping would encode a convention the
tree does not hold, which is the same failure the dossier-parity work already
declined when it chose the spec-id row over the owners row.

One thing is worth recording so the term is not later read as wholly unaddressed.
`nixos-modules/provider-catalog.nix` types `d2b.artifacts.<id>.package` as
`types.package` - singular, one derivation per `artifactId`, and an `artifactId`
selects exactly one Provider. That is the cardinality rule enforced structurally
at the one point where a Provider derivation enters d2b, by the option type
rather than by an assertion. This is an **inference**, not a discharge: it is a
defensible reading that the option type already satisfies the bullet, and a
reviewer should confirm or reject it. If confirmed, the output term is met and
section 11's row closes; if rejected, the term stays open behind the amendment
above.

### 12.3 Specification gap: the required-outputs rule has no conformance scenario

Smaller, and independent of whether 12.2 is amended.

The Phase 2 build-test table gives a named conformance scenario for each of the
two digest-mismatch rows - `nix-build-schema-digest-mismatch` and
`nix-build-manifest-digest-mismatch` - but gives **no scenario at all** for the
required-outputs-present row. Verified by reading the whole table: its fifteen
entries are `nix-build-artifact-id-missing-from-catalog`,
`nix-build-artifact-wrong-type-rejected`, `nix-build-duplicate-artifact-id`,
`nix-build-artifact-store-path-absent-from-bundle`,
`nix-build-artifact-store-path-absent-from-config`,
`nix-build-config-schema-failure`, `nix-build-schema-digest-mismatch`,
`nix-build-manifest-digest-mismatch`, `nix-build-resourcetype-collision`,
`nix-build-bundle-sorted`, `nix-build-content-hash-stable`,
`nix-build-artifact-catalog-digest-anchored`,
`nix-build-credential-ref-survives-build`,
`nix-build-inline-secret-lint-warning` and
`nix-build-inline-secret-strict-failure`. None of them is an
absent-required-output case.

The effect is that the rule is stated once, in the section 14.10 Phase 2 table,
and has **no conformance identity to cite**. `ADR046-zone-control-015`'s
`validation` field enumerates those fifteen scenarios by name, so an
implementation that omitted the required-outputs check entirely would satisfy
its stated validation. The amendment of 12.2 should add the missing scenario in
the same edit that fixes the layout, since a scenario cannot be written without
one.

**One correction to how this was reported to the register.** The finding reached
here as "section 15.3's Phase 2 build-test table". Section 15.3 is
"Provider tests"; the Phase 2 build-test table is in section **15.8**,
"Configuration generation and cleanup tests", and both work items that cite it
cite it as §15.8. The finding is correct; only the section number was wrong.

### 12.4 The catalog/manifest parity divergences, now confirmed by a landed test

Section 10.1 recorded the catalog/manifest parity test as owed. It landed in
`e15f88cc`, and it found real divergence, which it pins as exact data so that
resolving any of it fails the test and forces the entry to be struck in the same
change. The findings are read out of
`packages/xtask/src/provider_packaging.rs` and are recorded here so they are
visible outside the test that holds them.

**The digest disagreement.** Both artifacts declare exactly six digests and
agree on four - package, executable, manifest and config. The catalog's other
two follow the specification bullet
`package/executable/manifest/component/descriptor/config digests` and name a
**component** digest and a **descriptor** digest. `ArtifactDigestSet` in
`packages/d2b-contracts/src/v3/provider.rs` instead names an **exported schema
set** digest and an **exported service surface** digest. These are different
facts, not two spellings of one fact, and the contract is the side that departed
from the bullet's wording.

*Does the specification settle which side is wrong?* **Partly, and not enough to
act on.** It settles that the catalog's two names are attested concepts: the
same packaging document names a "component schema digest" among a component's
declared fields, and a Provider descriptor digest appears as a normative concept
in several Provider dossiers. It also settles that the contract's two names are
attested **nowhere**: `exported schema digest` and `exported service surface
digest` appear in no document under `docs/specs/`. What it does **not** settle is
whether `ArtifactDigestSet` is obliged to mirror the catalog bullet at all,
because the one normative artifact-catalog field table - section 4.3.1 of
`docs/specs/ADR-046-resources-zone-control.md` - enumerates only `digest`,
`executableDigests`, `manifestDigest`, `configSchemaDigest` and
`conformanceAttestationDigest`, and names **neither** pair. Two documents
therefore give two different digest sets for the same artifact, and a third
spelling sits in the contract.

**Ruling needed**, and it is a three-way reconciliation rather than a choice
between two: whether the artifact catalog carries six digests or the four-plus-
attestation of section 4.3.1, and if six, whether the fifth and sixth are
component/descriptor or schema/service. Recorded as needing a ruling; not
resolved here, because picking a side would be exactly the invention this
register exists to prevent.

**Five catalog facts with no counterpart in the manifest at all.** Each is named
by a specification bullet on the catalog side and absent from `ProviderManifest`:
package name, version, systems, platform, and support contact. Pinned in the
test as `CATALOG_FIELDS_WITHOUT_A_CONTRACT_FIELD` alongside the two disputed
digests.

**One contract-only field.** `TrustEvidence::publisher_trusted` - whether the
publisher is in the Zone's trusted publisher set at the verified root epoch - has
no catalog counterpart. Pinned as
`CONTRACT_FIELDS_WITHOUT_A_CATALOG_FIELD` together with the two contract-side
digests.

| Finding | Class | Owning wave |
| --- | --- | --- |
| Output obligation mislocated to `provider-catalog.nix`; the derivation-contents rule is `ADR046-zone-control-015` | Correction to this register | Recorded, no wave work |
| Required derivation outputs have no path, filename, output name, or layout | Specification gap | Amendment, before `ADR046-zone-control-015` in W5 |
| Output cardinality not checkable: no Provider crate has a package output, so the crate-to-output relation does not exist in the tree | Unmet obligation, blocked | Behind the amendment above |
| `d2b.artifacts.<id>.package` typed `types.package` already enforces the cardinality at the one entry point | Inference, needs confirm or reject | W3 panel |
| Required-outputs row has no conformance scenario in the section 15.8 Phase 2 table | Specification gap | Same amendment |
| Catalog names component and descriptor digests; contract names exported schema and service digests; section 4.3.1 names neither | Ruling needed, three-way | Amendment, before the Provider packaging surface is treated as frozen |
| Five catalog facts absent from the manifest; one contract field absent from the catalog | Unmet obligation, pinned as data | `ADR046-provider-002`, closes when the ruling above lands |
