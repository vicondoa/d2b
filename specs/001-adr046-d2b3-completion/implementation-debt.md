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
that exists in no crate. That catalogue is this register's only entry with no
owning wave, and it needs an integrator ruling rather than a schedule.

## 1. Blocked and unlanded dependencies

These prevent a work item from being completed at all, or force it to ship a
hole. Each must be closed by the wave named.

| Item | Debt | Owning wave |
| --- | --- | --- |
| `ADR046-routing-007` | CLOSED. The dependency declarations landed as integrator prep and the contract module landed first, on the reading that the recorded edge is inverted for the contract. | Closed in W2 |
| `ADR046-routing-009` | Both landed, 009 first. The recorded edge remains wrong in the graph: it says 009 depends on 007, but 007 imports 009's contract. The manifest should be corrected as a separate amendment under the drift rule. | Amendment, not blocking |
| `ADR046-routing-016` service | Still no handler for `zone-bootstrap` and `zone-enroll`, but the blocker moved: the session contract and enrollment machine now exist in the bus, so wiring the service to them is ordinary work rather than a missing contract. The four enrollment obligations are met in the bus session module, not in the service. | W3 |
| `ADR046-primitives-002` providers | `ProcessLaunchEffectPort` has no production adapter, so both process Providers are complete but unwired. The adapter is `ADR046-process-001`, destination `packages/d2b-provider-supervisor/`. | W4 |
| `ADR046-routing-014` | `ProviderInstance`'s eleven trait objects and the whole `RpcProviderProxy` family are not delivered. They are built on `d2b_contracts::v2_provider` types with no v3 replacement. Needs a v3 Provider-method DTO work item before it can be finished. | Needs an integrator ruling on which wave owns the v3 Provider DTO catalogue |
| `ADR046-routing-015` | `GeneratedProviderServiceServer` ttrpc dispatch not implemented: no v3 Provider proto, no service-name freeze, no generated bindings exist. `ProviderAgentAdapter`, `register_exact_instances`, and `ProviderAgentProcess` all depend on routing-014 surfaces that are themselves incomplete. | Same ruling as above |

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
| Capability snapshot needs re-approval | New public items landed in `d2b-zone-routing`, `d2b-provider`, `d2b-provider-toolkit` and `d2b-core-controller` after the last approval. The gate must be re-run and the additions reviewed with a stated reason. | W2 close |
| `drift-check.sh` does not cover the Zone generators | `gen-zone-schemas` and `gen-zone-nix-options` are not in `drift_paths`, so the gate does not regenerate their artifacts. An xtask unit test is the interim byte-for-byte guard. | W2 |
| `flake.nix` zone-schema-drift check | The work item asks for `checks.<system>.zone-schema-drift` plus a matrix pin refresh. Not added. | W2 |
| `public_mint_surface` runtime | Renders rustdoc for all 47 workspace members sequentially into isolated target dirs; roughly 30 minutes and growing with every crate added. The render phase is parallelizable; the dependency ordering is only needed for the analysis phase. | Integrator decision, standalone change |
| Unknown-spec-field rejection | Cannot be enforced while the shared `spec` type injects execution-policy defaults into every resource. Needs the generated per-type submodule to replace the freeform type, which requires editing a file the generator slice does not own. `nix-unit: zone-link-closed-spec` cannot pass until then. | W2 |
| Two engine refusal branches unreachable from outside | The contract constructors already reject the shapes that would trip them, so they guard only the deserialization path. Exercising them needs a deserialization-based vector, a different surface than the vector suite owns. | W2 panel to rule |
| Enrollment validation obligations | Four obligations unmet, blocked on the session contract. | W2, with routing-007/009 |
| `UNIMPLEMENTED_SCAFFOLD` markers | Still present in several crates, deliberately, because the capability gate fails closed on a crate advertising no public item. Each must be deleted by the slice that fills its crate. | Per slice |

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
| Appended Zone tags cannot reach the wire | The session contract appends six Zone members at new tags, but the canonical handshake offer encoder types its fields with the un-extended enums and lives in a file no W2 slice owned. Widening it in place would have invalidated the committed golden vectors. Enrolled links and bootstrap use preserved tags, so ZoneLink is unaffected; carrying an appended tag needs an owned decision on the offer encoding. | W3 |
| Session tag values are an inference | No specification fixes the numeric tags for the six appended members, nor the ZoneLink service wire string. The scheme chosen is append-only, next unused tag, never renumber, with two tags permanently reserved rather than reused. These are wire-visible and need panel confirmation before anything depends on them. | W2 panel |
| Subject-digest prologue field is a choice | The specs name no field for the subject-context digest. It is folded into the existing channel binding, which is already inside the canonical offer and therefore inside the handshake prologue, so no wire change was needed. Worth confirming. | W2 panel |
| Sealed enrollment record does not bind the child uid | The spec says the record binds the child static key pin to the child Zone uid. The session module holds the fingerprint and an opaque allocator-binding digest; the uid binding belongs to the durable store transaction owned by the ZoneLink controller. | W3 |
| No durable persistence for enrollment | Recovery takes the persisted facts as arguments. The store transaction that seals or invalidates a record is the controller's and is not implemented. | W3 |
| `component_session` runtime tests not duplicated | The bus re-exports the session runtime rather than forking it, so its 2,121 lines of tests were deliberately not copied; they run in the owning crate. The ported golden vectors are the port evidence. A scope judgement worth a reviewer's confirmation. | W2 panel |
| Principal digest has no frozen domain tag | The cross-Zone idempotency key needs a subject digest, but the frozen digest-tag list has no principal or subject tag, so the digest is currently undomained. If a tag is later frozen, the computation changes. | W3 |
| No closed reason for a multi-Zone batch | The routing reason enum has no variant for a batch spanning Zones, so a structural error is returned rather than misusing an unrelated routing reason. | W3 |
| `approved-public-api.txt` regeneration | Two slices added public items to the bus and flagged that the crate's public-API snapshot needs regenerating. Distinct from the capability allowlist, which they reported should be unchanged. | W2 close |
| Unix session tests delegated rather than ported | The manifest asked to port the unix session tests verbatim, but the integrator wired the owning crate as a dependency instead, so copying them would fork the audited substrate. Zone-level semantics were ported instead. Needs a ruling: accept delegation, or add the syscall dev-dependency and port literally. | W2 panel |
| Listener portal transport variant | One spec describes a pre-bound socket handed over a portal call while another describes an inherited connected socket. Only the connected form is implemented; the portal wire contract belongs to a transport Provider crate and is unspecified for the bus. | W6 |

## 7. Known flakes observed during Wave 2

Recorded so a later run does not rediscover them as regressions.

| Test | Observation | Assessment |
| --- | --- | --- |
| `d2b-unsafe-local-helper::shell_supervisor real_supervisor_preserves_pty_across_reconnect_and_kills_exact_scope` | Failed once with "supervisor did not exit" during a full parallel run, passed 3 of 3 in isolation, and passed on the next full run of all 4499 tests. | Environment-sensitive, not a wave regression. The wave never touched that crate, and the test spawns a real supervisor in a transient scope and waits for it to exit, which is timing-sensitive under heavy parallel load. Worth a bounded wait rather than an unbounded one if it recurs. |
| Capability seal fixtures | Two seal fixtures reported a downstream compile failure that read as a trust-boundary regression. The real cause was a stale fixture lock after the bus gained dependencies. | Fixed. Worth knowing that this failure mode is indistinguishable from a genuine seal break in its message, so a stale lock should be ruled out first. |
| Compile-fail tests under a caching compiler wrapper | A capability seal failed once with a wrapper client exiting nonzero under concurrent cargo invocations. | Mitigated earlier in the program by clearing every wrapper spelling for those spawned compilers. The original failure was never reproduced on demand, so that remains a reasoned mitigation rather than a demonstrated fix. |
