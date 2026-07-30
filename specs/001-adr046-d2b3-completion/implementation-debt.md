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
whose `.waves[]` entry for W3 pins `workItemCount: 4`. Discharging it closes
the Destination caveats recorded against `ADR046-routing-014` and
`ADR046-routing-015` below.

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
| `ADR046-routing-016` service | Still no handler for `zone-bootstrap` and `zone-enroll`, but the blocker moved: the session contract and enrollment machine now exist in the bus, so wiring the service to them is ordinary work rather than a missing contract. The four enrollment obligations are met in the bus session module, not in the service. **Not W3**: the destination is `packages/d2b-zone-routing/src/service.rs`, which no W3 work item owns. Every work item whose destination names `packages/d2b-zone-routing/` - `routing-002`, `routing-003`, `routing-006`, `routing-016` - is W2, so the artifacts name no later owning wave. Practically it is also blocked behind the same W5 durable enrollment store as the two enrollment rows in the wave-close table below. | Needs an integrator ruling: the only owning item is `ADR046-routing-016` in the sealed W2, and no post-W2 item owns the file |
| `ADR046-primitives-002` providers | `ProcessLaunchEffectPort` has no production adapter, so both process Providers are complete but unwired. The adapter is `ADR046-process-001`, destination `packages/d2b-provider-supervisor/`. | W4 |
| `ADR046-routing-014` | `ProviderInstance`'s eleven trait objects and the whole `RpcProviderProxy` family are not delivered. They are built on `d2b_contracts::v2_provider` types with no v3 replacement. Blocked on the v3 Provider-method DTO catalogue. | W3, inside `ADR046-provider-001` |
| `ADR046-routing-015` | `GeneratedProviderServiceServer` ttrpc dispatch not implemented: no v3 Provider proto, no service-name freeze, no generated bindings exist. `ProviderAgentAdapter`, `register_exact_instances`, and `ProviderAgentProcess` all depend on routing-014 surfaces that are themselves incomplete. | W3, inside `ADR046-provider-001` |

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
| `public_mint_surface` runtime | Renders rustdoc for all 47 workspace members sequentially into isolated target dirs; roughly 30 minutes and growing with every crate added. The render phase is parallelizable; the dependency ordering is only needed for the analysis phase. | Integrator decision, standalone change |
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
| Principal digest has no frozen domain tag | The cross-Zone idempotency key needs a subject digest, but the frozen digest-tag list has no principal or subject tag, so the digest is currently undomained. If a tag is later frozen, the computation changes. **Not W3**: the digest site is `packages/d2b-bus/src/zone_route.rs` (`ADR046-routing-005`, W2) and the frozen tag list is decision D101, landed by `ADR046-object-001` in W0. Both waves are sealed, and no W3 destination is either file. Freezing a new domain tag amends the decision register, which is an FR-046/FR-047 amendment path rather than wave work. | Needs an integrator ruling: a D101 register amendment, with no owning wave in the artifacts |
| No closed reason for a multi-Zone batch | The routing reason enum has no variant for a batch spanning Zones, so a structural error is returned rather than misusing an unrelated routing reason. **Not W3**: `ZoneRouteFailClosedReason` lives in `packages/d2b-contracts/src/v3/zone_routing.rs`, whose sole owning item is `ADR046-routing-001` in the sealed W2, and the refusal site `ZoneRouteError` is in `packages/d2b-bus/src/zone_route.rs` (`ADR046-routing-005`, W2). No post-W2 item names either file. | Needs an integrator ruling: no post-W2 item owns the routing reason enum |
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
