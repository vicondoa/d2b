# Implementation Debt Register

**Feature**: `001-adr046-d2b3-completion` | **Opened**: 2026-07-30

Durable record of implementation debt: stubs, unlanded
dependencies, semantics inferred where a specification was silent, code that
exists but is unreachable, and focused coverage that is not yet enforcing.

## What belongs here, and why it is separate

This register is **not** the current lifecycle's shared finding ledger.
[`deferred-findings.md`](./deferred-findings.md) is retained as historical compatibility
data and receives no findings under Constitution 3.0. This register records debt created by
building against a specification set that is not uniformly complete. Every
entry names the product owner or focused check that can close it.

Historical task identifiers, sequencing labels, manifests, and review terminology quoted
below provide context for the source records. Current ownership and closure come from
committed code, product contracts, and focused validation.

The rule that produced most of this register is worth stating, because it is
what made the debt visible rather than silent: an implementer who cannot
determine a semantic from the specification **reports the gap instead of
guessing**. A guess is indistinguishable from a decision once it is committed,
and a guess that lands in a contract propagates into every slice that consumes
it. Several entries below exist because an implementer stopped rather than
invent a wire contract, and that was the correct outcome each time.

---

## 0. Historical routing implementation state

The routing implementation record is **17 of 19 items complete, 2 partial**.
The distinction remains important because the missing work is product behavior.

- `ADR046-routing-014` is partial. The Provider registry, admission, lifecycle
  and forwarding are delivered; the eleven ProviderInstance trait objects and
  the whole RPC proxy family are not.
- `ADR046-routing-015` is partial. The conformance kit, bootstrap admission,
  audit ring, dispatch limiter and redaction are delivered; the generated
  service dispatch, the agent adapter, one-shot registration and the agent
  process are not.

Both are blocked on the same missing thing: a v3 Provider-method DTO catalogue
that exists in no crate. That catalogue belongs to `ADR046-provider-001`, whose
destination names
`packages/d2b-contracts/src/v3/provider.rs`, `packages/d2b-provider/src/lib.rs`
and `packages/d2b-provider-toolkit/` - exactly where the catalogue belongs.

The implementation delivered the catalogue only where the specification names
methods. Continuing would have meant inventing a wire contract. The remainder
is a **specification hole** and cannot be discharged until that hole is filled.
What is delivered and what is not is stated exactly in section 10.1;
discharging the remainder closes the destination caveats recorded against
`ADR046-routing-014` and `ADR046-routing-015`, and nothing else in this
register does.

The evidence rows for those two items in
`docs/specs/ADR-046-zone-routing.md` remain historical implementation evidence.
This register records the remaining technical debt.

## 1. Blocked and unlanded dependencies

These prevent a work item from being completed at all, or force it to ship a
hole. Each must be closed by the named product owner.

| Item | Debt | Product owner |
| --- | --- | --- |
| `ADR046-routing-007` | CLOSED. The dependency declarations landed as preparation and the contract module landed first, on the reading that the recorded edge is inverted for the contract. | Closed in W2 |
| `ADR046-routing-009` | Both landed, 009 first. The recorded edge remains wrong in the graph: it says 009 depends on 007, but 007 imports 009's contract. Resolve the recorded dependency against the owning contract and focused validation before relying on it for sequencing. | Not blocking |
| `ADR046-routing-016` service | Still no handler for `zone-bootstrap` and `zone-enroll`, but the blocker moved: the session contract and enrollment machine now exist in the bus, so wiring the service to them is ordinary work rather than a missing contract. The four enrollment obligations are met in the bus session module, not in the service. **Not W3**: the destination is `packages/d2b-zone-routing/src/service.rs`, which no W3 work item owns. Every work item whose destination names `packages/d2b-zone-routing/` - `routing-002`, `routing-003`, `routing-006`, `routing-016` - is W2, so the artifacts name no later owning implementation phase. **Ruled: W5, alongside `ADR046-store-004`.** See the ruling note below the table. | W5, `ADR046-store-004` |
| `ADR046-primitives-002` providers | `ProcessLaunchEffectPort` has no production adapter, so both process Providers are complete but unwired. The adapter is `ADR046-process-001`, destination `packages/d2b-provider-supervisor/`. | W4 |
| `ADR046-routing-014` | `ProviderInstance`'s eleven trait objects and the whole `RpcProviderProxy` family are not delivered. provider implementation delivered the method catalogue only for the methods the specification actually names, so the registry stays generic over the runtime's own opaque instance handle. The remainder is blocked on absent specification content, not on scheduling; see section 10.1. | W3 ownership stands, but blocked on a specification hole |
| `ADR046-routing-015` | `GeneratedProviderServiceServer` ttrpc dispatch not implemented: no v3 Provider proto, no service-name freeze, no generated bindings exist, and none can be written without a frozen service name and field numbering. `ProviderAgentAdapter`, `register_exact_instances`, and `ProviderAgentProcess` all depend on routing-014 surfaces that are themselves incomplete. | W3 ownership stands, but blocked on a specification hole |

### Ruling: the bootstrap and enroll handler belongs with durable enrollment

The `ADR046-routing-016` row above belongs with `ADR046-store-004` on a
dependency rather than on a file-ownership argument.

Two sibling rows in the historical close table below - "Sealed enrollment record does
not bind the child uid" and "No durable persistence for enrollment" - were
already resolved to W5 `ADR046-store-004`, whose destination is
`packages/d2b-resource-store-redb/src/{lib,actor,transaction}.rs`. A
`zone-enroll` handler cannot be completed without the durable enrollment
persistence those two rows describe, so scheduling the handler in any earlier
implementation phase would schedule work that cannot finish.

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

| Where | Inference | Product owner |
| --- | --- | --- |
| `v3/volume.rs` | `volumeAttachmentDefaults` entry shape is undefined everywhere in the spec set; every occurrence is an empty list. Typed as an opaque object rather than invented as `{volumeRef, view, access}`. This is the one non-strict spot in the primitive surface. | W5 |
| `v3/volume.rs` | `SensitivityClass` admitted as `public | private | secret`; only `private` is attested by spec text. | W5 |
| `v3/volume.rs`, `v3/process.rs` | `RepairPolicy`, `CleanupPolicy`, `AdoptionPolicy`, `EntryRestartPolicy`, `LeaseClass` and `Invariant` value sets are only exemplified in YAML samples, never enumerated. Only attested values are admitted. The Volume providers had to map onto this narrower set, using `exact-mode` for the TPM root and carrying `same-filesystem` as a drift class rather than an invariant. | W5 |
| `v3/network.rs` | MTU bound `576..=9216` inferred; spec states the default but no range. | W5 |
| `v3/credential.rs` | `ExpirySpec.hardDeadlineMs` additionally capped at the maximum lease lifetime, since a deadline beyond the longest possible lease is unreachable. Not a stated rule. | W5 |
| `v3/zone_routing.rs` | The v3 capability catalogue is not enumerated anywhere. Modelled as a bounded token set with subset ordering rather than a frozen enum, because freezing it would have invented a wire contract four slices consume. | W2, before the routing contract is treated as frozen |
| `v3/zone_routing.rs` | Zone label bound narrowed from the baseline 128 bytes to 63, on the reading that a Zone label is a Zone resource name. Deliberate narrowing. | W2 |
| `v3/zone_routing.rs` | The private allocated-capability field shape is an interpretation: modelled as non-serialized state attachable only through a narrowing constructor. | focused contract review |
| `zone_routing.rs` resolver | The local root may not absorb an unmatched target. Keeping the baseline's ancestor-coverage rule verbatim would make the resolver unconditionally permissive, since the local root is a suffix of every in-scope path. Non-root ancestors still cover descendants. | focused contract review |
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
| Unknown-spec-field rejection | Cannot be enforced while the shared `spec` type injects execution-policy defaults into every resource. Needs the generated per-type submodule to replace the freeform type, which requires editing a file the generator slice does not own. `nix-unit: zone-link-closed-spec` cannot pass until then. | W2 |
| Two engine refusal branches unreachable from outside | The contract constructors already reject the shapes that would trip them, so they guard only the deserialization path. Exercising them needs a deserialization-based vector, a different surface than the vector suite owns. | focused contract review to rule |
| Enrollment validation obligations | Four obligations unmet, blocked on the session contract. | W2, with routing-007/009 |
| `UNIMPLEMENTED_SCAFFOLD` markers | Still present in several crates, deliberately, because the capability gate fails closed on a crate advertising no public item. Each must be deleted by the slice that fills its crate. | Per slice |


### Discharged during routing implementation

Recorded as closed rather than deleted, so a reader can tell the difference
between debt that was paid and debt that was never real.

- The drift check now runs both Zone generators and compares
  `nixos-modules/generated/`, so the header those artifacts carry promising
  byte-for-byte comparison is now true rather than aspirational.

## 5. Specification drift found while implementing

Recorded so source contracts and focused tests can preserve the existing-code
decision where documentation is stale.

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
- **Provider implementation destinations differ in historical prose.**
  The committed Provider contracts, toolkit, semantic-service, and system-core
  paths are the implementation boundaries.
- **Zone and Volume assertion messages rewritten for FR-017 actionability.**
  The assertion messages named the violated rule but no concrete operator
  action. The shipped code in
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

## 6. Historical record-handling note

`FR-047` is defined in this feature's `spec.md` and remains the architectural
consistency requirement. Historical dispatch, registry, and review mechanics
are not implementation authority; current changes follow the owning contract
and focused evidence.

---

## Added after the routing implementation

New debt discovered while completing the implementation phase's last five items.

| Debt | Detail | Product owner |
| --- | --- | --- |
| Appended Zone tags cannot reach the wire | The session contract appends six Zone members at new tags, but the canonical handshake offer encoder types its fields with the un-extended enums and lives in a file no W2 slice owned. Widening it in place would have invalidated the committed golden vectors. Enrolled links and bootstrap use preserved tags, so ZoneLink is unaffected; carrying an appended tag needs an owned decision on the offer encoding. **Not W3**: the encoder is `packages/d2b-session/src/handshake.rs`, and no W3 work item names `packages/d2b-session/`. `ADR046-exec-018` (W5) owns the v3 session wire types and re-types `EndpointPolicy` - `encode_offer`'s sole input - onto v3 `ZoneId`/`ProviderId`, and `ADR046-reuse-001` (W5) owns the v3 contract extension to that crate. | W5, `ADR046-exec-018` |
| Session tag values are an inference | No specification fixes the numeric tags for the six appended members, nor the ZoneLink service wire string. The scheme chosen is append-only, next unused tag, never renumber, with two tags permanently reserved rather than reused. These are wire-visible and need focused contract confirmation before anything depends on them. | session contract owner |
| Subject-digest prologue field is a choice | The specs name no field for the subject-context digest. It is folded into the existing channel binding, which is already inside the canonical offer and therefore inside the handshake prologue, so no wire change was needed. Worth confirming in focused contract tests. | bus contract owner |
| Sealed enrollment record does not bind the child uid | The spec says the record binds the child static key pin to the child Zone uid. The session module holds the fingerprint and an opaque allocator-binding digest; the uid binding belongs to the durable store transaction owned by the ZoneLink controller. **Not W3**: no W3 work item owns `packages/d2b-session/` or `packages/d2b-core-controller/`. The transaction is `ADR046-store-004`, destination `packages/d2b-resource-store-redb/src/transaction.rs`, which the graph places in W5; the controller-side write lands in `zone_links.rs` behind it. | W5, `ADR046-store-004` |
| No durable persistence for enrollment | Recovery takes the persisted facts as arguments. The store transaction that seals or invalidates a record is the controller's and is not implemented. **Not W3**: same determination as the row above - the durable store backend and its transaction module are `ADR046-store-004` in W5, and no W3 destination is a store or controller path. | W5, `ADR046-store-004` |
| `component_session` runtime tests not duplicated | The bus re-exports the session runtime rather than forking it, so its 2,121 lines of tests were deliberately not copied; they run in the owning crate. The ported golden vectors are the port evidence. A scope judgement worth focused test confirmation. | session contract owner |
| Principal digest has no frozen domain tag | The cross-Zone idempotency key needs a subject digest, but the frozen digest-tag list has no principal or subject tag, so the digest is currently undomained. If a tag is later frozen, the computation changes. The digest site is `packages/d2b-bus/src/zone_route.rs` and the frozen tag list is decision D101. The contract gap is recorded in [`amendment-frozen-cross-zone-contracts.md`](./amendment-frozen-cross-zone-contracts.md). | Cross-Zone contract owner, before transport enablement |
| No closed reason for a multi-Zone batch | The routing reason enum has no variant for a batch spanning Zones, so a structural error is returned rather than misusing an unrelated routing reason. `ZoneRouteFailClosedReason` lives in `packages/d2b-contracts/src/v3/zone_routing.rs`, and the refusal site is in `packages/d2b-bus/src/zone_route.rs`. The contract gap is recorded in [`amendment-frozen-cross-zone-contracts.md`](./amendment-frozen-cross-zone-contracts.md). | Cross-Zone contract owner, before transport enablement |
| Unix session tests delegated rather than ported | The historical plan asked to port the unix session tests verbatim, but the owning crate is consumed as a dependency instead, so copying them would fork the audited substrate. Zone-level semantics were ported instead. If coverage is insufficient, add the syscall dev-dependency and port the missing cases. | session contract owner |
| Listener portal transport variant | One spec describes a pre-bound socket handed over a portal call while another describes an inherited connected socket. Only the connected form is implemented; the portal wire contract belongs to a transport Provider crate and is unspecified for the bus. | W6 |

## Provider-crate layout and naming policy: scope and exemptions

The crate-layout and naming policy - the mandatory `src/`, `tests/`,
`integration/`, `README.md` shape and the naming rule that `ADR046-provider-002`
carries - scopes to crates matching `d2b-provider-<base>-<implementation>`.

Two existing crates are **exempt**:

| Crate | Reason for exemption | Product owner for retiring the exemption |
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

## 7. Known flakes observed during routing implementation

Recorded so a later run does not rediscover them as regressions.

| Test | Observation | Assessment |
| --- | --- | --- |
| `d2b-unsafe-local-helper::shell_supervisor real_supervisor_preserves_pty_across_reconnect_and_kills_exact_scope` | Failed once with "supervisor did not exit" during a full parallel run, passed 3 of 3 in isolation, and passed on the next full run of all 4499 tests. | Environment-sensitive, not a code regression. The test spawns a real supervisor in a transient scope and waits for it to exit, which is timing-sensitive under heavy parallel load. Worth a bounded wait rather than an unbounded one if it recurs. |
| Capability seal fixtures | Two seal fixtures reported a downstream compile failure that read as a trust-boundary regression. The real cause was a stale fixture lock after the bus gained dependencies. | Fixed. Worth knowing that this failure mode is indistinguishable from a genuine seal break in its message, so a stale lock should be ruled out first. |
| Compile-fail tests under a caching compiler wrapper | A capability seal failed once with a wrapper client exiting nonzero under concurrent cargo invocations. | Mitigated earlier in the program by clearing every wrapper spelling for those spawned compilers. The original failure was never reproduced on demand, so that remains a reasoned mitigation rather than a demonstrated fix. |

## 8. Validation obligations not yet met

These obligations must be explicit rather than inferred: an unmet obligation is
acceptable when it is written down, but not when a product surface ships
without the corresponding evidence. Each names the work item that owes it.

This section was rebuilt by reading the `validation` field of the affected work
items against the tests that actually exist. The three Zone configuration
entries below are unchanged; what follows them is the remainder of that audit.

The three Zone configuration items carry obligations at test layers this implementation phase
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
non-deterministic bundle, and nothing else in the implementation phase covers that. The
generators are deterministic by construction and the drift gate now compares
their output, but neither proves determinism across two independent
evaluations of the same input.

### The rest of the audit

Six further obligations are owed by four other items. Each is recorded with
the honest reason it is not met and the thing that would discharge it.

| Work item | Obligation | Why it is not met | What would discharge it |
| --- | --- | --- | --- |
| `ADR046-primitives-002` | Host/Guest/user integration, alongside the shared conformance suite | Both Provider conformance suites run only against `ScriptedEffectPort`, a hermetic mock. A scripted collaborator proves the Provider's own decision logic and proves nothing about a real Host, Guest, or user domain, and the crates are imported by no production caller. | The `ProcessLaunchEffectPort` production adapter, then the same conformance obligations re-run against it across the three domains. The adapter is `ADR046-process-001` in W4. |
| `ADR046-primitives-003` | virtiofs host/guest mount tests | `export_lifecycle.rs` drives `VirtiofsExportController` through a `ScriptedPort`, so "the host serves and the guest mounts" is asserted as a controller state transition, not as a mount. Nothing in the implementation phase mounts anything. | A host-integration check that exports a Volume and mounts it in a booted guest. Blocked on the same absent effect adapter. |
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
the implementation phase-close table above. They are not repeated here.

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

## 9. Provider implementation debt and scope

Two items are in scope and one named group is explicitly not.

### In scope

**1. The v3 Provider-method DTO catalogue.** Owned by
`ADR046-provider-001` and stated in section 0. Discharging it closes the
Destination caveats recorded against `ADR046-routing-014` and
`ADR046-routing-015` in section 1, which are the two partial items in the
routing implementation delivery claim. Nothing else in the register turns those two rows from
partial to complete.

**2. A build-level determinism check for the generated Nix
catalog.** Section 8 records that the flake-check layer - proving a generator
emits identical output across two independent evaluations - is the one thing
neither construction-time tests nor the drift check cover. The drift gate
compares a generator's output against the committed tree, which catches a
generator that changed; it does not catch a generator that emits different
bytes on two runs of the same input. The implementation ships a new generator
(`ADR046-provider-002`'s Provider package and catalog emitter), so it must not
repeat the gap it can see in the implementation phase before it.

### Not in scope

**3. The missing nix-unit eval cases and host-integration checks recorded
against `ADR046-routing-011`, `ADR046-routing-012`, `ADR046-routing-013` and
`ADR046-primitives-003`.** These stay where section 8 puts them.

Two reasons, and the first is sufficient on its own. Every one of those
obligations already carries a recorded owner; duplicating them here would give
the same obligation two owners. Second, several cannot execute yet regardless
of who owns them: section 3 records that `resources-volume.nix` and
`options-zones.nix` are imported by no module, and section 8 records that the
host-integration obligations need both those imports and a production store,
all owned by later waves. Writing them in provider implementation would produce checks that do
not run.

The implementation does owe them the pattern. Item 3 above establishes the
build-level determinism check on the generator; the
`ADR046-routing-012` determinism obligations follow that shape when their implementation phase
lands, rather than each implementation phase re-deciding what a determinism check looks like.

---

## 10. Added after Provider implementation

Debt reported by the implementation slices, each verified against the tree
before it was recorded rather than transcribed from the slice's own account.
Where verification disagreed with a slice, the tree is recorded and the
disagreement is stated.

The three categories the register already separates are kept apart here,
because the distinction is what makes the entry actionable: an **unmet
obligation** names work someone still owes, an **inference** names a reading a
reviewer must confirm or correct, and a **specification correction** names a
place where shipped code and written specification disagree and code was kept.

### 10.1 Unmet obligations

| Debt | Detail | Product owner |
| --- | --- | --- |
| v3 Provider-method DTO catalogue, remainder | Partly delivered, and the rest is blocked on specification content that does not exist. Verified in `packages/d2b-contracts/src/v3/provider.rs`: method names exist for **one** of the eleven Provider families - the Transport triple `openTransport`/`closeTransport`/`observeTransport` and the controller currency triple `assessUpdate`/`planUpgrade`/`executeUpgrade` - and nothing is named for display, clipboard, notification, shell-terminal, credential, device, volume or network. **No request or response payload is written anywhere for any method**, including the six that are named: `openTransport` is described as returning an opaque handle and observations whose shape appears in no document. There is no proto, no frozen service name and no field numbering, which is exactly what `ADR046-routing-015` needs to freeze. Consequence, confirmed in the tree: `packages/d2b-provider/src/identity.rs` and `src/lib.rs` name the `ProviderInstance` sum type and the `RpcProviderProxy` payload only in prose, no such type exists, and the registry stays generic over the runtime's own opaque instance handle. | `ADR046-provider-001` retains ownership. **What would unblock it**: a specification amendment that (a) names the methods of the remaining ten families, (b) writes the request and response payload for every method including the six already named, and (c) freezes a service name and field numbering. Until that lands, no implementation phase can discharge it, and scheduling it into a later implementation phase would only move the same hole |
| Catalog / Provider-manifest parity test | **DISCHARGED in `e15f88cc`, after this row was written.** The row's finding stands as history: the offline Nix catalog (`nixos-modules/generated/provider-catalog-shape.nix`, 25 fields) and the `ProviderManifest` DTO (`packages/d2b-contracts/src/v3/provider.rs`) described the same Provider facts in two places with nothing comparing them, and the packaging slice's deferral to "whichever lands second" was void because both landed inside provider implementation, in `56196815` and `753e1e63`. What discharges it: `the_catalog_shape_and_the_provider_contract_describe_the_same_fields` in `packages/xtask/src/provider_packaging.rs`, which compares the generated catalog's flattened field set against the fields the contract structs declare. `xtask` is a member of `packages/Cargo.toml`, so the test runs enforcing under `make test-rust`. The divergences it found are not resolved by its existence - they are pinned as exact data in the test and read out in section 12.4, so resolving any of them fails the test until the register entry is struck in the same change | Closed in W3, `ADR046-provider-002` |
| Two conformance cells duplicated per Provider crate | `packages/d2b-provider-system-{systemd,minijail}/tests/execution_parents.rs` are near-identical files, differing only in the provider type and one test name. Two cells belong in the shared suite: execution-parent neutrality and the disagreeing wait owner. The slice named them `assert_execution_parent_is_neutral` and `assert_a_disagreeing_wait_owner_quarantines`; **those are proposed suite-helper names, not names in the tree** - the cells are currently spelled `a_non_host_execution_parent_yields_the_same_status_shape` and `a_candidate_whose_wait_owner_disagrees_is_quarantined` in both crates. The suite is `packages/d2b-process-conformance/src/suite.rs`, which no provider implementation slice owned | The implementation phase that next owns `packages/d2b-process-conformance/` |
| No Provider dossier parity check | **DISCHARGED in `e15f88cc`.** See section 11; recorded there with the rest of the audit, and with exactly what discharges it | Closed in W3, `ADR046-provider-002` |
| `d2b-provider-toolkit` has no `[dev-dependencies]` | Confirmed: its `Cargo.toml` declares one dependency and no dev-dependency table, so its integration tests cannot use `serde_json`. Malformed-wire rejection is therefore proved in the contracts module rather than against the toolkit's own fakes. Not wrong, but the coverage sits one crate away from the surface it describes | W3 or any later slice touching the toolkit; a two-line manifest change |
| `SchemaVersion` has no component accessor | Confirmed: `packages/d2b-contracts/src/v3/resource_schema.rs` exposes no `major()` or `minor()`, so `schema_version_parts` in `provider.rs` renders the type's own canonical string and parses it back. The comment in that function states the reasoning, and it is sound - the canonical spelling is the contract's own round trip, so the parse is exact rather than lossy. It remains indirect, and a one-line accessor would remove the parse and its two `expect` calls | Cosmetic. Any later slice owning `resource_schema.rs` |

### 10.2 Semantics inferred where the specification is silent

| Where | Inference | Product owner |
| --- | --- | --- |
| `v3/provider.rs` | The D089 standard capability matrix is modelled as a bounded token map (`StandardCapabilityMatrix(BTreeMap<BoundedToken, CapabilitySupport>)`) rather than a frozen enum, because the specification mandates the matrix and names the classes it covers but never enumerates the optional-capability identifiers. Absence fails closed: a capability the matrix does not list is unsupported rather than assumed. This is the same shape, and the same reason, as the v3 capability catalogue already recorded against `v3/zone_routing.rs` in section 2 | W3, before the Provider capability surface is treated as frozen |
| `nixos-modules/generated/provider-catalog-shape.nix` | The catalog's **25 field names** are an inference. The specification's bullets name concepts, not identifiers, so the field **set** is the specification's and the **names** are not. The file records the concept-to-field mapping explicitly in its `fieldGroups`, which is the right shape for a later reviewer to check the inference rather than rediscover it | W3, confirmed by the parity test recorded in 10.1 |

### 10.3 Specification drift found while implementing

Recorded as implementation-versus-contract drift. Historical entries are not
edited to claim a correction; current documentation follows committed code,
the owning product contract, and focused validation.

- **Provider crate-layout policy file name and lane.**
  `docs/specs/ADR-046-resources-zone-control.md` section 4.8.2, the
  `ADR046-pkg-001` Destination, and the matching historical task records and
  `docs/specs/providers/ADR-046-provider-activation-nixos.md` all name
  `packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs`, routed
  through the **advisory** `test-fixture-contracts` lane. The slice shipped
  `packages/d2b-contract-tests/tests/policy_provider_crates.rs`, wired into the
  **enforcing** hermetic `test-policy` lane at `tests/test-policy.sh`. The
  reasoning is sound and is kept: the check is filesystem-only and compiles
  nothing, so it is hermetic, and a hermetic check belongs in an enforcing lane
  rather than one that is advisory until fixture delivery is wired. Code is
  canon. The historical register is not corrected in place; any prose change
  belongs in its owning contract with focused validation, alongside the
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
  actually reused pid. **Owner: `ADR046-process-001`, historically assigned to
  `W4`** - read from that item's retained task record and implementation-phase
  note. This agrees with the `ADR046-primitives-002` row already in section 1.
- `UserDiscoveryEffectPort` (`packages/d2b-provider-system-core/src/user.rs`) is
  **new surface introduced by that crate**, not something the Provider
  catalogue defined. If the catalogue later defines an equivalent discovery
  port, it should supersede this one rather than sit beside it, or the same
  concept acquires two incompatible spellings.

### 10.5 Already recorded, verified not duplicated

`packages/d2b-provider-aca` and `packages/d2b-provider-relay` remain
non-conformant to the crate layout and exempt by name. This was already ruled
and recorded above under "provider implementation provider-crate layout and naming policy",
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

## 11. Validation obligations not met by provider implementation

Built the way section 8 was rebuilt: by reading the `validation` field of every
one of the implementation phase's four work items against the tests that actually exist, rather
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
| `ADR046-provider-002` | Workspace **dossier** parity policy | **Met, in `e15f88cc`, after this row first recorded it as unmet with no partial coverage at all.** Discharged by `check_dossier_parity` in `packages/d2b-contract-tests/tests/policy_provider_crates.rs`, driven on the real tree by `every_provider_crate_has_a_dossier_declaring_the_same_identity`. Three things about its shape are worth stating so a reader can verify it without re-deriving it. **Direction**: the check is crate-driven, and deliberately not symmetric. A dossier with no crate is legitimate, because the dossier set is the frozen initial Provider catalog and later waves implement against dossiers that already exist; a crate with no dossier is the failure. **Identity correspondence**: it does not merely pair a crate with a file. It resolves the crate name to the identity it denotes, requires `docs/specs/providers/ADR-046-provider-<identity>.md` to exist, and requires that dossier to carry exactly one `Spec ID` table row declaring that same identity - so a crate paired with the wrong dossier, a dossier with no `Spec ID` row, and a dossier with two are each rejected. **Anti-vacuity**: `a_dossier_without_a_crate_is_not_a_violation` asserts against the real tree that at least one crate-less dossier actually exists before it asserts that such dossiers are reported by nobody, so the asymmetry is proved over a populated case rather than over an empty one; `the_dossier_directory_holds_the_frozen_provider_catalog` separately pins that the directory the check reads is real and populated. The two recorded exemptions are unchanged and still pinned by `the_two_recorded_exemptions_are_exactly_the_naming_mismatches` | - |
| `ADR046-provider-002` | **Catalog parity** policy | **Met, in `e15f88cc`, after this row first recorded it as unmet.** Discharged by `the_catalog_shape_and_the_provider_contract_describe_the_same_fields` in `packages/xtask/src/provider_packaging.rs`, an enforcing `make test-rust` surface. The obligation is to compare the two descriptions, and that is met; the divergences the comparison found are a separate open item, pinned as exact data in the test and recorded in 12.4 | - |
| `ADR046-provider-003` | Shared conformance tests | **Met at the logic layer, unproven at every real boundary.** The shared suite in `packages/d2b-process-conformance/src/suite.rs` runs against both Providers, and two further cells are duplicated per crate rather than shared (10.1). Every cell runs over `ScriptedEffectPort` | `ADR046-process-001` in W4, then the same suite re-run against the production adapter |
| `ADR046-provider-003` | Host / user / non-Host tests | **Met at the logic layer.** `tests/host_reconciliation.rs`, `tests/user_discovery.rs` and both `tests/execution_parents.rs` cover the three cases, over injected ports only | As above |
| `ADR046-provider-004` | Shared semantic Service / Binding contract tests, and generated schema artifacts for the eight exact qualified ResourceTypes | **Audited in full; see section 13.** This row previously read "Not assessable here" and deferred to "the semantic-services slice's own audit". That deferral had no owner and nothing behind it, so it is retired rather than left standing. Summary of the result recorded in section 13: eleven of the sixteen enumerated obligations are met, four are met only at a weaker level than the phrase implies, one is met, and the item ships two live caveats - a telemetry Binding whose common status layer is empty and rejects everything, and a security-key family that cannot construct a signed projection factory at all - which were recorded only in Rust source comments and pinned by tests until now | Section 13, and the two amendments it names |

Two further observations from the audit, neither of which is an obligation.

The implementation phase's four items are unusually asymmetric in how much their `validation`
fields commit to. `ADR046-provider-003` names two things in six words;
`ADR046-provider-004` enumerates around fourteen. A short validation field is
not a weaker obligation - it is a less legible one, and 10.4 is what a
six-word field looks like when it is discharged only over scripted ports and
nothing in the field says it must not be.

The single largest gap this implementation phase leaves is not any one of the entries above.
It is that **three crates, one Nix catalog and one Provider contract are all
proven only against each other**. Every provider implementation deliverable is hermetic by
construction, every one is unwired, and the first evidence that any of it
matches a real system arrives with `ADR046-process-001` in W4. That is the
correct sequencing and it was chosen deliberately, but it means a green
provider implementation gate is evidence about internal consistency and not about behaviour.

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
**`ADR046-zone-control-015`**, implementation phase **W5**. Determined by
reading the retained task records rather than inferring: the item's
`destination` is
`packages/d2b-resource-compiler/src/{main,bundle,schema,validator,digest,sort,secret_lint,generation}.rs`
exposed as `pkgs.d2b-resource-compiler`; its `detailedDesign` opens "Implement
all Phase 2 build-time checks (§14.10 Phase 2 table)" and states explicitly that
for each `d2b.artifacts.*` entry the compiler must "extract and hash manifest and
config schema files"; its `validation` field enumerates the section 15.8 Phase 2
build tests. The implementation phase is recorded as `W5`. The implementation
state is `Planned` and no `packages/d2b-resource-compiler/`
exists in the tree, so nothing about this obligation is discharged anywhere
today.

**What the output term therefore is, and its state.** A cardinality rule over
the workspace: one Provider crate yields one Nix package output, not several.
Its state is recorded in 12.2 immediately below, because attempting to check it
is what exposed the gap.

### 12.2 Specification gap: the required derivation outputs have no path, name, or layout

**Disposition: closed by accepted ADR 0050 and its applied amendment.** The
historical audit below records the pre-amendment state. Section 4.9 of
`ADR-046-resources-zone-control.md` now fixes the output-selection predicate,
the complete relative layout, the executable set, digest preimages, and the
failure taxonomy.

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

**This needs a specification amendment, not a implementation phase.** The amendment must fix,
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

**Disposition: closed by the applied amendment.** The section 15.8 Phase 1,
Phase 2, and Phase 3 tables now carry the named Provider artifact and launcher
scenarios, and work items 015 and 016 cite them.

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

| Finding | Class | Product owner |
| --- | --- | --- |
| Output obligation mislocated to `provider-catalog.nix`; the derivation-contents rule is `ADR046-zone-control-015` | Correction to this register | Recorded, no implementation phase work |
| Required derivation outputs have no path, filename, output name, or layout | Specification correction | Closed by accepted ADR 0050 and its applied amendment |
| Output cardinality not checkable: no Provider crate has a package output, so the crate-to-output relation does not exist in the tree | Unmet obligation, blocked | Closed by the section 4.9.1 artifact-entry predicate |
| `d2b.artifacts.<id>.package` typed `types.package` already enforces the cardinality at the one entry point | Inference, needs confirm or reject | focused contract review |
| Required-outputs row has no conformance scenario in the section 15.8 Phase 2 table | Specification correction | Closed by the applied amendment |
| Catalog names component and descriptor digests; contract names exported schema and service digests; section 4.3.1 names neither | Ruling needed, three-way | Amendment, before the Provider packaging surface is treated as frozen |
| Five catalog facts absent from the manifest; one contract field absent from the catalog | Unmet obligation, pinned as data | `ADR046-provider-002`, closes when the ruling above lands |

## 13. The `ADR046-provider-004` audit, performed

Section 11's row for `ADR046-provider-004` was a placeholder. It said the
common semantic Service and Binding catalog was "not assessable here" and
deferred the assessment to "the semantic-services slice's own audit". No such
audit was performed and the deferral named no owner, so for the interval
between `70eb17a4` and this section the register carried a row that recorded
neither a pass nor a failure for the largest validation field in the implementation phase.
That is worse than an unmet obligation, because a reader cannot tell from it
whether anything is owed.

This section performs that audit the same way sections 8 and 11 were built: by
reading the item's `validation` field in `historical task record`
clause by clause against the tests and code that exist in
`packages/d2b-contracts/src/v3/semantic_services/` and the generated artifacts
in `docs/reference/schemas/v3/`, rather than by transcribing a slice's account
of its own work.

One note on method. The Provider-neutrality proof this item leans on was
rewritten in `c4e89e26` after a focused review reviewer found the original vacuous - two
of its assertions compared a value with itself, a third compared a clone with
its original, and its byte comparison built the schema contract once, outside
the loop, with no Provider installed. Everything below audits the **current**
test, not the one `70eb17a4` shipped.

### 13.1 Obligation by obligation

The `validation` field is one sentence with fourteen comma-separated clauses
plus a closing sentence, and the item's `destination` carries a sixteenth
obligation about generated artifacts. Each is taken separately.

| # | Obligation, as the field words it | State | What discharges it, or which part is missing |
| --- | --- | --- | --- |
| 1 | Exact names | **Met.** | `the_catalog_names_exactly_the_eight_frozen_resource_types` in `mod.rs` pins the sorted list of all eight dot-qualified types against a literal. `schema_identities_use_the_slash_form_and_the_api_type_uses_the_dot_form` separately pins that the schema identity is `<namespace>/<Type>/{spec,status}` and asserts the dot-qualified infix is absent from it, so the two spellings cannot be conflated. Each family module repeats the pair in `the_pair_names_the_exact_frozen_resource_types` against its own `*_RESOURCE_TYPE` constant |
| 2 | Strict serde / schema round trips | **Met for `spec`, absent for `status` and for the projection.** `assert_minimal_base_round_trips` serializes the minimal base `ResourceSpec`, deserializes it, compares canonical bytes, and re-validates the decoded value, and every one of the eight members runs it. Nothing round-trips a `status` layer or a projection spec through serde: the status layers are exercised only as field-name sets through `SemanticLayerSchema::validate_names`, and `validate_projection_spec` is driven with hand-built `ResourceSpec` values that are never encoded. The clause says "round trips" without restricting the layer | The two missing layers, or an explicit ruling that the spec layer is the whole obligation |
| 3 | Common base discoverability without any Provider package | **Met.** | `every_base_contract_builds_with_no_provider_installed` builds `schema_contract(std::iter::empty())` for all eight members and asserts the resulting contract's ResourceType, a `sha256:`-prefixed fingerprint, and version `1.0`. The catalog is a `OnceLock` per family behind `pub fn contract()`, reachable from `d2b-contracts` with no Provider crate in the dependency graph, which is the structural half of the same claim |
| 4 | Canonical minimal base acceptance without `spec.provider` | **Met.** | Each family's `the_canonical_minimal_base_is_accepted_without_a_provider_extension` runs `minimal_base_spec` for both members and asserts `spec.provider().is_none()` before `validate_minimal_base_spec` admits it. `minimal_base_spec` itself is the enforcing half: it rejects a fixture that supplies `provider`, `providerRef`, or `updatePolicy` (`MinimalBaseReservedField`) and rejects any fixture whose field set is not exactly the required set minus `providerRef` (`MinimalBaseFieldSetMismatch`), so the fixture cannot drift into being minimal in name only |
| 5 | Same-Zone refs / targets | **Met only at the type half. The same-Zone half is not implemented and not tested.** | `admit_binding_refs` checks two things: that `serviceRef` names this pair's Service ResourceType, and that the target is in the family's closed `BindingTargetType` set. Its own doc comment states the rest plainly - "Both must be same-Zone, which the caller establishes by resolving them in the Binding's Zone before calling" - so the Zone predicate is delegated to a caller that does not exist yet. The one test, `binding_refs_and_targets_are_admitted_against_the_frozen_sets`, covers audio only, with one accepted target, one rejected target, and one foreign Service type; the other three families' target sets are unexercised. **This is the largest gap in the item and it is invisible from the test names**, because a test called "same-Zone refs and targets are admitted against the frozen sets" reads as though it checked Zones |
| 6 | Owner versus projection discrimination | **Met.** | Structurally by `the_projection_field_set_is_a_strict_subset_of_the_service_base`, which asserts subset **and** strict inequality of cardinality for all four families, so a projection cannot silently become the owner base. Behaviourally by one negative case per family naming that family's owner-only fields: audio rejects `authority`, telemetry rejects `ingestEndpointRefs` and `authorityDescriptor`, USB rejects `backingDeviceRef` and `backingAuthority`, security-key rejects `authority` |
| 7 | Core projection rejection of `spec.provider` | **Met.** | `validate_projection_spec` returns `ProjectionProviderExtensionForbidden` before any field-name work, and `a_core_projection_rejects_a_provider_extension` drives it with a real `ProviderSpecExtension`. The check is one shared code path, so exercising it on one family is exercising it on four |
| 8 | Common fields only under `status.resource` | **Met at the registration boundary.** | `a_provider_status_extension_may_not_shadow_a_common_status_field` registers a USB Provider whose `status_details` declares `access`, which the USB Service common status layer already carries, and asserts `schema_contract` **fails**. The enforcement lives in `validate_provider_registration` in `resource_schema.rs`, which returns `ProviderFieldShadowsBase`. `a_pipewire_observation_is_not_a_common_status_field` adds the positive/negative pair on the audio Service layer |
| 9 | Implementation observation only under `status.provider` | **Met at the same boundary, and only there.** | Same shadow rejection as obligation 8 read from the other side. No test in this module drives `ResourceSchemaContract::validate_envelope`, which is where the full three-layer `status.resource` / `status.provider` split is actually enforced, so the layering is proved over field-name sets and registration rather than over a populated envelope |
| 10 | Status-only observations | **Met for two families, structurally impossible for a third, absent for the fourth.** | Security-key's `attachment_is_a_status_field_and_not_a_binding_spec_field` and USB's `the_attachment_phase_is_status_only` each assert the field is accepted by the status layer and rejected by the spec layer, which is the shape the clause wants. Audio has no such pair for its Binding. Telemetry cannot have one: its Binding common status layer is **empty**, so there is no observation to prove is status-only, and its test asserts exactly that instead. See 13.2 |
| 11 | No Device / Endpoint / Binding projection | **Met, though one of its two tests is close to vacuous.** | The discriminating one is `an_export_targets_only_the_owner_service`: for every family it accepts a `ResourceExport.resourceRef` naming the Service and rejects `Device`, `Endpoint`, and that family's own `*Binding`. The other, `a_projection_is_the_same_qualified_service_type_and_never_another_type`, asserts among other things that `audio.d2bus.org.AudioService` is not the string `"Device"`, which cannot fail; its useful content is that the projection's service type equals the pair's Service type and differs from the Binding type |
| 12 | Implementation-detail rejection | **Met.** | Catalog-wide by `an_implementation_detail_is_rejected_from_every_base_spec`, which pushes `pipeWireNodeAlias` at all eight members. Per family with details that family would plausibly have absorbed: audio `captureAlias`, telemetry `backend` / `ingestProtocol` / `backendEndpointRefs`, USB `busid` / `networkRef` / `relayEndpointRef` / `sysfsPath`, security-key `deviceRef` |
| 13 | Semantic factory-fingerprint stability under Provider / adapter identity changes | **Met, by `assert_base_is_provider_neutral` and not by the test whose name claims it.** | `the_stored_factory_fingerprint_is_rederivable_from_the_public_inputs` recomputes `factory_fingerprint` from the identical inputs and asserts equality; that is a purity check on a pure function and it would pass however Provider-dependent the catalog were. What genuinely discharges the clause is `assert_base_is_provider_neutral`, run by all four families. It installs two **different** Provider extension registrations, each with its own settings field name, captures the whole Provider-observable surface under each - both schema identities, both versions, all four frozen field sets, both base fingerprints, the projection schema fingerprint, the recomputed factory fingerprint, the canonical bytes of the identical minimal fixture, and a probe map of the contract's enforced accept/reject outcome for every candidate field plus and every present field minus - and requires the two observations to be equal. It carries two negative controls: the two installed contracts must differ from each other, and both fingerprint functions must move when a declared input moves. Structurally, `factory_fingerprint` takes no Provider or adapter argument, which is why the equality holds |
| 14 | Rejection of every implementation-qualified and former `*State` alias | **Met at a weaker level than "rejection".** | `no_implementation_qualified_or_state_alias_is_registered` builds the set of eight registered types and asserts nine aliases - `audio-pipewire.d2bus.org.AudioService`, `audio.d2bus.org.AudioState`, `device-security-key.*`, `observability-otel.*`, `device-usbip.*` and the rest - are not members. That proves **non-registration**, which is what the module can prove: there is no registry here to reject a lookup, and the module doc records that these eight types are deliberately absent from the closed standard ResourceType registry. An alias presented to a real resolver is rejected by nothing this item ships | The resolver-side rejection, whenever the Zone store admits installed Provider schemas |
| 15 | Each initial and fake alternate Provider must pass the identical base conformance fixture | **Met.** | Each family runs `every_implementation_passes_the_identical_base_fixture` with its real initial Provider name and an invented alternate: `audio-pipewire` / `audio-alternate`, `device-security-key` / `security-key-alternate`, `observability-otel` / `telemetry-alternate`, `device-usbip` / `usb-alternate`. The fixture string is one constant passed to both, so "identical" is enforced by construction rather than by two fixtures that happen to agree |
| 16 | Destination: generated schema artifacts for the eight exact qualified ResourceTypes | **Met, and over-delivered.** | `docs/reference/schemas/v3/` holds 20 semantic artifacts: `_spec` and `_status` for each of the eight types, plus one `_projection_spec` per family. They are generated by `packages/xtask/src/semantic_service_schemas.rs` from the catalog itself as single source, and regeneration is gated by `run_xtask gen-semantic-service-schemas` in `tests/unit/gates/drift-check.sh`, which is the **enforcing** `make test-drift` lane. Each carries `additionalProperties: false`, the frozen `properties`/`required` sets, and `x-d2b-*` extensions pinning the ResourceType, schema version and fingerprint; the projection artifacts additionally pin `x-d2b-allowed-backing-ref-types`, the Binding type, and both fingerprints |

Counted as the field words them: eleven met, four met only at a weaker level
than the clause states (2, 5, 9, 14), and one met for two of four families with
a recorded reason for the other two (10). Nothing in the field is wholly unmet.

### 13.2 Telemetry's Binding common status layer is empty and rejects everything

Recorded here because it was visible only in a Rust module comment and pinned
by a single test whose name states it, which is not where a reader of this
register would look for it.

**What the tree does.** `BINDING_STATUS_ALLOWED` in
`packages/d2b-contracts/src/v3/semantic_services/telemetry.rs` is the empty
slice. The layer's required set is empty too, so
`contract().binding().status().validate_names([])` succeeds and every non-empty
name set fails with `SemanticContractError::SchemaViolation`. The generated
`telemetry.d2bus.org_TelemetryBinding_status.schema.json` says the same thing in
the shipped artifact: `"properties": {}`, `"required": []`,
`"additionalProperties": false`. `the_binding_common_status_layer_is_closed_pending_frozen_names`
pins both halves, using `stamped` as the rejected probe.

**The consequence, stated plainly.** A controller reconciling a telemetry
Binding cannot write **any** common status field. Everything it observes has to
go under `status.provider`, which means it is implementation-owned rather than
provider-neutral, which is the opposite of what a common base exists for.

**Why.** The telemetry dossier describes `TelemetryBinding.status.resource` in
prose rather than as a member table. The module comment enumerates what is
described but unnamed: the effective signal, quota and policy digests, the
ingest and import readiness summaries, the producer counts, the queue and drop
counters, and the Binding's observed generations, occupancy, and stamping flag.
The Service side of the same family fared better only because two spellings -
`serviceRole` and `serviceReadiness` - happen to be stated literally.

**Class: specification gap.** Not an unmet obligation, because nothing in
`ADR046-provider-004`'s validation field requires a non-empty status layer, and
the slice's behaviour is the correct fail-closed reading of a document that does
not name the fields. Not an inference either, because the catalog declined to
infer - it froze nothing and rejects everything rather than choosing plausible
names that would then bind every implementation. The gap is in the telemetry
dossier, and it is discharged by amending that dossier to state the field-name
table, not by any change here.

### 13.3 Security-key cannot construct a signed projection factory at all

**What the tree does.** `security_key.rs` declares
`allowed_backing_ref_types: None`. `SemanticProjectionBinding::projection_factory`
turns that into `Err(SemanticContractError::BackingRefTypesUndetermined)`,
whose diagnostic label is `semantic-backing-ref-types-undetermined`.
`the_backing_ref_set_is_undetermined_and_fails_closed` pins both the `None` and
the error. The other three families return a factory: audio and telemetry back
onto `Endpoint`, USB onto `Device`.

**Why the failure is genuine rather than a missing line.** `ProjectionFactory::new`
in `packages/d2b-contracts/src/v3/provider.rs` rejects an empty
`allowed_backing_ref_types` with `ProviderContractError::BoundExceeded`, so
there is no "empty means unconstrained" spelling available. The catalog cannot
pass an empty set and cannot invent a non-empty one, so `None` and a typed
error is the only honest option left.

**Why the set is undetermined.** The security-key dossier places `deviceRef`
and the relay Endpoint inside the implementation's strict `spec.provider`
extension, not in the semantic base. No semantic base field of this family names
a backing resource at all, so there is nothing to derive the closed set from.
`Device` is the plausible guess and is exactly what the catalog refused to
assume.

**Consequence.** `ADR046-zone-control-019` and `-020` are documented in this
item's `integration` field as using the factory metadata to admit an owner
Service and core-create one same-type projection Service. For security-key,
there is no factory to use. Whichever implementation phase owns those items will find three of
four families work and the fourth returns a typed error.

**Class: specification gap, with an unmet-obligation consequence downstream.**
The gap is the security-key dossier not stating a semantic backing set, and it
is fixed by amendment - either by naming the closed set at the semantic level,
or by ruling that a family with no semantic backing resource legitimately has no
projection factory, in which case `ProjectionFactory`'s non-empty requirement is
the thing that needs to change. The consequence for `ADR046-zone-control-019`
and `-020` is a real unmet obligation, but it is theirs and it is blocked behind
this amendment.

**Discharged by ADR 0051 Amendment A.** The security-key dossier now declares
an empty provider-neutral backing set as a determinate deny-all, so the
projection factory is constructible without inventing a physical backing
reference.

### 13.4 The other underdetermined semantics this slice reported

Checked against sections 2 and 10.2 first; none of the four was already
recorded. All four are **inferences** rather than gaps in the sense of 13.2 and
13.3, because in each case the catalog did choose something and a reviewer needs
to confirm or correct that choice.

| Where | Inference | Class | Product owner |
| --- | --- | --- | --- |
| All four family modules | **Only the top-level field-name set of each layer is frozen.** The module doc states the rule and each family module names the interiors it declines to model: audio `grants` and `channels`, security-key `authority` / `target` / `policy`, telemetry `signals` / `quota` / `policy`, USB `accessPolicy` / `backingAuthority` / `attachmentPolicy`. The reason differs per case and is worth keeping distinct - some interiors are stated as prose, some appear only inside a dossier example, and audio's `grants` members and domains **are** stated but were still left unfrozen for consistency with the others. So this is not uniformly forced by the documents; part of it is a consistency choice. The effect either way is that two implementations of one family can disagree about an interior and both pass the common base | Inference | focused contract review, before the semantic bases are treated as frozen |
| `security_key.rs`, `usb.rs` versus `audio.rs`, `telemetry.rs` | **The Service mode discriminant is spelled three ways and each family keeps its own.** Security-key and USB use a field named `mode`; audio and telemetry use `serviceRole`. The values diverge again inside that: telemetry's authority value is `"authority"` while audio's is `"owner"`, so the three live spellings are `mode: "authority"`, `serviceRole: "authority"`, and `serviceRole: "owner"`. Each is what its own dossier says, so per-family fidelity and cross-family uniformity are in direct conflict and the catalog chose fidelity. Recorded because a consumer writing one code path across the four families has to special-case it, and because a later decision to unify moves four frozen field sets and therefore four fingerprints | Inference | focused contract review; unification, if wanted, must precede the fingerprints being consumed |
| `mod.rs`, `SEMANTIC_BASE_SCHEMA_MAJOR` / `_MINOR` | **No base schema version is stated for the semantic bases themselves, so `1.0` was chosen.** The constant's own doc says so. The value is not inert: it is an input to `layer_fingerprint` and therefore reaches every one of the sixteen base fingerprints and, through the projection schema fingerprint, all four factory fingerprints and the committed schema artifacts | Inference | focused contract review, before any Provider manifest pins a base fingerprint |
| `mod.rs`, `SEMANTIC_PROJECTION_PROTOCOL_VERSION` | **The semantic projection-protocol version has no stated spelling, so `"1.0"` was chosen.** The specification requires the factory fingerprint to bind this value and to exclude Provider and adapter identity; it fixes neither the spelling nor the value. It is an input to `factory_fingerprint`, so all four committed `x-d2b-factory-fingerprint` values depend on a string nobody specified | Inference | Same as above |

### 13.5 Where code and comment or register disagree

Three, all small, all recorded rather than corrected in place.

- **Section 11's row was stale in one further way it did not admit.** It said
  `docs/reference/schemas/v3/` "now also holds the semantic-service artifacts,
  four per domain across the four domains plus a `projection_spec` schema each".
  That is right and the count is 20, but the row presented it as an inventory
  observation while declining to assess it; the artifacts were already
  drift-gated at the time the row was written, which is assessable evidence the
  row had in hand and did not use.
- **Two test names overstate what their bodies check**, both noted in 13.1.
  `binding_refs_and_targets_are_admitted_against_the_frozen_sets` sits under a
  doc comment headed "Same-Zone refs and targets" and checks no Zone;
  `the_stored_factory_fingerprint_is_rederivable_from_the_public_inputs` checks a pure
  function against itself and derives its force entirely from a different test.
  Neither is wrong about the code's behaviour - the same-Zone predicate really is
  the caller's, and the fingerprint really is Provider-independent - but a reader
  auditing by test name would credit both with more than they carry.
- **The catalog has no production caller.** Nothing outside
  `packages/d2b-contracts/src/v3/mod.rs` and
  `packages/xtask/src/semantic_service_schemas.rs` names `semantic_services`.
  This extends section 10.4's observation about provider implementation rather than contradicting
  it: like the three system-Provider crates, the semantic catalog is proven only
  against itself and its own generator, and the first evidence it matches a real
  Zone store arrives with the zone-control items that consume the factory
  metadata.

### 13.6 Summary of what this section adds to the register

| Finding | Class | Product owner |
| --- | --- | --- |
| Telemetry Binding common status layer is empty, so no common status is writable for that type | Specification gap | Amendment to the telemetry dossier, before a telemetry controller is written |
| Security-key names no semantic backing resource, so no signed projection factory can be built for that family | Discharged by ADR 0051 Amendment A | Security-key dossier amendment before `ADR046-zone-control-019` / `-020` |
| `ADR046-zone-control-019` / `-020` will find one of four families without factory metadata | Unmet obligation, blocked | Behind the amendment above |
| Same-Zone half of the Binding ref/target rule is delegated to a caller that does not exist, and three of four families' target sets are untested | Unmet obligation | `ADR046-provider-004`, or the implementation phase that first resolves refs in a Zone |
| Serde round trip proved for the spec layer only, not status or projection | Unmet obligation, minor | Any later slice owning `semantic_services` |
| Alias rejection proved as non-registration, not as resolver rejection | Unmet obligation, deferred by construction | The implementation phase that admits installed Provider schemas into a resolver |
| Only top-level field-name sets are frozen; every named interior is unmodelled, partly by necessity and partly by consistency choice | Inference | focused contract review |
| Service mode discriminant has three live spellings across four families | Inference | focused contract review |
| Semantic base schema version `1.0` chosen with none stated; it reaches every base fingerprint | Inference | focused contract review |
| Semantic projection-protocol version `"1.0"` chosen with no stated spelling; it reaches every factory fingerprint | Inference | focused contract review |

## 14. Rulings recorded before controller implementation opens, and what debt the implementation phase takes on

Recorded the way section 9 was, before the implementation phase's slices open, so its scope and
its shared-file decisions are settled rather than argued at review. controller implementation is
the program's largest implementation phase - 31 work items across six parallel groups, against
provider implementation's four - so a shared-file collision that provider implementation could absorb in a
follow-up round would here collide across three groups at once.

Six rulings follow. Each was verified against the tree and the manifests before
being recorded, and the three categories this register already separates are
kept apart: an **unmet obligation** names work someone still owes, an
**inference** names a reading a reviewer must confirm or correct, and a
**specification correction** names a place where shipped code and written
specification disagree and code was kept.

### 14.1 Shared-file ownership and validation

Historical planning records described integration phases, commit tags, and
review roles. Those mechanics are not current requirements. The technical
finding is retained: shared broker operations and configuration modules need a
single source owner, typed request and response contracts, and focused tests
before consumers rely on them.

### 14.2 The four new broker ops use typed-unimplemented stubs

**The ruling.** The shared broker contract declares all four broker op variants
with their request and response types, and dispatch arms return a typed
unimplemented error until their implementations land. Consumers read one closed
enum rather than creating competing variants.

**What was verified.**

- **None of the four exists today.** Searching `packages/` and
  `docs/reference/privileges.md` for `DeletePersistentTap`, `CreateBridge`,
  `DeleteBridge` and `ApplyNftablesProjection` returns nothing, so all four are
  new surface rather than extensions of a landed variant.
- **Two W4 items in two different parallel groups need them.**
  `ADR046-network-005` (group `wi:ADR-046-resources-network`) and
  `ADR046-network-007` (same group) both consume them, and
  `ADR046-network-008`'s validation field additionally pins that "bridge
  `DeleteBridge` broker call made exactly once during finalizer" - and that item
  sits in the separate `wi:core-config-hub:w4` group. So the enum is read across
  group boundaries, not only within one.
- **The broker is an excluded sibling workspace.** `tests/AGENTS.md` records
  `packages/d2b-priv-broker/` as intentionally outside `packages/Cargo.toml`
  with its own lockfile, and `AGENTS.md` records its `unsafe_code = "deny"`
  policy. Two slices editing it concurrently would collide on both the enum and
  that separate `Cargo.lock`.

**The catalogue row moves with the op, and prep must carry it.** Verified in
`AGENTS.md`: the "Adding new per-VM behaviour" section requires privileged side
effects to be "routed through a typed `d2b-priv-broker` op declared in
`packages/d2b-contracts/`" and points at
[`docs/reference/privileges.md`](../../docs/reference/privileges.md) as "the
broker op catalogue", and the References section names that file
"authoritative broker op catalogue". An authoritative catalogue that omits a
declared op is wrong the moment the op lands, so the prep commit adds the four
rows in the same change as the four variants. Note that
`docs/reference/privileges.md` is shipped reference prose, so no implementation phase or finding
marker may appear in those rows.

**Class: inference.** Landing a variant whose dispatch arm cannot succeed is a
deliberate, temporary hole in a fail-closed surface. It is the right trade here
because the alternative is a merge conflict in the one workspace where a
conflict is most expensive, but a reviewer should confirm that the typed
unimplemented error is refused rather than treated as a soft failure by every
caller, and that no slice ships against it without replacing it.

### 14.3 `d2b-core-controller/src/configuration.rs` becomes a directory module in prep

**The ruling.** The prep commit converts
`packages/d2b-core-controller/src/configuration.rs` to
`configuration/{mod,bundle_apply,generation_transition}.rs`, so each of the three
items writing it owns a distinct file.

**What was verified.** The file exists in the tree today, and all three items
name it in their `destination`, in three different parallel groups:

| Item | Group | What its destination says about the file |
| --- | --- | --- |
| `ADR046-core-001` | `wi:ADR-046-core-controllers` | names `configuration` in its `src/{...}.rs` brace list |
| `ADR046-pstate-010` | `wi:ADR-046-provider-state` | "diff/apply loop, name-conflict detection, `pending-cleanup` Zone status, `maxFinalizerDurationSeconds` stall detection" |
| `ADR046-network-008` | `wi:core-config-hub:w4` | "bundle application, diff, generation-transition logic ..., prior-bundle retention" |

The two detailed destinations overlap substantively rather than merely sharing a
path: both claim the diff, both claim name-conflict handling, and both claim
generation transition. A file split alone does not resolve that overlap - it
resolves the *edit* collision, and the ownership question of who writes the diff
is a separate matter for the implementation phase plan's file-ownership map.

**Class: code-canon implementation decision.** `configuration.rs` is committed, passing code, so
reshaping it is a refactor governed by the "existing code is canon" rule. That
rule makes the code authoritative over prose, which means a slice may not
restructure it on its own judgement to make its own destination fit. Recording
the split here keeps the implementation ownership explicit rather than relying
on a slice-local workaround. No implementation should move the file merely to
make a destination label fit.

### 14.4 Provider-crate-layout policy uses the existing xtask lane

**Class: specification correction. Code and the test contract are canon.**

**What was verified, on both sides.**

- **The destination text.** `ADR046-pstate-011` names
  `packages/xtask/src/provider_crate_policy.rs` for the provider-crate-layout
  policy.
- **The closed-set rule.** `tests/AGENTS.md` states it without an escape hatch:
  "There is no 'type 7/8' escape hatch: the drift gates and meta gates are a
  **closed set** - do not add a new `tests/*.sh`", and its directory map labels
  `tests/unit/gates/` "drift/perf gates (closed set)". `AGENTS.md` repeats the
  prohibition from the other direction.
- **The item's own validation field, which settles it.** It reads
  "`make test-policy` runs `cargo xtask check-provider-crate-layout`." The
  check executes as an xtask subcommand under an enforcing policy lane, with
  no role left for a shell script. The **destination is the outlier within the
  item itself**, not a rule the ruling overrides.

**The ruling.** Implement `packages/xtask/src/provider_crate_policy.rs`, expose
it as `cargo xtask check-provider-crate-layout`, and wire it into the existing
`test-policy` target. The item's eight validation
outcomes - missing `src/`, `tests/`, `integration/` or `README.md`; an
`integration/` with no `.rs` files; all four present and non-empty passing;
non-provider `d2b-*` crates unflagged; idempotence across re-runs - are all
filesystem predicates and need no shell.

The historical manifest is not corrected in place; any prose change belongs in
its owning contract with focused validation, in the same way as the destination
drift already recorded in section 5 and the crate-layout policy file-name drift
already recorded in 10.3. That 10.3 entry is the direct precedent: the same policy family already shipped
under a different file name in the enforcing hermetic lane rather than the named
advisory one, for the same reason, and code was kept.

### 14.5 The config-hub item's task ordering is correct

`ADR046-network-008` at T069 sitting after `ADR046-network-009` at T068 is not a
transcription error: the graph gives `-008` `parallelGroup`
`wi:core-config-hub:w4` and `topologicalRank` 13, against `-009`'s
`wi:ADR-046-resources-network` and rank 11, and `tasks.md` carries the matching
`### Group ``wi:core-config-hub:w4`` (1 items)` heading immediately above T069.
The two are in different groups and `-008` genuinely sorts last in the implementation phase.

### 14.6 Which crate, module, and type name own the bundle input DTO

The two items disagree, and the disagreement is real:
`ADR046-pstate-010` puts `ZoneResourceBundle` / `BundleResource` and the
`contentHash` computation in `packages/d2b-core/src/v3/zone_bundle.rs`;
`ADR046-network-008` puts `ZoneBundle` / `BundleResource` / `BundleMetadata` in
`packages/d2b-contracts/src/generation_bundle.rs`, with the explicit rule that
`managedBy` and `configurationGeneration` must not be fields of `BundleResource`
and instead live in `packages/d2b-core-controller/src/resource_store.rs` as
persisted metadata.

**The crate is decided: `d2b-contracts`.** Four independent things point one way
and nothing points back.

1. **The manifest is three-to-one for `d2b-contracts`.** Searching every item's
   destination for a bundle DTO module returns four, not two:
   `ADR046-cli-011` (W5) names `packages/d2b-contracts/src/zone_bundle.rs`,
   `ADR046-volume-006` (W5) names `packages/d2b-contracts/src/v3/zone_bundle.rs`,
   `ADR046-network-008` (W4) names `packages/d2b-contracts/src/generation_bundle.rs`,
   and only `ADR046-pstate-010` (W4) names `packages/d2b-core/`.
2. **`d2b-core` has no `v3` module at all.** `packages/d2b-core/src/` holds no
   `v3` directory; every v3 resource DTO in this tree lives under
   `packages/d2b-contracts/src/v3/`. Honouring `d2b-core/src/v3/zone_bundle.rs`
   would open a second, parallel v3 namespace in a crate that has none, which is
   how one concept acquires two homes as well as two spellings.
3. **The dependency direction favours it.** `packages/d2b-contracts/Cargo.toml`
   depends on `d2b-core`, so contracts is downstream of core. The named Rust
   consumer of the DTO in both items, `d2b-core-controller`, declares exactly two
   d2b dependencies - `d2b-contracts` and `d2b-controller-toolkit` - and does
   **not** depend on `d2b-core`. Placing the DTO in `d2b-contracts` is reachable
   today; placing it in `d2b-core` requires adding a dependency edge to satisfy
   one item against three.
4. **The counter-argument was weighed and does not carry.** `AGENTS.md` records
   `d2b-core` DTOs as canonical for the bundle/manifest artifacts, and
   `ADR046-pstate-010` pairs its DTO with a Nix emitter
   (`nixos-modules/zone-resources.nix`), which mirrors that established
   emitter-to-`d2b-core`-DTO pattern. But that pattern is the v2 `/etc/d2b`
   private bundle - `bundle.rs`, `host.rs`, `processes.rs`, `privileges.rs` -
   and a Zone resource bundle is a v3 resource artifact, whose established home
   is `d2b-contracts/src/v3/`. The landed `nixos-modules/zone-resources-json.nix`
   emitter is grounded in a generated Nix table
   (`generated/zone-spec-canonical.nix`), not in a `d2b-core` DTO, so there is no
   precedent in the tree for this emitter reaching into `d2b-core` either.

**The module path and the type names are now ruled too:
`ADR046-network-008`'s spelling wins.** The DTOs land at
`packages/d2b-contracts/src/generation_bundle.rs` as `ZoneBundle`,
`BundleResource` and `BundleMetadata`. Three things carry it.

1. **`ADR046-network-008` is the only one of the four that *defines* the DTOs;
   the other three merely *name a module*.** Its `destination` was re-read in
   full against `historical task record`. It enumerates all three
   type names, marks them explicitly as the **input** DTOs, and states a
   normative exclusion with its reason: `BundleResource` MUST NOT carry
   `managedBy` or `configurationGeneration`, because both are persisted resource
   metadata that core sets at activation rather than bundle input fields. It
   then places the closed `ManagedBy` enum `{ Configuration, Controller, Api }`
   and `configurationGeneration: u64` in
   `packages/d2b-core-controller/src/resource_store.rs`, and its
   `detailedDesign` repeats the exclusion. A specification that says what a type
   must not contain, and why, is a definition. One that names a file is a
   reference. `ADR046-cli-011` and `ADR046-volume-006` name their module with a
   parenthetical (`(new)`, `(bundle index schema)`) and no types at all;
   `ADR046-pstate-010` names two types and a `contentHash` computation inside a
   destination list without any field-level rule.
2. **Specificity beats headcount here, because there is no majority to count.**
   The three-to-one figure the crate ruling rests on is about the **crate**,
   where three items genuinely agree on `d2b-contracts`. On path and type name
   they do not agree with each other either: the four destinations give four
   distinct module spellings, and the two W4 items give two distinct top-level
   type names. Counting cannot decide a four-way split with no repeated value,
   so the most normative text decides it, and that is `ADR046-network-008`.
3. **`ZoneResourceBundle` and `ZoneBundle` are one concept, so only one ships.**
   `ADR046-pstate-010`'s `ZoneResourceBundle` is `ZoneBundle` under another
   name, and its `contentHash` computation is a field or method **on**
   `ZoneBundle`, not a second DTO beside it. Recording that here is the point of
   the ruling: without it the two W4 slices land two spellings of one thing,
   which is the identical failure this register already flags for
   `UserDiscoveryEffectPort` in section 10.4 and for the duplicated conformance
   cells in 10.1. `BundleResource` is already common to both items and is
   unaffected.

**Forward-looking specification correction, for the W5 items.**
`ADR046-cli-011` (`packages/d2b-contracts/src/zone_bundle.rs`) and
`ADR046-volume-006` (`packages/d2b-contracts/src/v3/zone_bundle.rs`) must be
reconciled onto `packages/d2b-contracts/src/generation_bundle.rs` when their
implementation phase runs. Both are named here so this is not rediscovered as a fresh
three-way disagreement in W5; neither is in controller implementation's scope and neither blocks
it.

**This ruling unblocks the controller implementation prep commit.** The prior pass recorded the
path-and-name question as blocking prep, because prep cannot land a shared
module whose path and type names are undetermined. Both are now fixed, so prep
may land `packages/d2b-contracts/src/generation_bundle.rs` with `ZoneBundle`,
`BundleResource` and `BundleMetadata`, and the two W4 consumers open on a
stable contract.

**Class: ruled. The W5 reconciliation is a specification correction owed by
W5.**

### 14.7 What debt controller implementation takes on

Two items are in scope, each with the reason it belongs to this implementation phase rather
than to a later one.

**1. `ADR046-process-001` discharges provider implementation's largest recorded gap.** Verified
in both directions: the register names the item repeatedly - section 1's
`ADR046-primitives-002` row, section 8's `ADR046-primitives-002` and
`ADR046-primitives-003` rows, and section 10.4's owner line - and the
implementation graph places `ADR046-process-001` in W4 with
`parallelGroup` `wi:ADR-046-components-processes-and-sandbox`. The gap it closes
is the one section 11 called the single largest thing provider implementation leaves: three
crates proven only over scripted ports, with no production caller and
`integration/` directories holding nothing but a `README.md`. The
`ProcessLaunchEffectPort` production adapter is what turns a green provider implementation gate
from evidence about internal consistency into evidence about behaviour.

**2. Importing `nixos-modules/resources-volume.nix`.** Section 3 records the
Volume assertions as imported by no module, so they do not run, and section 8
records the matching eval-case obligation as blocked behind that import. It is
one line in `nixos-modules/index.nix`.

**One correction to the reasoning offered for this item.** The planning pass
justified it as "three W4 slices already open `nixos-modules/index.nix`, so this
is one line in a file the implementation phase is already in". That is wrong on the count.
Searching every W4 item's `destination`, `detailedDesign` and `integration`
fields for `index.nix` returns exactly **one** item, `ADR046-network-004`; the
other three items naming that file are `ADR046-identities-002` (W0, sealed),
`ADR046-nix-004` (W5) and `ADR046-zone-control-007` (W5). The scope decision
survives the correction - the implementation phase does open the file, once, so the marginal
cost is still one line and one owner - but the file is a **single-owner** file in
this implementation phase, not a shared one, so the import must be assigned to
`ADR046-network-004`'s slice in the file-ownership map rather than dropped in by
whichever slice notices first.

### 14.8 What controller implementation does not take on

Four groups stay where the register already puts them. None is deferred for
convenience; each has a reason that would not change by moving it.

**The sealed routing implementation nix-unit and host-integration obligations** recorded in
section 8 against `ADR046-routing-011`, `-012`, `-013` and
`ADR046-primitives-003`. Section 9 already declined these for provider implementation and both of
its reasons still hold: every one is owed by a sealed-W2 item that already
carries a recorded owner, so transcribing them here would give one obligation two
implementation phase attributions and make the discharging implementation phase ambiguous; and several still
cannot execute, because they need a production store that no W4 item delivers.

**The `zone-bootstrap` / `zone-enroll` handler**, ruled to W5 alongside
`ADR046-store-004` in section 1. That ruling rests on a verified dependency
rather than on file ownership: `packages/d2b-bus/src/session/enrollment.rs`
defines recovery entirely in terms of persisted facts and states that the caller
performs the single durable store transaction that seals the record. The enroll
handler is that caller, and the durable store lands in W5. Scheduling it into W4
would schedule work that cannot finish.

**`ADR046-routing-014` and `ADR046-routing-015`**, the two partial Provider
items. Section 10.1 verified that these are blocked on specification content
that exists nowhere: no request or response payload is written for any Provider
method, including the six that are named, and there is no proto, frozen service
name or field numbering. No implementation phase can discharge that, so moving it into W4 would
move the hole rather than close it. `ADR046-provider-001` retains ownership.

**The provider implementation specification gaps about required derivation outputs**, recorded in
12.2 and 12.3. The Phase 2 rule names three required outputs and specifies no
path, filename, Nix output name or layout for any of them, and the rule has no
conformance scenario to cite. Both need a specification amendment before
`ADR046-zone-control-015` in W5; neither is implementation phase work, and neither is W4's.

### 14.9 Summary of what this section adds to the register

| Finding | Class | Product owner |
| --- | --- | --- |
| W4 stays one sealed implementation phase and delivers in two integration phases; the opening phase's commits take `( W4 )` and `( W4fu<M> )`, not the `W4a` form, which means post-W4 prep | Inference | W4, confirmed at the implementation phase focused review |
| Four new broker ops land in the prep commit as typed-unimplemented, with their `docs/reference/privileges.md` catalogue rows in the same change | Inference | W4 prep |
| `d2b-core-controller/src/configuration.rs` splits into a directory module in prep; the three items' substantive overlap on diff and generation transition is a separate ownership question | Decision recorded against landed code | W4 prep, plus the implementation phase file-ownership map |
| `ADR046-pstate-011`'s destination and integration field both resolve to the xtask provider-layout policy | Specification correction, code and the test contract canon | W4; documentation correction separately, with focused validation |
| `ADR046-network-008` at T069 after `ADR046-network-009` at T068 is correct ordering across two groups | Not debt; recorded to forestall a finding | - |
| The bundle input DTO belongs to `d2b-contracts` on four independent grounds | Ruled | W4 prep |
| Its module is `packages/d2b-contracts/src/generation_bundle.rs` with `ZoneBundle`, `BundleResource`, `BundleMetadata`; `ADR046-pstate-010`'s `ZoneResourceBundle` is the same type and its `contentHash` is a member of it, not a second DTO. Prep is unblocked | Ruled | W4 prep |
| `ADR046-cli-011` and `ADR046-volume-006` name two further module spellings for that same DTO and must be reconciled onto it | Specification correction, owed by W5 | W5 |
| `ADR046-process-001` closes provider implementation's largest gap: three crates with no production caller, proven only over scripted ports | Unmet obligation, in scope | W4 |
| `nixos-modules/resources-volume.nix` imported by nothing; one line in `index.nix`, which exactly one W4 item opens | Unmet obligation, in scope | W4, assigned to `ADR046-network-004`'s slice |

## 15. Added after controller implementation Round A

This section is the Round A debt audit. It was built by reading the complete
`validation` field for each of `ADR046-process-001`, `ADR046-core-001`,
`ADR046-pstate-001`, `ADR046-pstate-002`, `ADR046-network-001`,
`ADR046-credential-001` and `ADR046-credential-002`, then reading the tests and
production wiring that actually exist. The eleven reported caveats were checked
against the tree, but they were not used as the inventory.

The distinction used by the rest of this register is preserved here:

- An **unmet obligation** names behavior or evidence that the implementation
  still owes.
- An **inference** names a defensible choice made where the specification is
  silent and which a reviewer must confirm or correct.
- A **specification gap** names an obligation that cannot be implemented without
  first adding or reconciling contract content.

### 15.1 Independent validation audit by work item

| Work item | Result of reading the complete validation field against the tree |
| --- | --- |
| `ADR046-process-001` | **Met at the hermetic adapter layer, not at the real system boundaries.** `production_adapter.rs` runs the shared suite through `ProviderSupervisor`, carries a deterministic fault matrix, tests a real seqpacket frame plus genuine pidfd transfer, and has a current-thread heartbeat latency test. The fix round additionally proves that a late successful launch is stopped before timeout is returned, or remains tracked if cleanup fails; terminal stops retire handles; pending observations are bounded; and the broker's actual pid-reuse diagnostic is classified as identity change. It does not prove privileged broker spawn with the real namespace/cgroup policy, real systemd units, real wait/reap, or real PID reuse. No production crate constructs the supervisor, and the three `integration/*.rs` files are declaration-only and run in no lane. The latency test is also weaker than the equal-or-stricter spike retirement condition: one 50 ms call with a 40 ms maximum heartbeat-gap assertion, rather than 200 concurrent calls and the 15 ms bound. |
| `ADR046-core-001` | **Partial.** Every implemented Round A module has focused unit tests, including authorization refusals, catalog activation, handler health, budgets, ownership, provider lifecycle, store-operation admission and watch accounting. The tests are example based; no property or permutation corpus was found for the field's explicit property-test half. Multi-process startup/restart is absent. `Cargo.toml` has `autobins = false`; `main.rs` is a library coordinator over supplied booleans and snapshots because the production ResourceClient, authenticated session connector, operation ledger and watch dispatcher do not exist. `cleanup.rs` is no longer a scaffold: it now implements the pure `PendingCleanup` projection and prior-generation pruning policy with focused tests. Store transactions, finalizer effects, watch delivery and audit appends remain unwired production-adapter work. |
| `ADR046-pstate-001` | **Partial and fail-closed.** The schema and status golden vectors are real. The enum round-trip test covers `StateSchemaPhase`, `MarkerStatus` and `SealingStatus`. Its name says `phase_and_status_reason_tokens_round_trip`, but no reason type is present and its body round-trips no reason. StateEnvelope construction, next-generation bounds and redacted diagnostics are tested, but digest construction and verification now deliberately return `DigestDomainUnavailable`. `VolumeStateSchema` is consumed by the Provider component namespace, but `VolumeSpec` and `VolumeStateStatus` are not integrated: outside `volume_state.rs`, only `provider.rs` consumes `VolumeStateSchema`, and `volume.rs` consumes none of these types. |
| `ADR046-pstate-002` | **Partial.** Descriptor bytes, the stateless round trip, missing namespace rejection, invalid kind, forbidden persistence, quota floor, host-custody refusal and guest-local-required refusal are covered. The named descriptor-Volume consistency property test is not: `descriptor_volume_projection_preserves_quota_source_and_exports` checks three example quotas against a four-field `ComponentStateVolumeProjection`; it constructs no `VolumeSpec`, creates no Volume or Export, and compares no schema, views, layout, ownership or attachment content. The custody check accepts a caller-supplied `StateSchemaCustodyClass`; no authoritative schema-id-to-custody mapping exists. Derivability rejection and placement-change version increment remain unimplementable with the current types. |
| `ADR046-network-001` | **Partial.** The JSON spec/status vectors, CIDR examples, attachment-index uniqueness, host blocklist, IfName collision and deterministic repeated derivation, external-NIC cross-Zone refusal, and reserved User declaration/gate are covered. No CBOR codec or profile exists. The CIDR test is a fixed example table, not the requested property test. The User test constructs a contract value and calls a pure phase gate; no controller creates the User, waits on a watch, aborts a real config-Volume operation, or proves the status UID/GID is ignored by authorization and audit. Two written specifications disagree about who creates the User and whether an authored empty additive blocklist is valid. |
| `ADR046-credential-001` | **Met except for one specification hole and one opacity limit.** The spec and status golden vectors, bounds, unknown-field rejection, `OpaqueAzureRef` preservation with redacted diagnostics, one-way lease/source wrappers, and status/error canaries are present. The reported claim that the interactive-login status field is absent is not true as written: `CredentialStatus` contains `interactionState`, `loginSessionGeneration` and `loginDeadline`. What is absent is the specified `challengeMetadata`, whose shape is never defined, and the placement of `BeginLogin`, `ObserveLogin` and `CancelLogin` conflicts with the frozen five-method service. The unkeyed lease/source digest also hides bytes but does not prevent offline guessing of low-entropy inputs; no keying authority is specified. |
| `ADR046-credential-002` | **Met hermetically, unwired in production.** All five method vectors, strict malformed/duplicate/non-canonical/oversize rejection, the use/admin Role matrices, delivery binding round trip, non-delivery rejection, zeroizing record behavior and real state-to-error mapping tests exist. The fix round proves the Provider cannot replace any of the twelve authorization-derived delivery fields. Field numbers were assigned without a specification rule and are now pinned by vectors. `CredentialAdmission` remains an injected trait with no production implementation, the server is unregistered, and no bus route, Provider Process selection, delivery-route authorization or encrypted forwarding path invokes it. |

### 15.2 Unmet obligations

| Debt | What the tree proves and does not prove | Owner / closing condition |
| --- | --- | --- |
| Core production process and multi-process startup/restart | `CoreProcess` proves the in-process ordering policy over supplied readiness and recovery facts. There is no binary, ResourceClient, authenticated connector, accepted store/operation ledger, or real watch dispatcher, so no restart has re-established those facts across processes. | W5 production store/watch integration (`ADR046-store-004`, `ADR046-store-002`, `ADR046-reconcile-003`) plus the Zone-runtime connector that constructs the process |
| Core property-test half | The nine Round A modules have example-based unit tests. No generated or permutation tests prove ordering independence, deterministic aggregation, or state-machine properties over a broad input space. Construction-time use of `BTreeMap` is not that evidence. | `ADR046-core-001`, before its validation field is called complete |
| Core cleanup production adapter | `packages/d2b-core-controller/src/cleanup.rs` now implements and tests the pure `PendingCleanup` and prior-generation pruning policy. It deliberately performs no store transaction, finalizer effect, watch delivery or audit append, and no production runtime adapter invokes those effects. | Production store/watch and finalizer integration must consume the typed cleanup policy; the pure module is no longer an implementation blocker |
| Process real-boundary integration | The adapter and its policy are real Rust code, but its conformance/fault tests inject deterministic or scripted backends. Privileged spawn and namespace/cgroup placement, transient system/user units, parent-owned wait/reap and actual PID reuse remain unproved. | Container and host-integration scenarios wired through repository lanes; the declarations in `integration/` are not evidence |
| Process production reachability | Only the supervisor crate and its tests construct `ProviderSupervisor`; no production crate depends on `d2b-provider-supervisor`. The prior absence of an adapter is closed, but the prior absence of a production caller is not. | Process controller/runtime integration, including `ADR046-reconcile-003` against the accepted store/watch backend |
| Process blocking-adapter retirement proof | `blocking_effects_do_not_stall_the_async_executor` is useful but weaker than the recorded spike cleanup: it drives one delayed call and permits a 40 ms gap. It does not drive 200 concurrent calls or enforce the 15 ms gap bound. | `ADR046-process-001` for the Process adapter half; end-to-end commit-to-launch latency remains `ADR046-reconcile-003` |
| Broker PID and start-time disclosure | `OperationFields::OpenPidfd` serializes raw `pid` and `expected_start_time_ticks`, and live error Display strings render PID plus expected/observed start times. The process adapter does not format these values, but the broker audit and journal paths do. | Broker audit hardening, no later than `ADR046-audit-001`'s `BrokerEffect` migration |
| Provider-state types are not embedded in Volume | `VolumeStateSchema` reaches `ComponentStateNamespace`, but `VolumeSpec` has no state-schema extension and no Volume status layer consumes `VolumeStateStatus`. | W5 `ADR046-volume-001` plus the provider-state fast follow required by the shared-path ruling in validation/delivery section 7 |
| Descriptor-Volume consistency property | The current projection helper returns source, byte/inode quota and an Export count. It neither constructs nor validates the actual source Volume and Export children, so equality of schema, views, ownership, layout, attachment and placement cannot regress visibly. | `ADR046-pstate-009` and production ProviderDeployment; use real typed Volume/Export objects rather than another projection-only assertion |
| Network controller User lifecycle | The pure contract test proves the intended declaration and phase decision. It does not prove creation ownership, watch ordering, config-Volume suppression, the emitted `ConfigVolumeReady=False/user-not-ready` condition, or that diagnostic numeric identity is excluded from authz and audit decisions. | `ADR046-network-005`, whose validation field already names these controller cases |
| Production Credential service path | The service has no authenticated bus adapter, exact Provider Process resolver, delivery-route authorizer, registered server or opaque encrypted record forwarder. A fake `CredentialAdmission` returning denial proves ordering after injection, not that production RBAC constructs the right result. | Credential/bus integration before any Credential Provider is production-reachable |
| Broker runtime skew protection | `PROTOCOL_VERSION = 4` correctly describes the current operation catalogue, but `BrokerRequestEnvelope` carries no version. `HelloRequest` carries a semver-like string, the runtime ignores it, and `hello_ok_response` returns hard-coded selected/server strings. The compatibility tests prove serde behavior only. | Broker/daemon wire owner before mixed catalogue versions may coexist; either negotiate and reject skew or rename/document the constant solely as catalogue metadata |

### 15.3 Specification gaps

| Gap | Why implementation cannot close it | Exact amendment or ruling needed |
| --- | --- | --- |
| Network CBOR golden vector | `ADR046-network-001` requires JSON and CBOR vectors, but no permitted contract defines a CBOR profile, canonicalization rule or codec. Inventing bytes would make the test define the protocol it claims to test. | Define the Network CBOR profile and codec, or amend the validation field to the one canonical encoding the resource plane actually supports |
| Network controller User ownership conflict | The resource specification/work item says the network-local controller creates and owns the User. The Provider dossier says Nix config publication declares it and the controller has read-only `get,watch`. | Choose one creator and align the resource specification, dossier, RBAC rows and lifecycle test |
| Empty authored Network host blocklist conflict | The Provider dossier illustrates an empty additive list whose defaults are unioned later. The resource specification and contract reject an explicitly empty list as incomplete. | Choose authored-additive or authored-effective semantics and align the example, validator and error contract |
| Provider-state reason round trip | The validation field requires `phase/reason` round trip, but no provider-state reason field, enum or closed token set is defined. The current test name overclaims a body that covers phase, marker status and sealing status only. | Define the bounded reason type and where it appears, then add its vector; or remove `reason` from the validation obligation |
| Derivable-payload rejection | `ComponentStateNamespace` carries the asserted `StorageNeed` enum but no payload, status, ledger or external-observation evidence. The constructor can reject a missing assertion, not prove the assertion false. | Put authoritative derivability evidence at ProviderDeployment admission, as `ADR046-pstate-012` proposes, and define the evidence input; do not pretend the descriptor constructor can infer it |
| Placement-change descriptor version increment | `ComponentDescriptor` has no descriptor-version field. Its config digest and the Provider compatibility fingerprint have different meanings. | Add and define an immutable component descriptor version and its comparison point, or remove the increment requirement in favor of an existing, correctly named generation contract |
| Schema custody classification source | `validate_schema_custody` accepts a trusted `StateSchemaCustodyClass`, but no schema contract maps a schema ID or signed registration to that class. A caller can select `Ordinary` for a credential schema and bypass the intended refusal. | Freeze the custody-class field in the signed schema registration and define the authoritative lookup used by ProviderDeployment before `ADR046-pstate-009` and `ADR046-pstate-012` consume it |
| Provider-state payload digest domain | D101 freezes the complete domain-tag set and contains no Provider-state payload domain. Accepting a caller tag would permit two domains for the same payload, so the helper now correctly fails closed. | **Amend D101 to freeze exactly the domain tag `d2b:v3:provider-state-payload` and define the StateEnvelope payload digest as `SHA-256(b"d2b:v3:provider-state-payload" || 0x00 || d2b-cjson/v1(payload))`, rendered in the D101 `sha256:<64 lowercase hex>` form.** Only then may `from_payload` and `validate_digest` succeed |
| Credential interactive challenge metadata and method ownership | D093 names bounded non-secret challenge/progress metadata but defines no fields, enum, bounds or null rules. It also places `BeginLogin`, `ObserveLogin` and `CancelLogin` in the common Credential surface while `ADR046-credential-002` freezes exactly five service methods that omit them. | Define the challenge metadata schema and decide whether the three login operations extend `d2b.credential.v3` or live on a separate typed Endpoint service; then add the corresponding vectors |
| Credential opaque-wrapper keying | Lease/source wrappers use domain-separated unkeyed SHA-256. The raw value is absent from output, but a low-entropy input remains guessable offline, and no keying authority or minimum-entropy contract exists. | Define a keyed opacity derivation and key owner/rotation contract, or explicitly classify these inputs as non-authorizing high-entropy values and enforce that at construction |
| Core cleanup file attribution | `ADR046-core-001` and `ADR046-network-008` both name `cleanup`, but the shared file is now implemented as pure cleanup policy and covered by focused tests. The former scaffold no longer creates an implementation ownership conflict; only the overlapping destination prose remains. | Update the two destination descriptions to identify the module as shared policy when those descriptions are next maintained; no code ownership decision is still blocking |

### 15.4 Inferences

| Inference | Why it needs confirmation | Owner |
| --- | --- | --- |
| Credential protobuf field numbers | No specification assigns the numbers. Round A used dense first-version tags, kept required request fields in the one-byte tag range, and pinned every message by byte vector. This is a sound first-version choice, but the tests now freeze an implementer decision rather than a written contract. | Confirm or replace in the Credential protocol amendment before external clients ship |

### 15.5 What the fix round actually established

These are closed findings, not debt. They are stated so the open entries above
do not erase real progress or cause a later reviewer to ask for the same fixes
again.

- **Credential delivery authority is no longer Provider-selected.** The
  authenticated admission result owns all twelve binding fields. The Provider
  receives that result read-only, and changing any one field in its response is
  rejected. The denial path still runs before Provider dispatch.
- **Credential failure-state coverage is no longer vacuous.** The test drives
  the real state-to-error mapping for locked, unavailable, denied, expired and
  revoked states. Service errors render the canonical codes. `OpaqueAzureRef`
  again preserves its specified non-secret value while Debug and Display remain
  redacted.
- **A timed-out launch no longer silently orphans a late process.** A late
  successful backend result is stopped and confirmed before timeout returns. If
  that cleanup fails, the successful launch is returned and retained as tracked
  authority instead of being discarded. Terminal stop retires retained handles,
  and both observation maps are bounded and consumed.
- **Broker PID-reuse diagnostics now enter the quarantine path.** The adapter
  recognizes the broker's actual live-handler error spelling as an identity
  change rather than an ordinary launch failure.
- **Watch cursors now describe delivery, not the store tip.** A commit advances
  the store revision only; a watcher advances only when it has credit and
  receives the event. One watch cannot advance another, compaction cannot pass an
  undelivered live cursor, and registration cannot claim unreceived history.
- **An empty state-Volume declaration now fails closed.** A component that sets
  the declaration flag but names no namespace is rejected by construction,
  deserialization and manifest admission. This closes the fourth CRITICAL
  finding; it does not close the separate derivability gap above.
- **A static external IPv4 attachment now requires a gateway.** Construction and
  deserialization both reject the unusable address-without-gateway shape.
- **Provider-state digesting no longer accepts an invented caller domain.** It
  fails with `DigestDomainUnavailable` until the exact D101 amendment above
  lands.
- **The broker catalogue change is now accurately versioned, authorized and
  classified.** The earlier fix set the constant to four, added the four new
  operations to the current closed catalogue rather than the legacy list, and
  pinned their audited and destructive flags. It did not add them to the
  canonical authorization matrix, its Nix mirror, generated schemas or broker
  dispositions. This correction completes those machine-readable contracts.
  The compatibility tests prove old request decode and old-decoder rejection of
  new operations; they do not claim runtime negotiation. The four dispatch arms
  were deliberately unimplemented at this Round A snapshot and have since been
  promoted live, as recorded in section 18.

### 15.6 Verification of the eleven reported caveats

| # | Verification | Class |
| --- | --- | --- |
| 1 | **Held.** Core multi-process startup/restart, binary construction, production ResourceClient, authenticated connector and watch dispatcher are absent. | Unmet obligation |
| 2 | **Held.** The real privileged/systemd/PID-reuse boundaries and production supervisor construction are absent, and all three scenario files are declaration-only. | Unmet obligation |
| 3 | **Held.** Broker audit fields and live diagnostics expose PID/start time; the Round A adapter itself does not format them. | Unmet obligation, pre-existing |
| 4 | **Held.** No CBOR profile or codec exists for the requested second Network vector. | Specification gap |
| 5 | **Held.** The creator of `User/net-local-controller` and the empty additive blocklist have conflicting written answers; Round A followed the resource specification. | Specification gap |
| 6 | **Held.** Credential proto field numbers are not specified; Round A assigned and pinned them. | Inference |
| 7 | **Did not hold as worded; the underlying gap held after correction.** `interactionState`, login generation and deadline exist. `challengeMetadata` is absent because its shape is undefined, and the three login operations conflict with the five-method service catalogue. | Specification gap |
| 8 | **Held.** Production bus routing, Process selection, route authorization and encrypted record forwarding are absent; the server is intentionally unregistered. | Unmet obligation |
| 9 | **Held.** Derivability cannot be inferred from the enum-only descriptor, and no descriptor version exists for the placement-change check. | Two specification gaps |
| 10 | **Held.** D101 has no Provider-state payload domain; digesting correctly fails closed. | Specification gap |
| 11 | **Held.** Version four is catalogue metadata today, not runtime skew protection. | Unmet obligation |

### 15.7 State of debt already recorded before Round A

- **Closed in the narrow sense:** the missing `ProcessLaunchEffectPort`
  production adapter recorded in sections 1, 8, 10.4 and 14.7 now exists, and
  both Process Providers run their shared conformance suite through it. The
  stronger Host/Guest/user and real-boundary obligations remain open as recorded
  above. A production adapter and a production caller are different claims.
- **Still open:** `nixos-modules/resources-volume.nix` is still imported by no
  module; `nixos-modules/index.nix` contains no import for it.
- **Still open:** the prior `ADR046-primitives-002` Host/Guest/user integration
  obligation. The new adapter makes the test implementable; the hermetic
  locality loop does not turn it into a real integration test.

## 16. Added after controller implementation Round B

This is the Round B debt audit, performed against tree snapshot `91fd5b9e`.
The audited work items are `ADR046-process-002`, `ADR046-network-002`,
`ADR046-pstate-003`, `ADR046-pstate-008`, `ADR046-pstate-010`,
`ADR046-credential-003`, `ADR046-credential-004`,
`ADR046-credential-005`, `ADR046-credential-007` and
`ADR046-network-008`. That set was derived from the Round B slice commits and
then checked against every item's complete `validation` field in
`historical task record`; the reported caveats were starting
points, not the inventory.

### 16.1 Specification correction: `ADR046-process-002` cannot fit in its destination

All three structural blockers reported by the slice held on verification.

1. **The dependency direction forbids the obvious local wiring.**
   `packages/d2b-contract-tests/tests/policy_provider_crates.rs` implements an
   allowlist, not a denylist. An in-scope Provider crate may depend only on
   `d2b-contracts`, `d2b-controller-toolkit`, `d2b-core`,
   `d2b-process-conformance`, `d2b-provider` and `d2b-provider-toolkit`.
   `d2b-provider-supervisor` is deliberately classified as a non-Provider
   host-side supervisor and is not in that allowlist. Neither
   `d2b-provider-system-systemd` nor `d2b-provider-system-minijail` may add the
   dependency that would let it construct `ProviderSupervisor` locally.
2. **No production composition point currently constructs the supervisor.**
   The only `ProviderSupervisor::new` and `ProviderSupervisor::with_limits`
   sites are in `d2b-provider-supervisor`'s own unit and integration tests.
   No production `Cargo.toml` depends on that crate. The candidate
   `packages/d2b-core-controller/src/providers.rs` is not such a point: it is an
   effect-free Provider lifecycle planner whose actions stop at
   `EnsureComponent`; it imports neither Process Provider and constructs no
   supervisor. `d2b-core-controller` is also `autobins = false`, and its
   `main.rs` records that the production ResourceClient, authenticated session
   connector and store watch dispatcher do not exist. No other production
   runtime or composition crate in the tree fills that role.
3. **Executable scenarios require repository Layer 2 wiring outside the stated
   destination.** `tests/AGENTS.md` places container scenarios under
   `tests/integration/containers/`, driven by `make test-integration`, and
   booted-system scenarios under `tests/host-integration/`, driven by
   `make test-host-integration`. The two Provider crates' `integration/`
   directories contain only README files. Adding a declaration-only Rust file
   there would not make either repository lane execute it and would repeat the
   dead-test pattern already rejected in Round A.

**Ruling and class: implementation ownership correction.** The recorded task
destination is insufficient to discharge its own integration and validation
obligations. Its scope must extend
to the production composition point that instantiates both Process Providers
over `ProviderSupervisor`, and to the container and host-integration wiring
that exercises system, user, Host and Guest behavior.

There is no suitable composition point to name today. One must first exist in
the production Zone runtime or `ProviderDeployment` path, own child Process
Provider instantiation, and be permitted to depend on
`d2b-provider-supervisor`, `d2b-provider-system-systemd` and
`d2b-provider-system-minijail`. The dependency-direction rule constrains the
two Provider crates, not such a core composition crate. Until that actor is
landed or designated, naming `providers.rs` would be a guess and the item
remains blocked. The implementation must include that composition destination
and the two Layer 2 destinations; it cannot be considered complete merely
because the pre-existing hermetic schema, status and adoption tests still pass.

### 16.2 Effect-adapter deferrals

Two Round B slices correctly stopped at their typed Provider boundaries rather
than adding privileged or runtime code to files they did not own.

| Work item | Verified state | Owner / closing condition |
| --- | --- | --- |
| `ADR046-network-002` | The five Provider modules and their hermetic bridge-port, nftables, route and IPv6 tests landed. `ApplyNftablesProjection`, `CreateBridge`, `DeleteBridge` and `DeletePersistentTap` now have live production broker handlers, generation fencing and audit fields. The production `NetworkEffectPort` still does not exist in `d2b-contracts` or `d2b-core`, and `integration/host_fabric.rs` remains a declaration-only Rust test routed by no repository lane. | `ADR046-nl-001` owns the remaining neutral trait and core adapter. The live-handler half formerly assigned to `ADR046-nl-002` is complete; its executable `host_fabric` scenario remains owed. `ADR046-network-005` cannot reach the live broker operations until the adapter exists. |
| `ADR046-pstate-003` | Marker, quota and domain policy landed, but `integration/volume_local.rs` is declaration-only. The exact Volume effect surface is not merely one of the four Network stubs: the neutral `VolumeEffectPort`, its host-runtime adapter and required closed Volume operations are absent. Existing legacy storage and swtpm broker handlers do not constitute that adapter. | `ADR046-vl-012` owns the concrete core/broker `VolumeEffectPort` adapter and its full provision/sealing scenarios in W6. `ADR046-pstate-009` owns the later W4 end-to-end provider-state and audit fixtures, but those cannot prove the real filesystem boundary until the adapter exists. The current `integration/README.md` statement that ProviderSupervisor owns this adapter is stale; the retained task record assigns it to `ADR046-vl-012`. |

The distinction in the second row matters. The Network deferral is no longer
blocked by typed-unimplemented broker operations; it is narrowed to the absent
neutral contract, core adapter and executable lifecycle scenario. The Volume
deferral remains blocked by an absent neutral contract and adapter plus
operations assigned to a later item. Both still require out-of-destination
writes, but they are not the same broker state.

The current tree corrects one ownership assumption in that table:
`NetworkEffectPort` already lives in
`packages/d2b-provider-network-local/src/controller.rs`, not in `d2b-contracts`.
The retained task records place that trait and its production implementation in
`packages/d2bd/src/network_effect_adapter.rs`, with prospective edits to
`d2bd/{Cargo.toml,src/lib.rs,src/resource_runtime.rs}`. It maps opaque intents only to typed
broker operations and performs no direct host mutation. The retired current implementation
blocker has no prospective effect; the owning contracts and focused checks govern subsequent
implementation and evidence.

### 16.3 Credential Provider work is in progress, not complete

`ADR046-credential-003`, `ADR046-credential-004` and
`ADR046-credential-005` landed partial against their exact `src/`, `tests/`,
`integration/` and README destinations. At snapshot `91fd5b9e`, the tree had
each crate's four named source modules and six named Cargo integration-test
files, while each `integration/` directory still had only a README. A
concurrent completion pass is now adding those fixture files and reshaping the
managed-identity entrypoint. That moving file inventory is not completion
evidence, so this register makes no final claim about individual test cells.

The acceptance criterion is the complete validation command and fixture set,
not compilation of the crate or the presence of files:

| Work item | Validation that must run before closure |
| --- | --- |
| `ADR046-credential-003` | `cargo test -p d2b-provider-credential-secret-service`, including its `src/` unit cells and all six named test targets, then `container-service.sh`, `host-placement.nix` and `cleanup-rollback.sh` through their repository Layer 2 lanes. |
| `ADR046-credential-004` | `cargo test -p d2b-provider-credential-entra --lib --tests`, including the complete lifecycle, conformance, fault, canary, delivery and placement matrix, then `container-service.sh`, `guest-placement.nix` and `cleanup-rollback.sh` through their repository Layer 2 lanes. |
| `ADR046-credential-005` | `cargo test -p d2b-provider-credential-managed-identity`, including the complete lifecycle, conformance, fault, canary, delivery and placement matrix named by this work item, then `container-service.sh`, `host-guest-placement.nix`, `aca-credential-ref.sh` and `cleanup-rollback.sh` through their repository Layer 2 lanes. |

The dedicated managed-identity dossier has a later, larger controller/agent
topology item whose exact command also names `tests/topology.rs`. That is not a
reason to silently add the later item's obligation to
`ADR046-credential-005`; this row records the W4 generated item's own six-test
destination and validation field.

### 16.4 `d2b-core::error::BrokerOp` is a completeness gap only

`packages/d2b-core/src/error.rs` has a second broker-operation enum used only
to format `broker-unimplemented` operator errors. It omits
`ApplyNftablesProjection`, `CreateBridge`, `DeleteBridge` and
`DeletePersistentTap`.

The omission is not the authorization-matrix defect closed in `f8e13283`.
Verification found only three concrete `BrokerOp` construction sites: two
`CreateTapFd` uses in `error.rs` tests and one in the `d2b-core` fuzz target.
The production broker dispatch instead matches
`d2b_contracts::broker_wire::BrokerRequest` and reports operation names through
its own string-based error and audit path. The current omission therefore
cannot make one of the four operations callable, deny it, or change its broker
dispatch. It leaves the generic core error catalogue incomplete and would
prevent a future core caller from representing those four operations through
that envelope. **Class: completeness gap. Owner: a controller implementation follow-up
on `packages/d2b-core/src/error.rs`, before any production caller uses
`Error::broker_unimplemented` for the new operations.**

### 16.5 Independent Round B validation audit

| Work item | Result of reading the complete validation field against the tree |
| --- | --- |
| `ADR046-process-002` | **Blocked, with a destination defect.** The two Provider implementations have pre-existing hermetic conformance and adoption coverage, but no Round B production wiring or executable integration scenario exists. Section 16.1 records all three blockers and the contract correction required. |
| `ADR046-network-002` | **Behavior met at the hermetic Provider layer; pin clause not met and production integration deferred.** Equivalent bridge-port, nftables coexistence, route and IPv6 tests exist in `d2b-provider-network-local`. The validation field's literal statement that all named tests are pinned in `host-prepare-network.txt` and `net-canaries.txt` is false: those files still identify the old `d2b-host`/broker and IfName tests, the IPv6 sequence is pinned in `ipv6-off-readback.txt`, the old nftables matrix is pinned in `nft-coexistence.txt`, and no pin names the adapted Provider coexistence test. Section 16.2 records the real adapter gap. |
| `ADR046-pstate-003` | **Partial.** Marker missing, replaced and mismatched states, cross-domain refusal, quota soft checks and the visible marker crash states are hermetically covered. A memory marker store does not prove crash behavior at each real filesystem provision step, a broker-maintained marker, cross-process isolation or real quota enforcement. The named host-integration file is non-executable. Section 16.2 records the W6 adapter owner. |
| `ADR046-pstate-008` | **Catalogue and hermetic validation met; production integration not met.** `audit.rs` now catalogues all 18 Provider lifecycle events and all six broker-owned operation event names from the volume-local specification. `otel.rs` defines all 15 metric descriptors. `audit_unit.rs` pins both catalogues, bounded audit fields, forbidden payload-field classes, exact descriptor labels, closed label value enums and `d2b.zone` as a Resource attribute. Nothing outside `audit.rs` calls `emit_volume_event`, and no emitter exports `METRICS` to `observability-otel`; the lifecycle items that perform each transition must call the sink when they land. `ADR046-pstate-009` owns the live audit and OTEL fixtures. |
| `ADR046-credential-003` | **In progress.** The specified source and Cargo test files existed at the audit snapshot; its three Layer 2 fixtures were absent then and are being added concurrently. The full named command and repository lane executions have not been accepted as passing in this audit. Section 16.3 is the closure rule. |
| `ADR046-credential-004` | **In progress.** Same status: source and six Cargo test files existed at the snapshot; the three Layer 2 fixtures are being added concurrently, and complete command/lane evidence is still owed. Section 16.3 is the closure rule. |
| `ADR046-credential-005` | **In progress.** Source and six Cargo test files existed at the snapshot; a concurrent pass is adding the four Layer 2 fixtures and the separately specified controller/agent topology. Complete command/lane evidence is still owed. Section 16.3 is the closure rule. |
| `ADR046-pstate-010` | **Partial, with a specification count defect.** The linked section says "All eight" but actually enumerates nine obligations: absent Volume, absent Provider, incident hold, bundle integrity failure, rollback, finalizer timeout, credential-ref guard, name conflict and metric identity absence. The core logic covers diff ownership, intent ordering, incident-hold/finalizer disposition, rollback, count retention and name conflicts; the input DTO rejects a bad content hash; the Nix case covers the credential-ref guard. There is no real store/controller cleanup, no Zone status/audit integration, no generation metric descriptor or canary, no container generation-activation scenario, no full Provider-config schema build rejection, and no test that performs two independent Nix builds and compares their bundle bytes. The destination's named `tests/configuration.rs` and `integration/configuration.rs` do not exist. The eight-versus-nine wording needs a contract-description correction; the missing behavior remains owned by this item and the production store/runtime work it depends on. |
| `ADR046-credential-007` | **Partial.** The generic option surface, Credential assertions, activation Role, one canonical envelope, sort/digest projection and store-path absence checks exist. The eval corpus does not cover a wrong-type artifact, duplicate catalog identity, the complete Provider-specific signed-schema cross-check, or every example promised by the field. None of the eight named host-integration cleanup, nonblocking, pending-status, stalled, child-preservation, dynamic-isolation, retention/rollback and tampered-bundle scenarios exists. The work item remains the owner; production execution also depends on the W5 resource compiler/store/runtime path. |
| `ADR046-network-008` | **Partial, with the former scaffold gap closed.** `cleanup.rs` now implements `PendingCleanup` projection and prior-generation pruning. `configuration/generation_transition.rs` now proves post-commit binding, Provider-schema verification, configuration metadata assignment, per-item controller/API name conflicts, absent-resource deletion scheduling and closed audit projections. The named generation-bundle contract test, nix-unit case and focused core tests now exist, and all four required Network broker operations are live. The production store/watch adapter and the named host-integration file remain absent; no executable end-to-end scenario proves mDNS-child deletion, one live `DeleteBridge` request and consumption of the terminal Deleted watch event before finalizer clearance. |

### 16.6 What the independent pass added

The supplied list correctly identified the blocked Process item, the three
in-progress Credential items, both effect-adapter deferrals and the core error
catalogue omission. The independent validation-field pass additionally found:

- `ADR046-network-002`'s adapted Provider tests are not pinned as its validation
  field claims.
- `ADR046-pstate-008` has complete Provider and broker event catalogues and
  metric descriptor/value validation, but no transition calls, Zone audit
  emission or OTEL export.
- `ADR046-pstate-010` says eight cleanup tests while its linked normative list
  contains nine, and several build, runtime, metric and independent-build
  obligations remain absent.
- `ADR046-credential-007` has no Layer 2 cleanup matrix and only partial
  eval/build coverage.
- `ADR046-network-008` now has its input-bundle contract, nix-unit case,
  generation-transition logic, cleanup policy and focused core integration
  tests. Production store/watch wiring and its end-to-end Network-finalizer and
  host-integration scenarios remain absent.

## 17. Credential Provider dependency-direction correction

The three credential Provider crates depended directly on
`d2b-credential-service`. That violated the enforcing Provider dependency
allowlist, which admits only `d2b-contracts`, `d2b-controller-toolkit`,
`d2b-core`, `d2b-process-conformance`, `d2b-provider` and
`d2b-provider-toolkit`. The violation was not resolved by adding the service
crate to the allowlist.

The chosen disposition is the neutral-contract option. Provider-facing request
and response DTOs, closed redacted errors, metadata, delivery binding shapes,
the strict outer codec, `CredentialAuthorization`, `CredentialProvider` and the
binding-preserving provider dispatch helper now live under
`d2b-contracts::v3::credential`. `d2b-credential-service` re-exports that
contract and retains the client transport, authenticated admission trait,
RBAC authorization helpers and server composition. Its server still performs
admission before dispatch and rejects a Provider response whose method or
delivery binding differs from the authorization-owned value.

This keeps Providers on the public neutral contract while preventing them from
linking the crate that owns service admission and server wiring. Provider tests
exercise their neutral dispatch contract directly. The service crate's own
tests remain responsible for proving denial-before-dispatch and rejection of
every altered delivery-binding field. No dependency allowlist was widened.

## 18. controller implementation closing verification gate

This section records only the non-CRITICAL findings selected for the closing
register update. The concurrent completion agent owns the five destination
files for `ADR046-pstate-004`, `ADR046-pstate-005`, `ADR046-pstate-006`,
`ADR046-pstate-007` and `ADR046-pstate-012`, plus the Core USBIP effect adapter
for `ADR046-network-007`. Those paths appeared or were changing during this
inspection, but their final content and validation outcome were not stable.
This register therefore makes no final completeness claim for those six work
items. T044, T045, T046, T047, T052 and T067 stay unchecked pending that
agent's accepted outcome.

### 18.1 C2 HIGH: Credential controller and observability completion gaps

| Work item | Verified state | Closing condition |
| --- | --- | --- |
| `ADR046-credential-006` | The neutral controller contract implements reconcile, observe, revoke, single-flight, exact subresource admission, idempotency derivation and bounded retry decisions. Its `rotation_policy_matrix_is_closed` test contains four timing cases only. It is not the required complete proactive, on-demand and on-expiry policy matrix crossed with success, locked, unavailable and expired outcomes, and no controller state-machine golden vectors pin complete decisions. The three Provider controllers delegate to this shared helper and add no complete matrix of their own. | Add canonical controller decision vectors and the complete 3 x 4 policy/outcome matrix, including the typed rotation-failure path, then execute them in an enforcing Rust lane. |
| `ADR046-credential-008` | All three Provider crates contain audit and telemetry builders, and `packages/d2b-contract-tests/tests/credential_audit.rs` structurally checks the common record, descriptors, collector fields and identity canaries. The production service `dispatch`, acquire, refresh, revoke and inspect paths call none of those builders, and the Provider binaries still report that production runtime wiring is unavailable. No production Credential controller or service path emits a record or frame into the Zone audit/OTEL sinks. | Wire authorized service and controller transitions to the Zone audit/OTEL sinks, preserve denied-request identity silence, and add execution evidence that the production path emits the validated frames. |

### 18.2 D1 HIGH: Network integration and latency evidence gaps

| Obligation | Work items | Verified state | Closing condition |
| --- | --- | --- | --- |
| mDNS integration | `ADR046-network-003`, `ADR046-network-005`, `ADR046-network-006` | Hermetic controller fakes record the mDNS toggle, and legacy nix-unit cases inspect old inline service behavior. No executable integration test proves creation and deletion of the separately owned mDNS Process resources required by the v3 controller contract. | Add an executable repository-routed integration scenario for enable, disable and finalizer deletion ordering. |
| p95 hint-to-handler latency | `ADR046-network-005` | No production benchmark enforces the named p95 threshold. `tests/unit/gates/performance-budgets.sh` does not measure this path, is classified advisory in `tests/layer1-jobs.json`, and exits with `SKIP` unless `D2B_PERF_STABLE=1`. The project has no pinned stable runner, so this obligation is unmeetable in the current gate and cannot be cited as validation evidence. | Land the production hint-to-handler benchmark, provision a pinned stable runner, enable it there and promote the job from advisory before citing a result. |
| Container network lifecycle | `ADR046-network-005`, `ADR046-network-006` | `packages/d2b-provider-network-local/integration/host_fabric.rs` is a constant-list Rust test, is not a Cargo integration target from that directory and is routed by no repository lane. There is no `tests/integration/containers/` runner for the required bridge, east-west, nftables, persistent-TAP and macvtap lifecycle scenarios. | Add the executable container fixture under the repository Layer 2 lane and run `make test-integration`. |
| External-NIC host integration | `ADR046-network-009` | Hermetic admission tests cover selected same-Zone and cross-Zone claim decisions, but no host-integration scenario covers fake macvtap-parent create, disruptive update, delete, status transitions and raw-identity exclusion. | Add the named runNixOSTest scenario and run `make test-host-integration`. |

These obligations cover `ADR046-network-003`, `ADR046-network-004`,
`ADR046-network-005`, `ADR046-network-006` and `ADR046-network-009` as a
closing set. `ADR046-network-004` also has the evidence omission recorded in
section 18.3; its emitter-to-example integration cannot be treated as complete
without that named flake lane.

### 18.3 C3 HIGH: Validation evidence lane corrections

| Work item | Omitted obligation | Correction |
| --- | --- | --- |
| `ADR046-network-004` | Its generated validation field explicitly requires `make test-flake` with the updated examples, but the collected controller implementation evidence omitted that command. | Run and import `make test-flake`; another Nix or drift result is not a substitute for this named example-evaluation obligation. |
| `ADR046-credential-008` | The collected evidence cited `make test-rust` and `make test-policy`, but neither executes `packages/d2b-contract-tests/tests/credential_audit.rs`. `make test-rust` explicitly excludes the fixture-dependent `d2b-contract-tests` crate. `make test-policy` selects only its closed list of policy binaries and does not select `credential_audit`. | Run and import `make test-fixture-contracts`. `tests/AGENTS.md`, `tests/test-rust.sh` and `tests/layer1-jobs.json` identify that enforcing fixture-contract lane as the job that builds `D2B_FIXTURES` and executes the full contract crate. |

### 18.4 G1 MEDIUM: Live broker operations retain stale generated descriptions

The finding's characterization held. `ApplyNftablesProjection`, `CreateBridge`,
`DeleteBridge` and `DeletePersistentTap` all dispatch to live handlers in
`packages/d2b-priv-broker/src/runtime.rs`, and
`docs/reference/broker-w2-dispositions.md` correctly marks each one
`promoted-live`. The contrary text is descriptive only:

- `packages/d2b-contracts/src/broker_wire.rs` still calls each operation a
  typed `Unimplemented` stub in its source doc comment.
- `docs/reference/schemas/v2/wire-protocol.json` contains the same stale
  descriptions because schemars copied those comments into the generated
  schema.

This is stale source and generated documentation, not a functional dispatcher
defect. Fix the Rust doc comments first, then run
`cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-schemas` to
regenerate `wire-protocol.json`; hand-editing the generated schema would leave
the canonical source stale and fail the drift contract.

## 19. Historical Provider and cutover debt

This section retains technical findings from the implementation record. Shared-file
decisions follow source ownership, contract compatibility, and focused validation.

### 19.1 Shared dependencies and validation

The implementation record identified shared broker, Provider, store, and
cutover surfaces that require one source owner and typed contracts. The
technical obligations below remain actionable and are closed by their owning
contracts and focused checks.

### 19.2 The canonical crate names are the ones that exist

**The ruling.** Where a destination names a crate that does not exist and a
near-synonym does, the existing crate is canon:

| Manifest also spells | Canonical crate | Evidence |
| --- | --- | --- |
| `d2b-bus-session` | **`d2b-session`** | 19 source files, workspace member at `packages/Cargo.toml:45` |
| `d2b-client`, `d2b-bus-client` | **`d2b-resource-client`** | 6 source files, workspace member at line 28 |
| `d2b-zone-router` | **`d2b-zone-routing`** | 4 source files, workspace member at line 27 |

Every alternative spelling is absent from `packages/` and from the workspace
member list. This is the AGENTS.md "Existing code is canon" rule applied
directly: the manifest prose disagrees with committed, passing code, so the code
wins and the drift is recorded rather than resolved by creating a second crate.

**Why this is worth a ruling rather than a slice-local judgement.** The failure
mode is not ambiguity, it is duplication. A slice that honours the absent
spelling creates a parallel session, client, or router implementation, and the
program then has two homes for one concept - exactly the outcome the controller implementation
bundle-DTO ruling in section 14.6 was recorded to prevent.

**Class: specification correction.** Code kept; the manifest destinations are
stale prose for these entries only.

### 19.3 The zone bundle DTO is `packages/d2b-contracts/src/zone_bundle.rs`

**The ruling.** The crate was already decided in section 14.6 (`d2b-contracts`).
What remained open is the module path, which the manifest spells two ways:
`ADR046-cli-011` names `packages/d2b-contracts/src/zone_bundle.rs` and
`ADR046-volume-006` names `packages/d2b-contracts/src/v3/zone_bundle.rs`.
Neither file exists. **The crate root wins.**

**What was verified.** controller implementation landed the sibling bundle DTO at
`packages/d2b-contracts/src/generation_bundle.rs` - the crate root, not `v3/`.
The `v3/` directory holds the resource object model: `resource.rs`,
`resource_ref.rs`, `resource_schema.rs`, `resource_status.rs`, and the
per-resource DTOs. A bundle is an emitted configuration artifact, not a resource,
so root placement is the convention the tree already shipped and `v3/` would put
one concept in two namespaces.

**Class: specification correction.** Code kept; `ADR046-volume-006`'s `v3/`
spelling is stale for this entry only.

### 19.4 `redb` lands with the slice that consumes it, not in prep

**The ruling.** The `redb = "=4.1.0"` dependency, and any `cargo-deny` license or
advisory allowance it needs, land in the **`ADR046-store-004` slice commit**, not
in a prep commit.

**What was verified.** `packages/d2b-resource-store-redb/Cargo.toml` currently
declares no `redb` dependency, and `packages/Cargo.lock` has no `redb` entry, so
nothing in the workspace consumes it today. The version is pinned exactly at
`=4.1.0` in `proofs/redb-resource-store-spike/Cargo.toml` and resolved to
`4.1.0` in that spike's `Cargo.lock`; "the already-pinned redb API" in the work
item's detailed design refers to that pin.

**The precedent this follows.** The provider implementation prep commit's focused review ruled exactly this
for `rtnetlink` and `nftnl`, and the reasoning is recorded inline in
`packages/Cargo.toml`: a new third-party dependency ships with the scope commit
that first consumes it, so the license and advisory whitelist update lands in the
same commit as the consumer. `rustix` went into that prep only because prep stubs
had to compile against it. No W5 prep stub needs `redb`, so the exception does
not apply and only one slice consumes it.

**Class: inference.** Defensible from an explicit prior focused review ruling on
materially identical facts, but not a stated rule.

### 19.5 The corrected SPIKE-01 rerun is a measurement of record

**The ruling.** `ADR046-store-004` remains responsible for a rerun that must
pass the unchanged 24,576 KiB whole-process maximum-RSS threshold with no
baseline subtraction. That rerun is a **measurement of record**, and the
corrected fixture, recorded result, retained canonical figure, and focused
validation must agree. Any one alone is incomplete.

**What was verified.** `RESULTS.md` records the canonical run as MEASURED-FAIL at
25,216 KiB (24.625 MiB), 640 KiB or about 2.6% above the gate.
`RESULTS-corrections.md` records a **prototype** of four corrections measuring
18,468 KiB, a pass with 6,108 KiB (24.9%) of headroom. That document is explicit
and correct that it has no authority over the canonical measurement, does not
supersede `RESULTS.md`, and does not reopen implementation scope. Changing the
canonical figure requires the owning product evidence and focused validation to
be updated together. This ruling does not weaken any of those bounds.

**The documentation consequence, which is easy to miss.** The whole-process RSS
value is a canonical feasibility measurement. Changing it requires the retained
result, the owning product evidence, and every affected focused reference to
agree; changing one record alone would leave the register contradictory.

**The finding the corrections document already surfaced, which the implementation phase inherits.**
Correction 3, shared immutable ChangeBatch fan-out, contributes **zero** to the
gate number, because `src/bin/rss-fixture.rs` registers its watches only after
its last write, so `dispatch_watch` is never called with a non-empty watch list.
The entire recovery comes from corrections 1 and 2. The canonical fixture
therefore under-tests the design it gates. This is recorded as an **unmet
obligation** against `ADR046-store-004`: the clone-per-watcher path needs
coverage that actually executes it, and a fixture that cannot exercise a
correction cannot be cited as evidence that the correction works.

**Class: unmet obligation**, for the fan-out coverage gap.

### 19.6 Hard latency targets without a pinned runner are recorded, not met

**The ruling.** W5 work items naming hard p95 or p99 latency targets are
delivered with their obligation **recorded as unmeetable** in this register,
exactly as controller implementation did, rather than marked met on advisory evidence.

**What was verified.** AGENTS.md states that `test-performance-budgets` prints
`SKIP` and enforces no latency budget unless `D2B_PERF_STABLE=1`, that promoting
it requires a pinned self-hosted runner with that variable set, and that "the
project does not currently have such a runner."
`tests/layer1-jobs.json` classifies the job `advisory`, and an advisory result
must not be cited as validation evidence.

**What this does not license.** It does not license deleting the target,
loosening it, or asserting it from a developer-workstation number. The obligation
stays open and visible until a runner exists.

**Class: unmet obligation.**

### 19.7 `ADR046-zone-control-015` stays blocked pending a contract update

**Disposition: closed by the contract update recorded in ADR 0050.** The item remains
implementation work owned by W5, but it is no longer blocked on an absent
derivation contract; the applied section 4.9 and section 15.8 scenarios are
the contract it consumes. The dependent 016 and 021 records may likewise cite
the corrected Phase 3 launcher and configuration scenarios.

**The ruling.** The item is **not** delivered in W5 on invented facts. Section
12.3 and the required-outputs register row already record the gap: the required
derivation outputs have no path, filename, output name, or layout anywhere in the
specification set, and the recorded remedy is a contract update **before**
`ADR046-zone-control-015`.

**The consequence for two dependent items, stated explicitly.**
`ADR046-zone-control-016` lists `ADR046-zone-control-015` among its
prerequisites, and `ADR046-zone-control-021` depends on `016` in turn. Both are in
`wi:core-config-hub:w5`. A slice that delivers `016` while `015` is blocked is
building on a contract that does not exist yet, so the blocking status propagates
and must be reported rather than absorbed.

**Class: specification gap.** The remedy is a contract update owned by the
product surface, not implementation-slice work.

### 19.8 The security-key semantic projection is not invented

**The ruling.** Section 13.3 records that security-key could not construct a
signed projection factory because no backing set was defined. W5 does not
invent a non-empty set. ADR 0051 Amendment A instead makes the empty
provider-neutral set determinate and deny-all, which unblocks construction
without assigning the physical `Device` to the semantic Service.

**Class: specification gap, discharged by ADR 0051 Amendment A.**

**Discharged by ADR 0051 Amendment A.** The security-key projection factory is
now admitted with an empty `allowedBackingRefTypes` set, and any attempted
backing reference remains denied.

### 19.9 What current implementation takes on from the standing register

The current implementation inherits a large share of the accumulated debt because the items
deferred "pending the durable store" name destinations in the production store chain. They
remain listed explicitly so their owners and focused closure conditions are visible:

| Debt | Recorded at | Owner in W5 |
| --- | --- | --- |
| `zone-bootstrap` / `zone-enroll` handler still unimplemented | `ADR046-routing-016` row, ruled W5 | alongside `ADR046-store-004` |
| Sealed enrollment record does not bind the child uid | provider implementation carry-forward table | `ADR046-store-004`, `transaction.rs` |
| No durable persistence for enrollment | provider implementation carry-forward table | `ADR046-store-004` |
| Appended Zone tags cannot reach the handshake offer encoder | provider implementation carry-forward table | `ADR046-exec-018` |
| `volumeAttachmentDefaults` entry shape undefined | primitive-surface inference table | `v3/volume.rs` |
| `SensitivityClass` admits values only `private` attests | primitive-surface inference table | `v3/volume.rs` |
| `RepairPolicy` / `CleanupPolicy` / `AdoptionPolicy` / `EntryRestartPolicy` / `LeaseClass` / `Invariant` value sets exemplified, never enumerated | primitive-surface inference table | `v3/volume.rs`, `v3/process.rs` |
| MTU bound `576..=9216` inferred | primitive-surface inference table | `v3/network.rs` |
| `ExpirySpec.hardDeadlineMs` lease cap not a stated rule | primitive-surface inference table | `v3/credential.rs` |
| Canonical JSON relies on the Nix builtin rather than a canonicalization implementation | primitive-surface inference table | `zone-resources-json.nix` |

The four items in the production store chain -
`ADR046-store-004`, `ADR046-store-002`, `ADR046-store-005`, `ADR046-reconcile-003` -
are the current technical dependency chain. Until they land, much of the earlier
implementation is proven only against test doubles, which is the honest reading of
their validation evidence rather than a criticism of it.

### 19.10 Summary of what this section adds to the register

- Three **specification corrections**: the canonical crate names (19.2), the
  zone bundle module path (19.3), and by extension the manifest destinations that
  spell them otherwise.
- Three **unmet obligations**: the ChangeBatch fan-out coverage gap that the
  canonical RSS fixture cannot exercise (19.5), the latency targets that no
  runner can measure (19.6), and the standing debt table in 19.9.
- The derivation output layout gap (19.7) is closed by accepted ADR 0050 and
  its applied amendment.
- The security-key backing-set gap (19.8) is discharged by ADR 0051 Amendment A.
- Three **inferences** to confirm: per-snapshot preparation against the
  one-snapshot requirement (19.1) and the `redb` dependency placement (19.4).

### 19.11 The corrected RSS measurement of record is release-validation work

**The ruling.** The `ADR046-store-004` slice implements the four corrections and
may take a **provisional** RSS reading for local feedback. The **measurement of
record** is taken during release validation on a quiet machine, and the machine
state is recorded beside the number.

**What went wrong first, recorded because the implementation phase nearly shipped on it.** The
slice was originally instructed to guard the measurement by checking
`pgrep -a cargo` and to "wait and retry" if the machine was busy. That guard was
wrong in three independent ways.

1. **It watches the wrong processes.** `cargo` forks `rustc`, and the memory is
   in the children, not the parent. At the moment this was caught the host was
   running five `rustc` processes at 1.33 GB, 1.24 GB, 838 MB, 788 MB and 568 MB,
   plus a `nix build` at **8.8 GB** resident and a separate `nix eval` at 3.0 GB.
   None of those are named `cargo`. The check reported three processes and would
   have read as near-idle.
2. **The wait is unbounded and belongs to nobody.** The load came from unrelated
   worktrees (one running a full workspace `nextest`, and another running a Nix
   eval). Those are the operator's, they run for
   hours, and a slice agent has no authority to wait on them or standing to
   decide when they are done.
3. **The bias runs toward a false pass, which is the unsafe direction.** Under
   memory pressure the kernel reclaims, so a whole-process **maximum** RSS
   high-water mark reads *lower*, not higher. A measurement taken on a loaded
   machine is therefore biased toward passing the 24,576 KiB gate. That gate is
   what unblocks the implementation phase's entire critical path, so a falsely-passing reading
   would have been the most expensive possible defect to carry forward.

**The precondition, stated so it is reproducible.** Before the measurement of
record: no `rustc`, `cargo`, `nix build`, or `nix eval` process belonging to any
worktree; `/proc/pressure/memory` `full avg300` at or near zero; and swap-in
quiescent. Record `nproc`, load average, `free -m`, and the memory-pressure
figures alongside the median and the full spread. State the machine was quiet as
part of the evidence, not as an aside.

**Class: unmet obligation** until the measurement of record is taken under that
precondition. The provisional slice reading is explicitly not evidence.

<!-- RETIRED-READONLY-END -->

## 20. current implementation destination and rename drift, and two corrections to earlier entries

The full adjudication is
[`amendment-w5-destination-drift.md`](./amendment-w5-destination-drift.md). Only
what changes an existing entry in this register is restated here.

**Class: specification drift.** Eleven W5 items and two already-`Merged` items
carry destination paths naming crates that do not exist, where a committed crate
covers the same obligation under a different name: `d2b-bus-session` and
`d2b-bus-session-unix` against `d2b-session` and `d2b-session-unix`,
`d2b-bus-wire` against `d2b-bus`, `d2b-zone-router` against `d2b-zone-routing`,
`d2b-client` and `d2b-bus-client` against `d2b-resource-client`,
`d2b-bus-contracts` and `d2b-zone-service` against `d2b-resource-api`, and
`d2b-provider-runtime` and `d2b-provider-agent` against `d2b-provider`.

The historical record does not decide this class, and that is the entry worth
carrying forward. Both sides are task records: `ADR046-exec-016` names
`packages/d2b-bus-session/`
while `ADR046-session-001`, `Merged` in W1, names `packages/d2b-session/`, and
Both member specs were authored against different naming proposals and the task
export carried both forward faithfully. The deciding rule is therefore
"existing code is canon", with the
committed crate as the destination - except where the member spec's own
detailed-design text permits the current name, in which case there is no drift.
`ADR046-exec-016` and `ADR046-exec-017` are that exception: both say "rename
crate ... or retain name", and `ADR046-exec-017`'s Removal proof says "if the
name is retained, no prior owner is removed".

### 20.1 Correction to section 10.3: the crate-layout policy record is stale

Section 10.3 recorded that `ADR046-pkg-001` named
`packages/d2b-contract-tests/tests/policy_provider_crate_layout.rs` while the
slice shipped `policy_provider_crates.rs`, routed through "the **advisory**
`test-fixture-contracts` lane". Both halves have been overtaken:

- The named file now exists. `2232c8c1` added it, 533 lines, driven by Cargo
  metadata and an on-disk `packages/` scan so a crate omitted from the workspace
  cannot escape coverage.
- `test-fixture-contracts` is no longer advisory. `tests/layer1-jobs.json`
  classifies exactly one job advisory today, `test-performance-budgets`.

Section 10.3's reasoning is retained with the current lane classification. Three
locations are live and each has a distinct role - the xtask entrypoint at
`packages/xtask/src/provider_crate_policy.rs`, the dependency-allowlist and
naming-exemption policy at `policy_provider_crates.rs`, and the layout policy at
`policy_provider_crate_layout.rs`. Together they provide the provider layout
policy and its focused coverage.

### 20.2 Two obligations that are absent rather than relocated

Recorded separately because the rest of the drift resolves to a rename, and a
reader who generalises would conclude nothing is missing:

- `ProcessAttachClient`, `ADR046-exec-022`'s fourth prescribed rename, has no
  counterpart anywhere in `packages/`. The other three renames it names all
  landed in `d2b-resource-client`.
- `nixos-modules/options-volumes.nix` does not exist and no other module covers
  it. `ADR046-volume-004` and `ADR046-vvfs-006` both name it for user-facing
  volume and attachment options. Its sibling destination
  `nixos-modules/resources-volume.nix` does exist and is substantial, so a scan
  that stopped at "one destination of two is present" would have marked the row
  covered.

`nixos-modules/resources-zone-control.nix` at `ADR046-zone-control-007` is a
third case, between the two: no file answers to the name, but Zone-control
resource handling is spread across `options-zones.nix`,
`options-zones-resources.nix`, `resources-zones-processes.nix`,
`resources-zones-volumes.nix` and `generated/`, and `index.nix` already computes
`declaredZones` and `zoneRows`. The implementation must bind the destination to
that set or accept a new file; the source record does not decide ownership.

### 20.3 A normative numeric conflict: the configuration-cleanup stall threshold

**Class: specification drift, and an unmet reconciliation obligation.** Full
record in
[`amendment-w5-destination-drift.md`](./amendment-w5-destination-drift.md)
section 7; only what a later reader must not miss is restated here.

The configuration-cleanup stall threshold has two published defaults.
`packages/d2b-core-controller/src/cleanup.rs:257` carries
`CONFIGURATION_CLEANUP_STALL_THRESHOLD_MS_DEFAULT = 600_000`, ten minutes, and
`ADR-046-resources-host-guest-process-user` agrees. Three statements in
`ADR-046-resources-zone-control`, one in `ADR-046-security-and-threat-model`,
and `ADR046-zone-control-016`'s `detailedDesign` all say five minutes.

**Ruling: committed passing code is canon for this implementation phase. The constant stands at
600,000 ms and no runtime behaviour changes.** The historical record does not
select between the retained design values -
`ADR046-exec-015` carries "10 min default" and `ADR046-zone-control-016` carries
"default 5 min" in their retained design records. Nothing in the specification
set gives one member standing over another on a shared default, so the owning
product contract must settle the value before the option is wired.

Three things worth carrying forward.

- **Nothing production reads the constant.** Its only two call sites are inside
  one `#[cfg(test)]` unit test, `cleanup_stall_due` takes the threshold as a
  parameter, and `cleanupStuckThreshold` appears nowhere outside `docs/specs/` -
  no Nix option, no schema field, no CLI surface. No operator is affected today
  in either direction, and the conflict is therefore cheap now and expensive
  once the option is wired, when changing it becomes a consumer-visible default
  change under the deprecation policy.
- **The failing test is the guard.** `ADR046-zone-control-016`'s destination
  explicitly names `packages/d2b-core-controller/src/cleanup.rs` while its design
  says five minutes, so an implementer working it literally will edit the
  constant and break
  `configuration_stall_clock_is_bounded_and_clock_injected`, which pins the
  transition at exactly `00:10:00.000`. Retuning that test to five minutes is the
  tempting repair and is exactly how a halved operator-visible default ships
  green and unrecorded. That test is the only artifact in the tree asserting what
  the default is.
- **Implementation constraint while this debt remains**: do not move the
  constant, retune the test, or wire the option. A candidate that does any of
  the three is incomplete until the product contract and focused evidence agree.

The product contract must pick one value. The recommendation recorded, and it is
a recommendation only, is five minutes with the code changed to match in a
deliberate separate change carrying a changelog entry: crossing this threshold
sets a Degraded condition and never refuses or force-clears a finalizer, so a
false positive costs one investigation while a false negative hides a stuck
finalizer for twice as long, and the shorter window is the fail-closed direction
for a threshold that surfaces rather than denies.

## 21. C1 code-canon gap: add the omitted system-core handler names

**Status: planned/assigned, not delivered.** The earlier read-only analysis correctly found
that the committed unreleased v3 `ZoneHandlerName` enum cannot encode the two system-core
handler observations. The retired current implementation repair did not land.

Code canon still lacks the contract correction. Ownership resolves only from
authoritative member specifications and focused implementation evidence. The required values are
`ZoneHandlerName::SystemCoreHost` and
`ZoneHandlerName::SystemCoreUser`, serialized by the existing kebab-case rule as
`system-core-host` and `system-core-user`. The actual projection is the
`Zone.status.handlers[]` list, with exactly one record of each name and each record carrying
`phase` and `lastReconciledAt`. Duplicate or missing required records and wrong-name
substitution are rejected. `ProviderLifecycle` remains a distinct allowed value and cannot
substitute for either required record.

The implementation plan also covers focused Rust round-trip/list coverage, the ownership-compatible
`packages/d2b-contract-tests/tests/policy_contracts.rs` guard and
the existing paired `docs/reference/resource-plane-runtime.md`. It treats
`packages/xtask/src/zone_schema.rs` and
`docs/reference/schemas/v3/core.d2bus.org_Zone.schema.json` as read-only proof inputs and must
show that generator output remains byte-identical because the desired Zone spec is unchanged.
The current implementation ownership map remains historical context; implementation
and evidence import follow authoritative member specifications and focused checks.

No `apiVersion`, `schemaVersion`, `manifestVersion`, `bundleVersion`, or wire-field version
bump is required: no field or operation changes, the desired-state Zone schema is unchanged,
and v3 is unreleased. Ownership of paired Rust contract changes, tests,
reference status docs, consumers/emitters, and generator no-drift proof resolves only from
authoritative member specifications and focused checks.

CHK054 is checked as a specification-quality resolution. Prospective implementation
resolves from authoritative objects.
