# Changelog

All notable changes to d2b are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## [Unreleased]

### Added

- Added `make generate`, a local Bazel aggregate for regenerating committed
  schemas, docs, completions, protocol bindings, Nix resource outputs, and
  policy inputs without changing the remote-default `make check` profile.
- Added `labs/d2b-agentterm`, an experimental crate exploring a terminal that a
  human and an AI agent can drive at the same time. It wraps a program in a
  pseudoterminal and passes it through unchanged, so a person interacts with the
  program normally, while a headless terminal emulator consumes the same output
  and answers structured questions over a unix socket: what is on screen, what
  changed over a trailing window, and when the screen last settled. It also
  accepts injected keystrokes, which merge into the same input queue as the
  human's so the two can never interleave mid-escape-sequence. It has its own
  Cargo workspace under `labs/`, is outside CI, and changes no shipping
  component.

- Added test-only, production-unwired workspace destinations for `d2b-bus`,
  `d2b-session`, and `d2b-session-unix`.

- Added a standalone redb resource-store feasibility spike covering the ten-table
  physical schema, bounded fair async writer and group commits, revision-backed
  watches and controller hints, crash-boundary recovery fixtures, scale/RSS
  fixtures, and commit-to-handler latency measurements. Functional, watch,
  conflict, crash-recovery, group commit at 48/50, 96%, and latency thresholds
  passed, but whole-process RSS was 25,216 KiB (24.625 MiB), 640 KiB or about
  2.6% above 24,576 KiB. That failure blocks the production backend and watch
  dispatcher; subtracting a process baseline is not permitted. The redb
  dependency remains isolated to the disposable proof workspace.

- Added the self-contained, test-only per-Zone `d2b-bus` exact-address router with native
  Role/RoleBinding authorization, relay and diagnostic verb enforcement,
  single-owner authenticated registration, revision-bound revocation,
  pinned transport-correlated cancellation routes, reconnect replacement, and
  credit-bounded fair named streams, now connected to authenticated
  ComponentSession capabilities through consuming registration and
  reconnect/disconnect lifecycle handling.

- Added the test-only, production-unwired transport-neutral ComponentSession v3
  contract and runtime with
  strict Noise NN/KK/IKpsk2 authentication, replay-safe records, native
  authorization leases, fair named streams, cancellation, deadlines, and
  reconnect handling with redacted handshake and record diagnostics. Added Unix
  seqpacket, stream, socketpair, and vsock adapters with consumed
  peer-to-subject mapping, exact descriptor identity, multi-scope attachment
  credits, and fail-closed transport and socket-activation descriptor cleanup.

- Added the test-only, production-unwired async controller toolkit and core
  reconciliation engine with
  store-watch relisting, bounded per-resource single-flight dispatch,
  cross-resource concurrency, commit-gated expedited effects, serialized
  upgrades, owner/dependency propagation, suppression, leases, and fair hint
  admission.

- This work exposes no operator-facing capability yet. Production use remains
  blocked on authenticated ComponentSession-to-bus registration, Zone
  registration, store/watch integration, and a corrected whole-process RSS
  rerun.
- Added opaque Volume effect-port contracts and dependency-safe local Volume
  finalization.
- Added strict virtiofs Export contracts and host-side effect-port composition.
- Added exact Device and holder-bound security-key admission with fail-closed
  hidraw selection.
- Added controller-created TPM child-resource contracts and a broker-backed
  production reconcile path that preserves TPM state.
- Hardened TPM state before the first flush, routed reconcile through the
  broker-owned legacy migration journal, and bound launch tickets to the
  validated state intent.
- Refused the unbound legacy security-key broker operation and raw hidraw
  selectors until a bundle-backed stable-selector Provider path is present.
- Enforced canonical virtiofs Provider identity and mount-path validation.
- Added authenticated, Zone- and workload-bound Secret Service Credential sessions with opaque lease ownership and disconnect/finalization revocation.
- Complete the GPU Device Provider lifecycle with Host-global claim
  admission, opaque worker identities, restart adoption, bounded status and
  telemetry, and fail-closed broker preflight.
- Added separate USBIP Service and Binding lifecycle supervision so Guest attachment cleanup completes before owned device unbind and Host-global authority release.
- Added typed Core bundle projections and daemon broker composition for Host-global USBIP claims, private Binding runners, restart identity checks, and scoped cleanup.
- Add a typed qemu-media Guest runtime with broker-owned process launch,
  Host-global KVM admission, QMP health and hotplug handling, restart
  adoption, ordered finalization, and redacted audit/telemetry projections.
- Added typed shell pool and session lifecycle contracts with workload-user
  placement, bounded terminal replay, and restart-safe supervisor adoption.
- Added authenticated local Unix transport admission with bounded portal ownership and descriptor lifecycle controls.
- Added Wayland display, desktop notification, and Wayland clipboard Providers
  with bounded lifecycle state, explicit policy and RBAC, redacted audit and
  telemetry output, authenticated stream admission, and hermetic tests.
- Carry sealed typed clipboard and notification configuration through the Zone
  resource runtime, issue supervisor-authoritative notification source and
  host-sink receipts, and keep short AF_UNIX telemetry test sockets faithful.
- Hardened nonce/idempotency cleanup, display principal lifecycle reuse, and
  clipboard rate-bucket garbage collection.
- Added canonical compile-safe package scaffolds and flake outputs for the
  remaining Provider dossiers so each accepted Provider has one workspace and
  package identity before its behavior is implemented.
- Added bounded lifecycle providers for Azure Container Apps, Azure virtual
  machines, Cloud Hypervisor, and Azure Relay, including opaque effect
  contracts, redacted audit records, bounded telemetry, crash recovery, and
  hermetic lifecycle coverage.
- Route Network host effects through the typed broker adapter with
  generation-fenced persistent TAP realization cleanup and fail-closed
  NetworkManager ownership checks. Require Core-admitted Host-global
  physical-NIC claims before external attachment effects.
- Added typed activation generation, audio PipeWire, system-core, minijail,
  and systemd Provider boundaries with bounded lifecycle and readiness
  contracts.
- Added structured activation helper input/output and production Zone status
  emission for the system-core Host/User handler pair.
- Added strict resource-envelope compilation with canonical schema, version,
  unknown-field, type, required-field, and reference rejection.
- Added a single Host-global authority startup barrier and an async
  reservation lifecycle persisted through effect closure and restart recovery.
- Added the Core-issued TPM migration decision, typed production effect-port
  adapter, and broker-owned fd-relative journal replay; unbound effects remain
  fail-closed.
- Host-install no longer performs an unsealed legacy migration; absence-only
  state is quarantined until Core supplies the never-provisioned decision.
- Recovery exposes a sealed external-inventory provenance port; active
  physical-NIC rows quarantine when that port is not installed.
- Added the authenticated transport-vsock Provider with replay-safe Guest and
  Zone session admission, bounded framing, named-stream bridging, and native
  guest relay lifecycle.
- Added allocator-only transport settings and integration coverage for CID
  authority, restart matching, close ordering, redaction, and attachment
  refusal.
- Added authenticated Zone Resource API lifecycle coverage for Volume, Network,
  Device TPM, and Cloud Hypervisor Guest resources, including dependency-aware
  readiness, restart continuity, and dependency-safe removal. Cloud Hypervisor
  Guest adoption remains an explicit preflight block when nested TAP/cgroup
  posture is unavailable.
- Added the public Zone Resource API authorization row and generated privileges
  contract so typed Resource reads and mutations use authenticated daemon
  admission.
- Added logical resource-store backup validation before schema migration and
  restore checks that preserve resource identity and retained TPM state.
- Add strict candidate-bound recovery-point attestation validation, protected
  closure checks, write-once delivery records, terminal failure handling, and
  post-merge delivery closure for SPEC-001 Wave 7.
- Define signed controller scopes, target capabilities, canonical ResourceType
  placement anchors, per-target EffectPort requirements, and shared Host/Guest
  daemon and broker artifact checks.
- Add fixed Host and Guest `d2bd` modes with provider-neutral Guest
  ComponentSession enrollment, bounded target admission, and mode-bound
  broker effects.
- Route Guest Process and EphemeralProcess execution through authenticated
  Guest ComponentSession and Guest-profile broker contracts with signed
  target, session, boot, assignment, and generation binding.
- Route Zone requests through the root public listener with NSS-backed User resolution, committed RoleBinding authorization, sealed downstream leases, and a stop-only host-shutdown capability. Default ZoneBus constructors keep committed subject installation deny-all; only Zone-runtime composition receives a registrar-bound issuer that seals the verified Zone subject projection.
- Rewire the Resource API onto the per-Zone ResourceManager (U8): a
  manager-backed `ResourceStoreBackend` resolves reads, mutations, and
  external WATCH through the single-writer manager while the public
  operation shapes and wire envelopes stay intact; resource generation maps
  onto the existing Exact-revision wire preconditions and external WATCH
  carries epoch+sequence runtime revisions with `RevisionExpired` relist
  semantics (R23-R25).
- Compile the `PolicySet`, role bindings, and bootstrap facts from durable
  Role / RoleBinding / User / Provider spec rows plus the Nix bundle
  generation (KTD6), with the one-way bootstrap latch: a published policy
  revision permanently disables bootstrap and a provisioned Zone never
  re-enters Unprovisioned (R28).
- Land the manager-backed Resource API surface on the daemon (U8/U9 F1
  wiring): the per-Zone runtime resolves each plane's manager client and
  watch hub from the published `v3_planes` table, holds a second
  `NativeAuthorizer` whose policy mirrors the primary projection on every
  install, and routes public resource requests by type - the converted Phase
  A types ride the manager, everything else stays on the redb service. The
  per-type partition is now enforced by routing instead of by refusal.
- Project the new plane's views onto the public read contract: manager rows
  render the canonical envelope (Nix-materialized rows persist the compiled
  spec beside its authored metadata) with `type`, `metadata.uid`,
  `metadata.generation`/`revision`, and the actor's in-memory status
  (`phase`, `observedGeneration` = row generation, R11). A failed actor's
  status also carries the closed failure classification
  (`driverFailure: {operation, retryable}`): status is never persisted (R11),
  so this is the only way an operator or fixture can see why a resource
  failed.
- One test per posture asserts the whole decision set at once - descriptor
  contract, escrow custody, response indices, credentials, namespace
  mapping and admission fences - so dropping any single decision fails the
  test rather than surfacing later as a decode or identity error.
- A Guest realized through the manager now establishes its own authenticated
  ComponentSession, and the acceptance lane can see it. The converted Guest
  effect path resolves the committed Guest and its guest-control Endpoint,
  connects (reusing the live session cache), registers the live generation on
  the Zone target directory, and re-adopts every assignment the directory
  holds for that Guest - so a target-local realization present inside the Guest
  is re-bound to the new generation and a missing one stays for its owning
  actor to realize again, never inherited across a reconnect.
- The target-control session generation is published on every acceptance, not
  only on a replacement: `Guest ComponentSession Resource API server starting`
  is emitted at `info` with the accepted route's own `generation` field, which
  is the same value the Guest binds as its live target-control generation and
  the parent records in the session descriptor. Previously the first
  acceptance after boot published nothing, and the only existing event was at
  `debug` - invisible under the daemon's default `info` filter - so the host
  journal showed a bound listener with no session behind it.
- The host posture for state farms and shared directories is a declared
  contract, `packages/d2b-broker/src/ops/state-posture-contract.json`: per path
  level it names the creator, owner/group/mode, POSIX ACL entries (and who
  applies them), and the rights each host principal needs or is refused.
  One file feeds all three consumers - the broker posture code embeds and
  applies the `guest-store-view` rows, `nixos-modules/host-daemon.nix` derives
  the shared-run-dir and state-root tmpfiles lines from the declared rows, and
  the declarations are validated live.
- `tests/host-integration/state-posture-contract.nix` boots the Zone-native
  Cloud Hypervisor Guest recipe and asserts the live host against every
  declared level: owner/group/mode/ACL equality, and the allowed AND denied
  operations per principal (root, `d2bd`, a `d2b`-group launcher, and an
  unrelated user) for the per-Guest state chain, the per-Guest directory, the
  store-view farm, `/run/d2b`, and the state root. It fails if a level's
  group/mode/ACL stops matching the declaration, or if the daemon loses the
  search-only reach it has on levels it does not own.
- The private bundle resolves Device-owned worker rows. `device_worker_posture`
  (`d2b-core::bundle_resolver`) is the one closed table of the five worker
  templates a Device Provider declares - `swtpm-socket`, `swtpm-init-flush`,
  `gpu-worker`, `gpu-render-node`, `video-worker` - carrying for each the
  broker runner role, the packaged executable, the seccomp policy class, the
  namespace classes, the user-namespace requirement, the device binds, and
  the umask. `build_device_worker_intents` mints one trusted intent per
  declared row from it, and `BundleResolver::find_device_worker_intent`
  resolves a launch through the exact declared row (role id = row name) and
  the declared template, refusing ambiguity.
- `d2b-resource-compiler` projects `append_device_tpm_worker_templates` and
  `append_device_gpu_worker_templates`: for every `Process/swtpm-<device>`,
  `EphemeralProcess/swtpm-flush-<device>`, `Process/gpu-<device>`, and
  `Process/video-<device>` row the Device Provider declares, it emits one
  non-dynamic `ProcessTemplateBinding` pinning the artifact executable and
  admitting controller-supplied launch arguments. A row whose declared
  sandbox disagrees with its template's closed posture is refused at compile
  time (`provider-device-worker-posture-mismatch`), and a template whose
  executable the artifact does not package yields no binding.
- `ProcessTemplateBinding` gained `new_with_launch_args` (declared row,
  bounded launch arguments) and `Process/swtpm-<device>`-style
  `EphemeralProcess` rows are bindable. The declared-row arm of
  `ResourceBundle::verify_process_templates` now admits a Device-owned worker
  row: the binding's owner must be the Provider the owning Device's
  `providerRef` names, the row's `executionRef` and `template` must equal the
  binding's, and the row must be a `worker` class row.
- Device-owned worker rows launch with typed launch parameters instead of a
  bare template argv. `DeviceWorkerLaunch` (`packages/d2bd/src/process_provider_runtime.rs`)
  is the closed enum over the four declared families (`Swtpm`, `SwtpmFlush`,
  `Gpu`, `Video`) with one params struct each; the Process driver derives it per
  declared row (`Process/swtpm-<device>`, `EphemeralProcess/swtpm-flush-<device>`,
  `Process/gpu-<device>`, `Process/video-<device>`) from the trusted template,
  the owning Device row and the daemon's runtime paths, and the provider
  composes the generator's argv from it (`device_worker_launch_args`). The
  Process spec stays argv-free and the declared rows stay path-free by contract;
  the parameters are derived, never authored into either.
- The one-shot outcome is published as the row's status projection
  (`{"ephemeral": {"state": "succeeded" | "failed", "code": "..."}}`), so a
  reader can tell a concluded pass from a successful one. The TPM effect port's
  flush gate reads that projection rather than the phase, which is what lets a
  failed flush fail the device path.
- `tests/host-integration/device-worker-launch.nix` is the end-to-end proof on a
  live host: the compiled zone bundle carries the declared rows and their
  digest-pinned `launchArgs` bindings, the rows reach the manager with their
  Device owners, the swtpm worker really runs with the argv the Process
  controller composed (state directory, ctrl/server sockets, socket principal)
  and binds its sockets, the flush publishes its one-shot outcome, `d2b delete
  Device/...` retires the declared rows children-first and leaves no worker, and
  the GPU rows never report Ready but end Failed with a named driver failure.
- U17 launcher-sweep documentation at the two remaining direct broker
  `SpawnRunner` sites (the TPM swtpm/swtpm-flush spawn and the GPU/video
  worker spawn) while they existed. At the time of the sweep the v3 bundle
  carried no launch intent for the providers' typed templates
  (`swtpm-socket`, `swtpm-init-flush`, `gpu-worker`, `gpu-render-node`,
  `video-worker`): the resource compiler projected worker templates for
  virtiofsd and the managed-identity agent only, and the generic Process
  ticket path's template matcher knew only the legacy role names. The
  projection, its resolver posture table, and the supervisor/broker fences
  landed in the follow-up slice (`2026-09-12-u17-device-worker-rows.md`), and
  the port slice (`2026-09-12-u17-port-conversions.md`) converted both sites;
  no direct daemon spawn remains, and the launch-parameterization gap those
  ports still have is recorded there and in the plan's U17 status.
- U17 slice 2 (the port conversions) with the regression tests that pin them:
  five TPM port row tests and one Process-driver `NeverAdopt` regression
  (proven to fail before its fix). The TPM effect port and the GPU lifecycle
  port now realize their workers as the Device Providers' declared Process
  rows and observe the phases their Process controllers publish; the daemon
  has no direct broker spawn left.
- The trusted site-runtime contract `site.json` and its reader: the Process
  controller now launches the GPU worker with the Wayland socket the site
  itself provisions (`d2b.site.waylandUser` + `d2b.site.waylandDisplay`),
  instead of refusing with `device-worker-wayland-sock-unbound` for lack of a
  trusted source. The artifact is declared by the bundle index (`sitePath`)
  and emitted by `nixos-modules/site-json.nix` from the same option pair the
  session wiring uses, so the bundle cannot name a runtime directory the site
  does not create. A site without a Wayland session emits `waylandSocket:
  null` and the GPU launch still refuses by name.
- The daemon/fixture half of the spec's §36 invariant matrix is discharged:
  volume recovery over an existing host layout (adopts, never recreates, one
  layout effect across both driver lifetimes), the ZoneLink product rows
  (ordinary guest realization never creates or moves a link; link topology and
  guest availability move independently), and the shared-provider restart rows
  (the declared child set is adopted after a restart and re-committed when a
  child is missing), with the process/VM rows and the shared-backend limits
  cited to the tests and host-integration stages that already prove them.
- Recorded the architecture decision that makes resource-store writes
  reachable only through single-use authorization evidence owned by the
  storage contract crate, minted by the authorization evaluator and bound to
  one store instance, so evidence minted anywhere else is refused by that
  store rather than accepted.
- Added ADR 0050, a documentation-only decision record fixing the on-disk shape
  of a Provider Nix derivation so the build-time required-outputs check becomes
  writable. An artifact pins one Nix output, and where the derivation has more
  than one that output must carry evidence it was chosen, tested over `outputs`,
  `outputSpecified` and `outputName` rather than over `all`, which both rejects
  a correctly pinned selected output and throws on a store-path-valued package.
  The signed manifest, its detached Ed25519 signature, and the root config JSON
  Schema are regular files at fixed identity-independent paths under
  `share/d2b/provider/`, a closed directory that refuses any fourth entry. The
  executable set is located by enumerating `bin/`, name-checked as read from the
  directory, and required to be ELF with an execute bit, since a valid image at
  mode 0644 otherwise passes every other check and can never be launched.
  Digests have stated preimages, with a new domain tag binding the whole sorted
  name-to-digest map, and admission requires the operator pin, the publisher's
  signed claim, and the compiler's own recomputation to agree pairwise. Path
  resolution is anchored and fd-relative for both the compiler and the launcher,
  in two least-authority handle modes because a path-only descriptor cannot be
  read, behind an injectable boundary so sequencing and error mapping are
  testable without a store or a real exec. Failures are a bounded taxonomy that
  names actionable remedies, including exact toolkit commands, without emitting
  store paths, key material, or unbounded lists. A framework-owned Nix helper
  builds a conforming entry point for interpreted Providers so the ELF rule has
  a supported route. The record adds conformance scenarios for every rule it
  states and corrects five scenario identifiers cited as validation that exist
  nowhere in the specification set. No crates, services, controllers, or
  Providers are created, and no code behaviour changes.
- Added ADR 0051, a documentation-only decision record fixing the semantic
  backing contract for the provider-neutral `SecurityKeyService` and
  `SecurityKeyBinding` pair. The security-key family's closed
  `allowedBackingRefTypes` set is empty, because its provider-neutral base
  names no backing resource and the physical device belongs to the
  implementation extension; an empty allowlist denies every backing reference
  and is never read as unconstrained. The record replaces the backing
  declaration with a two-state value, requires export and backing admission to
  read the stored resource so a resource held on lease from another Zone can
  neither be re-exported nor become the backing of a new local authority
  claim, promotes the projection-protocol version to a declared descriptor
  field so a Provider artifact built for a different version is reported as
  version skew with an install-a-matching-artifact remedy rather than as a
  fingerprint mismatch, and requires every declared factory field to be
  published in the generated projection schemas. No crates, services,
  controllers, or Providers are created, and no specification or reference
  document changes.
- Added canonical Provider manifest and root configuration schema emission in
  `d2b-provider-toolkit`, plus the `manifest emit` and `manifest verify`
  authoring commands. Emission uses the exact `d2b-cjson/v1` bytes required by
  Provider artifacts, and verification reports a bounded first-divergent-byte
  offset with a direct re-emission command.
- Add a production redb resource-store measurement surface that exercises
  bounded replay, shared watch delivery, admission backpressure, and
  deterministic slow-watcher recovery alongside whole-process memory
  accounting.
- Start each configured Zone's production resource runtime from the broker-owned
  store descriptor, with restart adoption, bounded readiness, and fail-closed
  CLI resource routing.
- Added the asynchronous v3 resource API contract, typed client and service
  admission layer, storage-neutral backend interface, and literal protobuf wire
  vectors for every request, response, and supporting message.
- Added native Role and RoleBinding authorization with revision-bound
  positive-decision caching, fail-closed relay and bootstrap policy, exact
  capabilities, and sealed authorization evidence that only a real evaluation
  can issue and only its paired store instance can verify.
- Added the authenticated ttrpc resource-service adapter while leaving
  production bus and Zone dispatch explicitly unwired.
- Added canonical resource identity and reference types with bounded identifier
  parsing, typed validation errors, redacted authenticated subject context, and
  stable serialization and schema contracts.
- Added foundational Zone resource options with same-Zone reference validation,
  execution-policy defaults, and deterministic resource identity indexing.
- Added the strict v3 resource envelope, metadata, layered desired and observed
  state, canonical JSON and schema-binding contracts.
- Added singular UID-bound resource ownership, bounded owner-change hints,
  reverse indexing, cycle and depth rejection, and child-first deletion order.
- Three ADR 0046 spec-literal drift lints run as fixture-independent
  contract-test policy binaries in the mandatory policy gate: datetimes under
  `docs/specs/**` must be exactly millisecond-precision RFC 3339
  (`YYYY-MM-DDTHH:MM:SS.sssZ`), qualified ResourceType tokens must use the
  `.d2bus.org.` infix, and the retry delay must be the integer `retryAfterMs`
  scalar rather than a superseded duration-string form. Each rule enforces a
  frozen decision that a hand enumeration had previously miscounted, and the
  only exemption is the exact decision-register row that defines the rule.
- ADR 0046 delivery docs: documented the `merge-target` capture step and the
  `MergeTarget` artifact schema, so operators can see how a sealed candidate is
  bound to its pull requests and which check conclusions permit a merge.
- Contributor docs: documented the heavy-lane target structure - the single
  `heavy-gate` semaphore, the public lanes that acquire a slot, and the guarded
  internal `heavy-lane-*` targets they delegate to - in the operating manuals.
- Contributor docs: documented the ADR 0046 spec-literal lint allowlist. The
  sole exemption is the decision-register row that defines a rule in
  `docs/specs/ADR-046-decision-register.md`; there is no inline
  `d2b-lint-allow` marker, and the lint rejects that escape hatch by design.
- ADR 0046 live-test docs: the operating manuals now route `D2B_LIVE=1` live-host
  and hardware entrypoints through the heavy-gate semaphore instead of documenting
  a bare ungated invocation.
- Two ADR 0046 envelope-structure lints now close resource-shape classes that
  manual review kept missing. The first requires every Host or Guest example
  whose `allowedDomains` admits the `user` domain to carry a non-null
  `defaultUserRef`. The second requires every complete resource envelope - one
  that declares `apiVersion`, `type`, `metadata`, `spec`, and `status` - to
  carry the universal status base, including both `status.update` and
  `status.resource`. Both lints judge only complete envelopes: a focused
  fragment, a shorthand schema table, and a status body deliberately elided
  with `...` are exempt, so the lints enforce the contract without flagging
  illustrative snippets. The universal-status lint reads fenced YAML, JSON,
  and Nix documents, so explanatory prose that references a field path such as
  `Credential.status.credential.expiresAtUnixMs` under the documented
  `status.<field>` mapping convention is never flagged. The Host/Guest lint
  additionally exempts an intentional negative example - a shape authored to be
  rejected - only when the pinned documenting file carries the exact
  `d2b-lint: expect-d116-eval-error` marker, so a teaching block that
  deliberately omits `defaultUserRef` to demonstrate the eval-time failure is
  not mistaken for a real declaration.
- `cargo xtask heavy-gate verify-slot` verifies that the calling process
  genuinely holds a heavy-gate slot. It re-runs the inode, ownership, and
  atomic open-file-description lock proof through the inherited slot descriptor
  and exits non-zero (without side effects) unless a real slot is held, so a
  shell or Make guard can distinguish "already inside the gate" from "must
  acquire" without trusting an environment variable.
- Added `changelog.d/`, a changelog-fragment directory, so concurrent branches
  no longer collide in the single `## [Unreleased]` block of `CHANGELOG.md`.
  Each branch writes one `changelog.d/<branch>.md` file holding standard
  Keep a Changelog `### <Section>` headings and entries; `changelog.d/README.md`
  documents the naming rule, the accepted format, and the fold.
- Added `cargo xtask changelog-fold`, which merges every fragment into the
  `## [Unreleased]` block by section in Keep a Changelog order, appending to
  the sections already present, leaving released versions untouched, and
  deleting the fragments it consumed. Fragments are folded in file-name order
  for a byte-stable result, a run with no fragments leaves the changelog
  untouched, and a fragment with an unknown heading, a repeated heading, an
  empty section, or content outside a section aborts the run with the offending
  file and line instead of dropping the entry. `--check` validates and computes
  the fold without writing.
- Recorded twenty ADR 0046 foundation decisions (D099 to D118) in the decision
  register, closing the implementation-level contracts the first implementation
  stage needs before any of its four serialized slices can open.
- Froze the resource-plane byte formats that become permanent the moment any
  data exists: the `d2bkey/v1` store key codec, the `d2bval/v1` value frame, the
  `d2b-cjson/v1` canonical JSON profile with its domain-separated SHA-256
  digests, the resource protobuf representation and field-number policy, the
  UUIDv4 resource UID spelling, the fixed-precision UTC timestamp spelling, and
  the ResourceType segment grammars and byte bounds.
- Froze the security-critical resource-plane contracts: the literal
  bootstrap-authorization allow table, its two-phase derivation from durable
  store state, and its one-way end condition; and the store commit boundary,
  which admits only a pre-authorized mutation carrying a policy snapshot the
  write transaction rechecks without duplicating any authorization logic.
- Froze the resource API surface: the always-present common status layer, the
  outcome scalar encodings, the authenticated subject context component types,
  the service name and code-generation ownership, the v3 error model, the
  request/list/watch/batch admission bounds, the revision-log compaction
  defaults, and the owner, finalizer, label, annotation, and reference bounds.
- `make check-tier0` now scans the whole repository for every non-ASCII dash
  codepoint (U+2010, U+2011, U+2012, U+2013, U+2014, U+2015, U+2212, U+FE58,
  U+FF0D) and fails closed with every offending `file:line`. The scan covers
  every tracked file plus every non-ignored untracked file, skips binaries, and
  adds under 100ms to the gate.
- Added `cargo xtask heavy-gate`, the sole two-slot per-UID semaphore for every
  long-running validation lane. It uses open file description locks, retries
  acquisition every 250 ms up to a 30-minute ceiling, fails closed when the
  platform cannot provide those locks instead of degrading to unsynchronized
  execution, hands the locked descriptor to the child so the slot is held for
  the child's whole life, and owns the child's process group so an interrupt or
  timeout cannot orphan a running lane.
- Added the `heavy-check`, `heavy-test-integration`,
  `heavy-test-host-integration`, `heavy-test-hardware`, `heavy-cargo-test`, and
  `heavy-flake-check` Makefile targets, which route the container,
  host-integration, hardware, Rust, and building `nix flake check` lanes through
  that one semaphore so concurrent validation cannot oversubscribe the shared
  Nix store, cargo target directory, or KVM device.
- Contract types for the nine primitive resource kinds - host, guest, process,
  volume, user, network, device, credential, and the execution policy they
  share. Each validates on construction, rejects unknown fields on the wire,
  and keeps host detail out of the contract: a volume source is an opaque
  policy identifier rather than a path, a device admits no device node, and a
  user carries no numeric identifier.
- Zone routing: contracts for zone identity, capability scope, route
  advertisement and route decision, plus the engine that admits, withdraws and
  decides routes, the resolver that seals a topology and matches a target to
  its nearest entrypoint, and the service that composes them.
- Zone session contracts extending the endpoint purpose, role and service
  package taxonomies for Zone use, and the session runtime reachable from the
  message bus, including cross-Zone routing and a per-hop relay handler.
- Process providers for the two supported system managers, sharing one
  conformance suite so the same assertions run against both, and volume
  providers for local layout and exported shares.
- A provider model and a provider toolkit factoring out what the provider
  crates had each been hand-rolling.
- A resource client owning route resolution and the deadline, retry and
  cancellation machine.
- Declarative configuration for Zones: a topology option, eval-time assertions
  covering zone naming, parent topology, and uplink placement, generated
  per-type option modules and schemas, and a per-Zone resource bundle.
- The `Provider` ResourceType contract. A Provider resource authors exactly
  two fields, `artifactId` and `config`; every other Provider property is
  read-only data resolved from the signed manifest and catalog entry that
  identifier selects. The contract covers exact package, executable, manifest,
  config, schema, and service digests, publisher and trust identity, exported
  ResourceTypes with their base schema bindings, controller, service, and
  worker component descriptors, the closed dependency alias set, the signed
  standard capability matrix, registered `spec.provider` and `status.provider`
  extension schemas, export and import projection factories, and the upgrade,
  drain, and restart policy.
- Provider installation admission. Package presence alone is not installation:
  a `providerRef` resolves only a Ready Provider resource whose row selects the
  supplied manifest, whose artifact passes production trust admission, whose
  Provider API major is exact with only additive minors and no handshake
  downgrade, and whose published method surface is a subset of the surface its
  signed component graph exports.
- Provider toolkit fakes and fault injection. Hermetic fake core, resource
  store, dependency bus, supervisor, and effect clients, plus a per-call fault
  schedule, so a Provider crate can prove its controller's behaviour without a
  socket, a filesystem, a process, or a wall-clock wait. The fake bus resolves
  one declared dependency alias and never returns its binding table, the fake
  supervisor records a launch intent without spawning, and the fake effect port
  records an intent without mutating anything.
- Contract and integration coverage for a well-behaved Provider and for a
  malicious one: a self-attested or revoked artifact, an artifact negotiating
  past the required API, a Provider claiming a ResourceType twice or binding
  one it does not own, an extension schema registered for a foreign
  ResourceType, a worker granting itself a dependency portal or method
  surface, a backing Device or Binding smuggled across a Zone boundary, an
  impostor bootstrap binding, alias probing, a status write outside the owned
  set, and a capability refused without a signed declaration.
- The `system-core` bootstrap Provider now reconciles `Host` and `User`
  resources. A user-only Host always reports the no-isolation posture and
  its fixed message, a Host with any other execution policy reports no
  posture at all, and a status that tries to set or suppress either field is
  rejected rather than merged. Local User discovery reports whether the
  machine resolves a declared identity, and reports a declared group
  membership that did not verify as drift rather than as readiness.
- `system-core` refuses every resource type outside `Host` and `User`,
  including `Process`, `EphemeralProcess`, `Volume`, `Network`, `Device`,
  `Credential`, and any semantic type. The boundary is an allowlist, so a
  type nothing has claimed yet is refused too.
- Provider artifacts are now declared in Nix under `d2b.artifacts.<id>`, giving
  each Provider package a derivation plus its catalog metadata. A Provider
  resource selects one with `artifactId = "<id>"`. Nix compiles those
  declarations into an offline catalog that is sorted by identifier and
  selected by exact digest: there is no runtime marketplace, no download, no
  PATH or directory discovery, no `latest`, and no version-range solving. An
  artifact that was not declared does not exist. Malformed identifiers, missing
  or unknown catalog fields, and inexact digests are rejected at evaluation
  time with a message naming the field. The catalog's public projection carries
  no Nix store path.
- A Provider crate policy now enforces the packaging conventions across the
  workspace: every Provider crate carries `src/`, `tests/`, `integration/` and a
  `README.md` with the nine required sections, each `integration/*.rs` file
  declares exactly one orchestration target, one crate is exactly one Provider
  identity, and a Provider crate depends only on the public contract, the
  toolkits and the SDK rather than on the daemon, the broker, the store, or
  another Provider's internals. Two pre-existing crates scheduled for
  replacement are exempt with a recorded reason.
- A new build-level check proves the Provider catalog emitter is deterministic:
  two independent evaluations of the same declarations, constructed
  differently, must produce byte-identical output, and a negative control must
  produce different output so the comparison cannot pass vacuously.
- A shared semantic Service and Binding contract catalog in
  `d2b-contracts`, covering the audio, security-key, telemetry, and USB
  families. Each family publishes the provider-neutral base spec and common
  status field sets, their schema identities, versions, and fingerprints, a
  canonical minimal base fixture that carries no `spec.provider`, and the
  semantic half of its export and import projection factory. The bases are
  discoverable before any Provider package is installed, so an operator can
  read the contract a Provider must implement without first choosing one.
- Committed JSON Schema artifacts for every semantic base and projection
  layer under `docs/reference/schemas/v3/`, generated by
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- gen-semantic-service-schemas`.
  Each artifact is strict and unknown-field-denied and carries its frozen
  schema version and fingerprint, so a Provider can bind the exact identity
  it must implement.
- Fail-closed admission for a Core-generated projection Service. A projection
  admits only `providerRef`, the semantic base and import fields, and
  ResourceImport ownership; a `spec.provider` extension, an owner authority
  descriptor, or a backing reference is rejected. An export must target the
  owner Service and never a Device, an Endpoint, or a Binding.
- The semantic projection-factory fingerprint takes no Provider or adapter
  identity as an input, so selecting a different conformant implementation on
  either side of a cross-Zone export leaves it unchanged.
- Added an internal Process Provider supervisor adapter with bounded asynchronous dispatch, pidfd handoff, process identity revalidation, and service-manager identity binding, covered through hermetic adapter tests. No production runtime constructs this supervisor yet, and real broker, namespace, cgroup, and service-manager boundaries remain unverified.
- Added Network resource validation and status contracts, deterministic Linux-safe interface names, opaque attachment generation fences, and a reserved Network controller User readiness contract. These are contract and validation surfaces, not proof of a live Network Provider lifecycle.
- Added internal Volume payload-state schemas, status phases, generation-bound state envelopes, and Provider component state namespace validation for storage need, quota, placement, custody, and views. These types are not yet embedded in production Volume handling, and payload digesting remains fail-closed pending a frozen domain.
- Added strict Credential lease and status contracts plus an exact five-method Credential service implementation with one-way opaque identifiers and authorization-owned delivery bindings. The service remains unregistered and has no production bus, Provider selection, or encrypted forwarding path.
- Added internal fail-closed core controller policy modules for API publication, authorization, Provider lifecycle, ownership, watches, budgets, store admission, health, startup, and recovery. They remain library logic with no production binary, ResourceClient, authenticated connector, store transaction adapter, or watch dispatcher.
- Added local Network Provider primitives and hermetic tests for deterministic interface naming, isolated bridge ports, ownership-scoped firewall plans, route readiness, and defense-in-depth IPv6 suppression. The matching broker handlers are live, but the neutral Network effect adapter and production caller are not yet present.
- Added local Volume Provider policy and hermetic coverage for fail-closed state markers, atomic state-write sequencing, ordered locks, quota checks, redacted audit event shapes, and bounded telemetry descriptors. No neutral Volume effect adapter drives these rules against a real filesystem, and the audit and telemetry catalogues have no production sink calls.
- Added Secret Service, Entra identity-Guest, and managed-identity Credential Provider implementations with hermetic admission, delivery-binding, lease, placement, fault, and redaction coverage. Their binaries still report production runtime wiring as unavailable, so these Providers are not yet consumer-reachable Credential sources.
- Added Nix authoring for per-Zone resources, canonical integrity-pinned bundle output, and separate private artifact catalogs.
- Added eval-time Credential declaration validation and an activation authorization contract; production application still depends on the resource compiler, store, and runtime path.
- Added an internal local Network Provider controller, generic net-VM system module, typed config Volume contract, ownership-scoped firewall planning, and generation-fenced bridge and persistent-TAP cleanup logic. The controller cannot reach the live broker operations until the neutral Network effect adapter is implemented.
- Added internal fail-closed state machines and hermetic tests for Provider Volume migration, sealing rotation, snapshots, and relocation, including restart decisions, bounded retention, and source-state preservation. These protocols are not wired to a production Volume effect adapter or real filesystem boundary.
- Added internal fail-closed admission for optional Provider component state declarations. Empty or unjustified declarations are rejected, but authoritative derivability and schema-custody evidence are not yet available from production Provider deployment.
- Added a neutral Credential controller contract for exact operation authorization, bounded active-lease capacity, rotation and revocation decisions, scheduled metadata observation, and single-flight reconciliation. It has no production controller caller, and its complete policy-and-outcome decision matrix is still pending.
- Added bounded Credential audit record and telemetry frame builders with structural redaction tests. Production service and controller paths do not yet call these builders or emit them to Zone audit and telemetry sinks.
- Added internal USBIP effect and authority contracts for ownership-scoped firewall projections, relay cleanup, and confirmed projection removal before authority release. The full USBIP Device Provider and controller remain future production integration work.
- Added Host-global external physical-NIC authority admission policy that refuses cross-Zone bridge multiplexing before a host effect; live host behavior is not yet covered by an executable integration scenario.
- Added eval-time Zone-authored Network resources with canonical bundle projection, typed artifact resolution, CIDR and attachment validation, dynamic bridge prerequisites, and fail-closed cross-Zone L2 isolation. The v3 Network controller is not yet production-reachable.
- Added internal post-commit Zone configuration generation planning for configuration-owned metadata, cleanup status, per-item name-conflict isolation, and count-bounded prior bundle retention. Production store, watch, finalizer, audit, and status adapters are not wired yet.
- Added generation bundle contract validation that rejects caller-supplied lifecycle ownership metadata and checks Provider schema digests in the core planner; full build-time schema validation and executable generation activation remain pending.
- Add a resource compiler that verifies Provider artifact layout, signatures,
  canonical metadata, executable digests, and anchored launch references.
- Added a framework-owned ELF shim builder for interpreted Provider programs,
  with bounded same-output resolution and fd-relative interpreter execution.
- Added nonblocking Zone configuration generation activation with integrity
  checks, bounded cleanup retries, rollback retention, and credential
  revocation ordering.
- Added shared Provider artifact contract types, typed failure-code entries,
  and anchored launch seams for later resource compiler and runtime
  implementations.
- Added typed ResourceExport and ResourceImport contracts with signed
  projection-factory and same-Zone reference admission.
- Added Provider adapter traits and Core admission for Service-only exports,
  import-owned origin rejection, factory metadata matching, and closed
  backing and Binding target allowlists.
- Added a catalog-derived security-key semantic descriptor with the protocol
  version, base schema fingerprints, and signed projection factory metadata.
  The semantic backing allowlist is explicitly empty and denies every backing
  reference while physical lease and relay behavior remains unchanged.
- Added Core-owned ResourceImport projection lifecycle planning with
  deterministic creation, update, revocation, and binding-safe cleanup.
- Added bounded Zone bus metrics for routing latency, registrations, named
  streams, credits, backpressure, rejections, active sessions, and
  disconnects with closed low-cardinality labels.
- Added per-Zone configuration generation activation with integrity checks,
  count-based retention, non-blocking cleanup, redacted audit records, and
  restart-safe EphemeralProcess TTL cleanup.
- Adds strict Host, Guest, Process, EphemeralProcess, User, and Endpoint
  execution contracts with canonical Zone resource-bundle integrity checks.
- Adds bounded process-provider, guest attachment, Zone service, routing, and
  local Zone client seams with fail-closed identity and lifecycle handling.
- Added the v3 Zone resource compiler foundations, deterministic per-Zone
  bundles, private artifact catalog, and typed Process, Volume, and topology
  projections.
- Added typed Volume layout, view, attachment, store-view, TPM, and
  Zone-control Nix compiler projections with canonical generation coverage.
- Added generated v3 ResourceType schemas with drift-checked Nix validation and
  fail-closed semantic Service and Binding sharing projections.
- Added a typed asynchronous process attachment client for existing
  `EphemeralProcess` resources and configured launcher Processes, with
  ComponentSession named-stream lifecycle, route and peer pin checks,
  workload-user-preserving request shape, bounded retries, cancellation, and
  redacted failures.
- Added bounded TPM and GPU Device Provider lifecycle contracts with opaque
  Core effect boundaries, persistent TPM-state protection, render-node
  arbitration, and GPU-before-video sequencing.
- Added fail-closed Zone authority admission for Provider cardinality, quotas,
  emergency controls, and Host-global hardware backings.
- Added a fail-closed Provider crate layout policy that uses Cargo metadata,
  checks the normative source, test, integration, and README contract, and
  rejects Provider crates omitted from the workspace.
- Added exhaustive bounded ingress policy coverage for every observability
  transport, including structural failures, capacity ordering, and quarantine
  thresholds.
- Added the security-key Provider policy README contract and a container
  integration scenario covering opaque authority, lease, CID, session, and
  Guest frontend boundaries.
- Added hermetic configuration-owned cleanup coverage, explicit pending/stall
  conditions, schema-rejection audit recovery, and foreign-userland device
  cleanup smoke runners.
- Add staged, crash-recoverable redb logical restore and registered physical
  schema migration with identity checks, durable publication, rollback, and
  corruption quarantine.
- Add the crash-safe redb Zone resource-store backend with owned-descriptor open, bounded fair writes and MVCC reads, atomic resource indexes and revision logs, range-seek replay, and shared immutable watch fan-out.
- Connected production resource watches to bounded named-stream delivery with
  cursor-aware replay, owner notifications, backpressure, eviction recovery,
  and controller queue consumption.
- Added bounded Zone telemetry emission, closed metric-label validation, and
  redacted resource attributes for observability integrations.
- Added durable, hash-chained Zone audit records with rate-limited
  best-effort writes and administrator-only NDJSON export.
- Added read-only Zone health reports and bounded support bundles that omit
  resource specifications, names, paths, and process identifiers.
- Added authenticated Zone audit export, doctor, and support-bundle commands
  with bounded, redacted diagnostic output.
- Join broker and resource audit records with one Zone-scoped opaque operation
  key, fail closed on durability disagreement, and require synchronized audit
  segment data and directory metadata before reporting privileged success.
- Defer broker evidence for successful Zone activation outboxes until the
  broker records it, then drain the matching outboxes through the serialized
  store writer before publishing a generation.
- Make resource outboxes replayable with deterministic mutation identities,
  ordinals, timestamps, migration of older rows, terminal deny/error records,
  retention checkpoints, and bounded exports.
- Bound telemetry frame count, bytes, age, and retries while redacting
  identity-bearing values before export; enforce the same typed frame policy
  at retained emitter and ingress boundaries.
- Retain only observability Provider foundation contracts and bounded ingress;
  production OTLP/vsock/ComponentSession adapters, collector/forwarder/exporter
  loops, journald integration, projection/share, and resource ownership remain
  a separate completion unit.
- Added declared Bazel carriers and explicit local integration successors for
  the remaining Layer-1 Nix, policy, fixture, metadata, and workflow checks.
- Added eligibility and local-only reason parity checks, including the
  hermetic pure-evaluation proof and non-baseline Nix realization worker-image
  experiment.
- Publish stable checkout-bound repository, commit, and Git-valid branch
  metadata for developer BuildBuddy invocations while preserving unstamped
  action keys and cache reuse.
- Added a bounded Cloud Hypervisor Guest status projection with generation-fenced readiness, degradation, draining, and finalization eligibility.
- `make clean` removes this worktree's cargo target directories and its
  scratch tree, then collects unreferenced Nix store paths. The shared
  sccache directory is kept deliberately, so the next build re-links instead
  of recompiling from scratch. A directory is only removed when it lies
  inside the worktree and holds no git-tracked file, so an unexpected match
  fails closed rather than deleting committed content. `D2B_CLEAN_DRY_RUN=1`
  reports what would go without removing it; `D2B_CLEAN_SKIP_GC=1` and
  `D2B_CLEAN_KEEP_SCRATCH=1` narrow the sweep. Measured on a working
  worktree: 68 GB reclaimed.
- Added typed Zone resource commands for guest, host, execution, shell,
  endpoint, provider, activation, export, and import operations, with bounded
  deadlines and stable JSON envelopes.
- Added safe bash, zsh, and fish completion generation for the built-in CLI
  command registry.
- Added provider-neutral owned-child planning, atomic response recovery,
  dependency ordering, and finalizer-safe teardown for Process, Endpoint, and
  Volume resources.
- Added Device resource contracts and provider implementations for TPM,
  USBIP, security-key, and GPU/video hardware, including bounded lifecycle
  controllers, opaque host effects, and hermetic validation.
- Added Zone Device authoring validation and provider layout checks.
- Added a Cargo-authoritative Bazel 9.2.0 check graph with fixed Rust, Nix,
  policy, fixture, proof, and companion suites.
- Added credential-helper-only BuildBuddy execution with trust partitioning,
  redacted evidence, and one typed pre-dispatch local fallback.
- Add the durable SQLite spec store to `d2b-resource-runtime`: single-writer connection owned by a dedicated store task, WAL + `busy_timeout`, IMMEDIATE transactions with commit-before-return Ensure semantics, envelope-minus-status rows with provenance and deleting marks, a same-transaction durable-mutation audit log, and embedded `rusqlite_migration` schema migrations.
- Added the `d2b-resource-runtime` crate as the scaffold for the v3 resource
  runtime rewrite: a module skeleton (manager, resource, driver, context,
  provider, target, guest_target, watch, spec_store, identity, error,
  revision) plus a compile-and-run smoke test, registered in the Bazel graph
  (`all-tests` suite and `rust-main-packages` entry).
- Added the workspace dependencies `ractor` 0.16 (default features, no
  cluster) and `rusqlite` 0.40 with `bundled` SQLite, with cargo-deny
  admission updated in the same commit that first consumes them.
- Run the Azure Container Apps provider's in-source Rust tests through the
  Layer-1 Bazel graph.
- Driver-visible live resource read on the v3 resource-runtime contract
  (KTD3): `ManagerEndpoint::view` / `ResourceContext::get_view` return the
  manager's in-memory runtime view for one key - the committed row plus the
  status its actor last published (R11: memory only, zero store reads) and
  the row generation that status was published for.
- `ResourceView::observed_status`: the observed status of the exact row
  generation, so a `Ready` carried over from an older generation cannot
  pass as current readiness.
- Established the closed 27-Provider artifact matrix, deterministic catalog
  shape and output-layout contract, Provider matrix flake output, and
  fail-closed trust and activation catalog-digest checks.
- Added the ZoneLink route-admission contract for immutable operation intent,
  authenticated session profile binding, exact Zone identities, generations,
  capability, policy, and bounded runtime expiry.
- Added durable ZoneLink route identity and post-commit admission handoff.
- Azure Relay transport sessions now use concrete Guest-local WebSocket
  connectors and credential leases bound to the exact ZoneLink, session, and
  reconnect generation.
- Added ZoneLink Nix coverage for local and gateway-backed transport
  projections, same-Zone reference validation, and Guest execution placement.
- Added an immutable Zone/resource identity contract that binds Zone UID,
  resource UID, desired-state generation, and committed revision.
- Added shared provider-neutral display and clipboard bridge attribution and
  transfer-frame contracts.
- Added authenticated Guest-control session binding and revision-resumable,
  idempotent target-local Resource seeding for Cloud Hypervisor Guests.
- Added exact peer-bound controller ResourceV3 sessions with bounded bootstrap descriptor handoff and scoped Cloud Hypervisor assignments.
- Added a strict private Cloud Hypervisor Guest setup descriptor, deterministic child ResourceRefs, UID-free create batches, and private runtime identity fencing.
- Host integration coverage for authenticated resource activation, restart
  adoption, and Provider catalog wiring.
- Cloud Hypervisor Guest coverage now reports missing nested TAP/cgroup
  posture as an explicit preflight block instead of claiming adoption.
- Add bounded logical backups and staged, descriptor-relative restore
  primitives for Zone resource stores.
- Add global watch-delivery admission accounting with typed backpressure and
  deterministic slow-watcher eviction.
- `tests/tools/repro-rust-gate-env.sh` reconstructs the Rust gate's toolchain
  environment and runs a single command inside it, for diagnosing failures that
  only appear there without running the whole gate.
- Added bounded telemetry and authoritative audit callsite adapters for Zone
  resource storage, API, controller, provider, session, and bus lifecycle
  operations.
- Add a bounded GNU Make Rust test DAG with grouped keep-going output,
  dependency-ordered leaves, serial broker feature passes, and explicit
  companion coverage for doctests and harness-free binaries while retaining
  the default fixture-dependent contract and CLI surfaces when Nix is
  available.
- Add opt-in version 1 execution manifests through
  `D2B_EXECUTION_MANIFEST`, including deterministic sub-surface fragments and
  atomic partial evidence for failed and handled-interruption runs.
- Guest-side target-control service (U13 guest half): the
  `d2b.target-control.v1.TargetControl` ttrpc method is registered on the same
  authenticated ComponentSession that carries the Guest-local Resource API, so
  a Host-zone resource can realize target-locally inside a Guest through the
  published `GuestTargetRuntime`. Frames are decoded with the U13 codec; a
  frame that is unreadable or carries a foreign protocol token is refused
  before any dispatch.
- The live session generation is bound from every accepted session's
  authenticated route (`GuestTargetService::bind_session`), so the fence is
  the daemon's actual live generation: a request naming a non-live generation
  answers `SessionUnavailable` and performs no effect.
- The consumption path for converted types: each admitted realize is applied
  through the target-local effect code registered for its resource type, with
  `specDigest` recomputed from the exact carried bytes before the effect runs.
  A realization is reported `ready` only after its effect is serving; a failed
  effect leaves it `realizing` for the owning Host driver to retry. Adoption
  after a reconnect re-discovers the effect and reports `missing` when it is
  gone.
- Guest target layer (U13): `TargetDirectory` resolves a resource's declared
  `Host/<name>`/`Guest/<name>` execution reference into a realization handle,
  records exactly one target assignment per Host-zone resource, and binds a
  guest assignment to the authenticated ComponentSession generation that
  minted it.
- `GuestTargetRuntime` and the `GuestTargetControl` port: target-local
  realizations keyed by the owning Host-zone resource identity. A realization
  is an implementation detail of the target runtime - no desired spec of its
  own, never a second API-visible resource in the guest namespace.
- `TargetBinding`: the directory-backed target handle a resource actor holds.
  Every operation re-validates against the live session, so a handle retained
  across a reconnect can never realize, observe, or delete for its successor.
- An opt-in host trace for per-plane blit resolution in the lab's renderer fork,
  reporting a separate count for each condition that can reject a plane lookup
  and the plane texture's actual internal format and size. A lookup governed by
  several conditions returns a single negative result that names none of them,
  which had previously been read as the resolution failing when it was
  succeeding.
- Added `labs/venus-vulkan-video`, an isolated prototype that lets stock,
  unmodified upstream Firefox in a guest VM decode H.264 on the host GPU through
  Venus and virtio-gpu, replacing a forked Firefox and a separate virtio-media
  V4L2 decode path. The lab carries its own isolation contract, pin and evidence
  manifest, a reversible `/dev/kvm` grant helper, a scoped teardown that cannot
  reach unrelated VMs on the same host, and capability probes that run inside the
  GPU sidecar's bubblewrap namespace. It is self-contained, requires no host
  configuration change, and is deliberately outside the framework's option
  schema and gates.
- The lab reaches its goal: the browser is unpatched, decode runs on the host
  video engine, and the picture is correct. Getting there needed four
  interlocking fixes in the guest Mesa virgl driver and the host virglrenderer,
  all concerning the import of a decoded frame whose planes share one buffer.
  The plane index has to survive an import that hits the buffer-object cache,
  that import has to be describable to the host at all, a description covering a
  newly seen plane has to leave the guest rather than be discarded as a retype,
  and the host has to build an image for that plane in a pixel format the driver
  accepts. Both forks also gained opt-in import and blit tracing, because the
  existing debug machinery compiles out in release builds and so reported
  nothing at all.
- Added `labs/venus-vulkan-video/SOLUTION.md`, recording the full account: why a
  decoded frame is one allocation with its planes at offsets, how the browser
  consumes it, each defect and its fix, the changes still required for unrelated
  reasons, the measurements, and the several plausible fixes that proved inert.
- Added typed Volume storage providers for anchored layouts, bounded quotas,
  snapshots, TPM-safe state, and private virtiofs exports.
- Added authenticated ZoneLink and Guest ComponentSession driver admission
  that consumes the sealed route profile and its owning authenticated session,
  retaining the single-owner session authority in a non-cloneable driver lane.

### Changed

- Added compiler-checked negative trait bounds for the authority-bearing
  ComponentSession admission, verified Unix peer, session acceptor, and
  authenticated session types. The defining crates now reject `Clone`, `Copy`,
  `Default`, the named `From` mint paths, and zero-input `From<()>` construction
  for the admission and verified-peer evidence types through the Rust trait
  solver, regardless of aliases, selected module paths, or macro expansion in
  each configuration that is actually compiled. Generic checks cover
  unconditional blanket implementations, while separate checks cover the
  concrete unit and ComponentSession admission parameterizations currently
  used by the workspace. Future concrete parameterizations must add their own
  assertion. The ambiguity diagnostic now names the forbidden trait set and
  the compile-fail tests pin that wording.

- Kept the source trait-implementation inventory as a best-effort breadth
  check for explicit workspace source forms outside the compiler-enforced set.
  Focused regressions cover direct and inline `#[path]`, ordinary children of
  path-loaded modules, raw identifiers, lexical symlink paths, direct and
  `self as` aliases plus plain named and `::{self}` imports of local modules
  containing a discovered capability binding, alias-before-target ordering,
  chained aliases, chained re-exports, harmless aliases in every covered
  spelling, a harmless two-hop re-export that requires fixed-point convergence,
  direct and nested-group glob imports, nested re-exports reached through a
  glob, direct and grouped globs whose target is a renamed module alias,
  unresolved and two-hop glob propagation, terminating glob cycles with
  explicit-shadow precedence, direct and grouped block-local globs, and
  non-capability acceptance cases for block-local and renamed-target globs.
  Module aliases and module-level globs resolve monotonically over the finite
  set of parsed bindings and declared module targets; capability propagation
  resolves every glob target through that completed alias fixed point.
  Explicit bindings shadow glob imports, conflicting glob bindings fail closed,
  and hard iteration budgets independently guard target and taint convergence.
  Capability relevance includes descendants of a resolved module alias.
  Unknown glob destinations taint their importing module, and that taint
  propagates through later glob re-exports. A glob rooted at a Cargo-declared
  dependency name is classified as external and imports no local capability
  binding, preserving ordinary workspace dependency globs.
  Block-local globs carry lexical scope identities: proven same-scope
  non-capability module aliases remain accepted, while capability-relevant,
  ambiguous, or unresolved aliases fail closed. Other unmodelled glob shapes
  fail closed when they can classify an impl self type; this syntax scanner
  does not claim complete Rust glob or name resolution. Generic or cfg-gated
  declared type aliases, cfg-gated renamed imports, unsupported aliases, and
  lexically scoped capability aliases also fail closed during classification.
  Every parsed module item, including one declared inside a function or block,
  reaches the same attribute validation and external-source resolution.
  Unresolvable
  external modules, including missing `#[path]` targets, fail closed. Direct
  `path` and recursive `cfg_attr` receive dedicated handling; the source-inert
  allowlist accepts `cfg`, `doc`, `allow`, `warn`, `deny`, `forbid`, `expect`,
  `deprecated`, and the exact `rustfmt::skip` tool path in their approved
  shapes. Procedural, unknown tool, malformed, and every other unrecognised
  direct or conditional attribute fail closed with remediation. Approved
  snapshots retain rendered signatures for exact comparison, while scanner,
  Cargo, and rustdoc failures emit fixed operation labels, package or crate
  identities, exit status, and crate-relative locations without raw tool
  stderr, signature token streams, absolute scratch paths, or attacker-authored path literals.
  The source leg does not claim general Rust name or module resolution, macro
  expansion, `include!` expansion, or coverage of downstream implementations.

- Pinned the actual downstream `From<X>` boundary. A dependent crate that owns
  `X` can implement `From<X>` for a capability and compile when it only returns
  authority it already holds. A paired compile-fail fixture proves it cannot
  construct the capability directly because private construction state remains
  inaccessible. Private fields, instance identity, sealed traits, and consumed
  capabilities remain the primary anti-fabrication boundary.

- Strengthened cancellation publication race coverage to prove both activity
  and response state remain locked until their correlated entries are visible.

- Made the canonical spike-measurement policy guard cover all seven result
  rows, registered qualitative evidence summaries, and a global inventory of
  measurement-shaped fragments under `docs/**` and `CHANGELOG.md`. Per-class
  mutations now plant differently phrased or partial copies in an unregistered
  document and prove that the inventory rejects them; paraphrases that omit
  every number-and-unit, denominator, or canonical class phrase remain outside
  this mechanical check.

- Deferred the production redb backend, watch dispatcher, and real-backend
  reaction benchmark to the storage-integration wave after the feasibility
  spike missed its RSS gate. Backend acceptance now owns only independently
  satisfiable backend signals; watch-budget saturation evidence belongs solely
  to the watch item.

- Bound ComponentSession minting and bus registration to an instance-specific,
  registrar-issued single-use capability, with a compile-fail seal covering
  foreign session authorities.

- Preserved stable ComponentSession error codes and prescribed remediation
  through bus endpoint failures while retaining class-only observer labels.

- Added closed-label observations for failed correlation cleanup and queued
  stream shedding, classified expected handshake failures without internal-bug
  labels, and redacted channel identifiers from debug output.

- Propagated inbound cancellation into generated service handlers, suppressed
  cancelled replies, dispatched pinned endpoint cancellation during revocation
  and reconnect, and made ttrpc response correlation independent of
  caller-selected stream identifiers.

- Split established transport read and write ownership behind a bounded
  record-budgeted writer queue so blocked writes no longer stall inbound
  records, cancellation, or control work. Cancellation now removes queued
  replies before transport delivery or fails the session closed if protection
  has already committed their record sequence.



- Reclaimed cancelled receive waiters before applying the per-session waiter
  bound, preventing normal repeated cancellation from exhausting and
  disconnecting a component session.

- Released bus correlation and operation slots on every post-start response
  path, including malformed responses and terminal receive failures, while
  observing cleanup errors and propagating those not caused by driver teardown.


- Unified Core and toolkit controller identity, selector, trigger, registration,
  retry, and resync contracts. Core changes now drive the executor-native
  reconciliation runner through a bounded, coalescing registered-resource
  adapter with explicit admission and backpressure counters. Until a durable
  store backend is registered, expedited commit authorization fails closed
  rather than manufacturing evidence. Terminal runner failures retain the
  complete accumulated report alongside typed failure details.

- `tests/test-proofs.sh` now discovers proof crates by scanning
  `proofs/*/Cargo.toml` instead of iterating a hardcoded list. The previous
  shape paired that list with a silent skip when a directory was absent, so a
  renamed or never-created proof crate reported success while executing
  nothing. An empty `proofs/` tree now fails closed. Every discovered crate
  must have a sibling lockfile and runs clippy and tests with `--locked`. The
  redb proof also executes its four ignored full-scale correctness fixtures in
  release mode; this adds about five minutes but ensures the proof gate runs
  its principal oracle, watch, conflict, and owner-fan-in experiments.

- Added ADR 0046 and its complete, documentation-only normative
  specification set for the d2b 3.0 Provider control plane. The set includes
  foundation, resource, cross-cutting, and Provider dossier specifications
  indexed by `docs/specs/README.md` and `docs/specs/providers/README.md`.
  It specifies Zone-local resources over the 19 standard ResourceTypes
  (including `Endpoint`), independently packaged multi-process Providers
  selected by `{ artifactId, config }`, an asynchronous embedded redb resource
  plane with owner-driven reconciliation and commit-gated expedited reconcile,
  status-first component state with optional Volumes, layered base-plus-provider
  ResourceType specs with three-layer status, resource currency/upgrade/recycle
  with CLI projections, Guest-resident Entrablau identity custody, fast hermetic
  tests with integration-only slow coverage, and Noise-protected
  ComponentSession/d2b-bus channels on the `d2bus.org` public namespace,
  together with the security/threat-model and feasibility contracts. No crates, services,
  controllers, or Providers are created.


- Accepted ADR 0046 and its specification set, flipping each metadata
  `Status` from `Proposed` to `Accepted`. No crates, services, controllers,
  or Providers are created.
- Retargeted every ADR 0046 slice branch at the protected `v3` integration
  branch instead of `main`, which the v3 line never merges into. References to
  main-branch ADR 0045 provenance are unchanged.
- Corrected the disk-hygiene contract in `AGENTS.md`: Rust worktrees each keep
  their own `packages/target/` and deduplicate compiled output through
  `sccache`, rather than sharing a cargo target directory. A shared target dir
  is deliberately avoided because cargo's target-dir lock is workspace-wide.
  The worktree-removal guidance, the disk-space preflight remediation text,
  and the ADR 0046 cleanup guidance that assumed a shared-cache symlink are
  corrected with it.
- Enabled the required Layer-1, eval-shell, and Entra example PR gates for
  changes targeting the `v3` branch as well as `main`.
- Derive daemon admission from accepted Unix peer credentials and the configured lifecycle group.
- Reopen Zone stores with their persisted revision metadata while keeping the
  Resource API fail-closed until registrar-owned ComponentSession routing is
  registered.
- Add typed system-core Host and User handler contracts without publishing
  fabricated readiness.
- Bound Entra credential acquisition, refresh, authorization, redacted status,
  bounded degradation, and finalization cleanup to the owning Guest and Zone.
- Require exact authenticated Guest, Provider, and Zone bindings before any
  client effect, preserve committed refresh metadata after post-commit
  validation failures, and prevent finalized credentials from minting again.
- Reject GPU Device resources on unsupported host platforms during Nix
  evaluation.
- Make shared Host-global leases unique, validate GPU/video identities and
  closure proofs, recover partial restarts without duplicate workers, and
  reject malformed GPU runner shapes before device opens.
- Hardened the canonical ACA, Azure VM, Cloud Hypervisor, and Azure Relay
  Providers with bounded readiness, startup, credential, bootstrap, and
  reconnect behavior.
- Treat pause-at-boot as an initial QMP proof and bound greeting timeouts to
  fresh launches while adopted runners retry through health degradation.
- Keep a failed fresh-launch generation terminal until finalization so a
  stopping runner cannot be adopted or reserve device authority again.
- Bound managed-identity Credential leases to authenticated subject, Zone,
  workload, Provider session, and bounded expiry state; restart checkpoints
  remain secret-free and finalization revokes only the owning workload's
  handles. Reacquisition replaces terminal or stale-session records without
  inflating the accepted client rotation generation, and repeated checkpoint
  restore remains occupancy-safe and idempotent.
- Extract provider-neutral daemon runtime ownership into `d2bd-runtime` while preserving the `d2bd` process topology and in-process provider composition.
- Nix source builds can opt into a fixed multi-user `/var/cache/d2b-sccache`
  provisioned by the NixOS module as `root:nixbld` mode `2770`, with the Nix
  daemon supplying the global sandbox path.
- Move USBIP, security-key, TPM, and GPU/video state, argv, tests, and Nix
  ownership into their device providers while keeping daemon and broker effects
  unchanged.
- Fence Provider controller assignments to one resource target and route scoped
  Resource API watches and mutations through the existing Zone bus.
- Use typed Resource API services for Guest activation, configuration access,
  and readiness instead of legacy Guest control callers.
- Remove the standalone no-bash AST walker test and its executable Bazel and
  Make wiring from the source-hygiene graph.
- Restore remote BuildBuddy compilation with local Bazel test execution until
  the remote worker image supplies the upstream test wrapper runtime.
- Remove the heavy-gate semaphore, self-reexec guards, and host provisioning;
  public integration, performance, smoke, and flake lanes now run their
  existing work directly.
- Let BuildBuddy compile Rust tests while the remote profile runs their test
  actions through the pinned local project shell.
- Remove obsolete per-target remote-execution exclusions from local-runtime
  Rust suites.
- Remove the redundant local Rust suite; package suites now compile every Rust
  test through BuildBuddy.
- Limit stable-principal collision checks to hash-derived UIDs so declared
  Guest-root activation profiles can share UID 0.
- Capture all Bazel test output in the facade warning gate instead of relying
  on non-local BuildBuddy test-log URIs.
- Compile Guest and Provider-owned Process resources from canonical Zone-scoped
  Nix inputs and retain Guest system evaluations outside daemon authority.
- Bind Zone resource bundles and stores to immutable identity tuples and
  publish complete local generations only after every Zone is prepared.
- Route Guest start, stop, and restart through typed Provider lifecycle
  dispatch with immutable Zone, Guest, Provider assignment, policy, and
  operation identity.
- Quarantine UID-less or stale lifecycle and runner records without replay,
  adoption, cleanup, or deletion.
- Make committed Network resources and Guest attachments the sole network authority. Network host effects now use one root-owned, UID- and generation-bound admission index with collision preflight, immutable derived names, broker scope validation, and foreign-state preservation; generic Network Guest hardening retains the DHCP neutralizer and IPv4-only posture.
- The broker no longer loads retired realm controller or identity metadata;
  the remaining argument placeholders are limited to the clean-break deletion
  boundary.
- Migrate the Azure Container Apps Provider's Zone-native controller and
  effect consumers to the v3 Provider and Resource contract crates. The
  legacy gateway compatibility module remains until its Gateway DAG consumer
  is migrated.
- Reconciled the root and copied Guest Rust lockfiles with the current
  Zone-native provider and neutral contract manifests.
- Harden Cloud Hypervisor Guest restart adoption, VMM health degradation,
  disruptive recycle, and finalizer-safe deletion.
- Convert the Credential resource family to the v3 `ResourceDriver` contract
  (U12): `packages/d2bd/src/credential_driver.rs` carries the secret-service,
  entra, and managed-identity controller behavior as one type-level driver -
  validate folds `validate_spec` plus the per-kind scope checks, recover
  adopts a serving managed-identity agent Process, reconcile mints and
  ensures `Process/mi-agent-<credential>` through the manager (the agent is a
  Process resource; the driver never spawns it, KTD13), and delete preserves
  the revocation-first ordering: the provider RevokeToken call must confirm
  (or already have confirmed) before any owned Process child is marked
  deleting, and a missing or no-longer-current session generation fails the
  pass closed (R28).
- Delete the runner-backed `CredentialResourceReconciler`,
  `credential_controller_descriptor`, its finalizer/mutation helpers, and the
  old reconciler test set from `packages/d2bd/src/credential_resource_runtime.rs`.
  What remains is the provider-side session surface the conversion still
  needs: the typed `CredentialSession` registry, the exact non-secret
  revocation request that binds one session generation, and the same-Zone
  scoped credential client. Status is now the driver's in-memory typed
  projection (R11), carrying the old phase/outcome classification
  (`credential-provider-unavailable`, `credential-agent-pending`,
  `credential-agent-unavailable`, `credential-agent-draining`,
  `credential-lease-revoked`, `credential-revocation-uncertain`).


- The old revocation gate read the lease facts from the Credential's durable
  status (`/status/resource/credential/...`), which the rewrite deletes.
  `CredentialDriverEffects::lease_facts` supplies them instead; `None`
  reproduces the old "no lease state" case (skip revocation) exactly. Until
  a production lease-fact source is wired, that branch stays dormant, as it
  is today in-tree (nothing writes `leaseState`).
- `test-host-integration` passes the host's `/dev/vhost-vsock` into the build
  sandbox, so a vmCheck may give its node the same virtio-vsock device an
  enrolled Guest receives. The QEMU the test driver uses already ships
  `vhost-vsock-pci`; only the sandbox's device namespace excluded it.
- Move the per-Zone SQLite spec store under the daemon's own state root
  (`<daemon-state>/zones/<zone>/spec-store.sqlite3`). The broker-provisioned
  `<state-root>/zones/<zone>` directory is owned by the zone-store principal,
  so the daemon could not create a file in it (`SQLITE_CANTOPEN`).
- `test-host-integration` builds vmChecks with the invoking user's identity
  (`--option build-users-group ""`) so the VM test driver keeps the
  privileges QEMU and its tap wiring need, without changing the host's
  global Nix configuration.
- The resource plane partition is now enforced by one authority in
  `d2b-contracts` (`ResourcePlane`, `resource_plane`, the converted-type
  registry) instead of per call site, and the API store facades and the
  daemon's child bridges consult it. A converted type can no longer be read
  or written through the legacy store: the access fails with a named,
  non-retryable `WrongPlane` error that carries the resource type and the
  caller instead of being reported as absence or as a retryable read failure.
- The daemon's Cloud Hypervisor session lists converted types through the
  manager plane, so a controller that relists its owned children no longer
  races the legacy mirror.
- The fence arms when the zone's manager plane is published; before that the
  durable path keeps serving, so first boot and unit fixtures are unaffected.
- Driver failures now carry a structured detail - the operation, the stage,
  the failure kind, the verdict, and the compared values (redacted where the
  value is secret) - and the operator log line and the wire failure layer are
  projections of that one value, so what an operator reads is what a test can
  assert.
- The verdict distinguishes structural outcomes: `NotYet` defers and requeues,
  `Refused` is a decision against the row, and `Error` is an operational
  failure; only `Refused` and terminal `Error` end a row's reconcile.
- A registry of failure kinds records what each kind means and its likely
  cause, and the reference document is generated from it rather than
  hand-maintained.
- The broker resolves a launch's posture - standard, controller with
  escrow, or serving worker - once from the trusted intent and carries it as
  a value. The file-descriptor contract, bootstrap and console response
  indices, escrow custody, runner credentials and the user-namespace
  mapping all read that value instead of re-evaluating the same condition,
  so a decision cannot be omitted at one site while the others agree. The
  intent-derived wire role alias is likewise evaluated once, for both spawn
  validation and observation.
- A resource read now returns one classified result (`Present`, `Absent`,
  `Unavailable`, `Error`) that also records the plane it was answered from,
  instead of each call site collapsing everything that is not a row into
  "not found". The classification carries its own default: absence and
  unavailability defer and requeue, and only a site that can name its
  terminal evidence reports a terminal failure. A provider or manager that
  cannot answer is therefore no longer indistinguishable from a resource
  that is genuinely gone.
- The guest, volume, binding and endpoint controllers read through the
  classified surface, and the guest effect surface folds an unreadable row
  into one named terminal error that records the plane and the cause.
- One producer now renders a resource row for the wire on the API side, and
  one produces the live status projection in the resource runtime. Readers
  of a converted type receive the canonical envelope, metadata, spec and
  observed status, so the shape can no longer differ between a `get`, a
  `list`, a mutation confirmation and a status read.
- The daemon's Core, guest, interaction and shared-provider views take their
  phase string from the runtime's wire-status projection instead of each
  mapping the status vocabulary locally.
- Added a `finalize` step to the `ResourceDriver` contract and made the
  delete pass run it before the driver's delete: the erased actor boundary
  calls `ResourceContext::finalize_owned_resources()`, which returns
  `ChildrenDraining` while any owned child row is still live. All converted
  drivers (`activation`, `binding`, `credential`, `core`, `endpoint`,
  `guest`, `interaction`, `process`, `shared-provider`, `system-core`, plus
  the telemetry driver) now finalize their owned children before their own
  provider teardown stage, so a parent's teardown can never run ahead of a
  live child.
- The manager holds a cleanup-completed parent row (including its durable
  deleting mark) until its owned children retire, and the delete pass gates
  each level on its children draining.
- One canonical `LaunchIdentity` (`d2b-process-conformance`) replaces the
  partial, per-layer Process launch identity. The value carries the semantic
  owner ref and UID, the exact execution target, the cross-target Guest
  selector, the broker VM scope, and the legacy runner role, and it is
  validated at construction: an incomplete row fails once, naming the missing
  input (`launch-identity-missing-target-ref`,
  `launch-identity-invalid-execution-ref`, ...).
- One resolver (`resolve_launch_identity`, `d2bd::process_resource_runtime`)
  produces that value from the durable row (`owner ref -> owner uid ->
  execution target -> target ref -> vm/role`). The Guest-owned guest-runtime
  target derivation (`guest_runtime_process_matches`) is now a rule inside
  the resolver instead of a call-site special case in the Process driver.
- The launch ticket carries the resolved identity and consumes it (owner,
  target, VM scope, binding-worker split) instead of re-deriving
  `target_vm_name`; the bundle process-DAG ticket path names its own VM
  through the same value. The broker's identity fence
  (`BundleBackedLaunchResolver::resolve_intent`) reads
  `ticket.launch_identity()` for the launch VM, the legacy role, and the
  serving-worker split instead of matching `(execution_ref, target_ref)`.
- The Process driver's adopt/probe identity (`ProcessResourceIdentity`) now
  embeds the resolved `LaunchIdentity`; the legacy `ProcessResourceRuntime`
  context resolves the same value for unconverted rows, and the guest-side
  Cloud Hypervisor probes resolve through the same resolver at ticket time.
- An incomplete identity is refused as `process-identity-incomplete` at the
  driver boundary (a terminal, named failure) instead of surfacing as a
  fence rejection one field at a time.
- A Volume layout report that is not `Ready` (Degraded/Pending) completes the
  driver's long effect as a *retryable failure* instead of a success. `Completed`
  re-enters the pass immediately, which respawned the layout effect - and, for a
  Nix closure source, the broker `StoreSync` its source resolution performs - at
  completion rate with no bound (measured ~10 StoreSync attempts/s while the
  store-view lock stayed quarantined). The retryable class is the resource
  actor's documented ownership (R13): it schedules exactly one requeue after the
  manager's backoff, so a degraded layout runs its effect, and therefore one
  source resolution, at a fixed interval instead of spinning.
- Converted the nine fixed Core controller-family resource types (`Zone`,
  `ZoneLink`, `Provider`, `Role`, `RoleBinding`, `Quota`,
  `EmergencyPolicy`, `ResourceExport`, `ResourceImport`) to the v3 plane:
  a new `CoreDriver`/`CoreResourceDriverFactory`
  (`packages/d2bd/src/core_driver.rs`) serves them through
  `CoreDriverEffects`. The eight metadata-only types converge without
  effects; `Provider` observes its manager-served owned `Process`/`Volume`
  rows plus the fixed Host/Zone rows and runs the preserved
  `provider_observation` / `ProviderHandler::plan_observed` logic, publishes
  its typed status in memory (R11), and finalizes children-first with the
  controller-drain gate before converging. The nine types join
  `V3_CONVERTED_RESOURCE_TYPES` and are served only by the manager plane.
- Converted the `Guest` resource family to the v3 plane. A new
  `GuestDriver`/`GuestDriverFactory` (`packages/d2bd/src/guest_driver.rs`)
  serves the four runtime Providers - cloud-hypervisor, qemu-media, azure
  container apps, azure virtual machine - as one `Guest` driver over the
  dyn-erased `GuestDriverEffects` port
  (`packages/d2bd/src/guest_effects.rs`: the CH real path plus the preserved
  framework QEMU/ACA/AzureVM controllers). Validate checks the spec against
  the owning Provider row; reconcile ensures the kind's desired children
  through the manager child API (QEMU: Volume + Process; ACA: Endpoint; CH
  children stay committed by the CH controller session), retires owned
  children the derived set no longer names endpoint-first/process-last, runs
  the provider effect, publishes in-memory status (R11) and requeues while
  not converged; delete runs the provider stage before child retirement.
  `Guest` joins `V3_CONVERTED_RESOURCE_TYPES` (31 -> 32) and is served only
  by the manager plane.
- The Cloud Hypervisor controller's layered Guest status now reaches the
  public view through the in-memory projection channel
  (`ResourceContext::set_status_projection` /
  `ResourceView.status_projection`): `UpdateStatus` captures the provider
  status into the sink instead of writing a durable row, and the
  finalizer-acknowledgement arms are no-ops because the manager's
  deleting-row hold replaces the provider finalizer (F3). The converted
  child arms of the CH session refuse closed.
- `resolve_committed_guest_session_target` falls back to the endpoint row's
  `metadata.generation` for the session target, because a manager-owned
  Endpoint row carries no durable status to stamp an endpoint generation.
- Converted the interaction/shell family to the v3 plane: new
  `InteractionDriver`/`InteractionDriverFactory`
  (`packages/d2bd/src/interaction_driver.rs`) serves the six
  display/audio/shell types (`display-wayland.d2bus.org.WaylandPolicy` /
  `WaylandSession`, `audio.d2bus.org.AudioService` / `AudioBinding`,
  `shell-terminal.d2bus.org.ShellPool` / `ShellSession`) over the
  `InteractionDriverEffects` port
  (`resource_runtime/interaction_effects.rs`). Reconcile registers the
  family's dependency/child watches once per target (R12/R17), ensures the
  desired child set through the manager child API (commit before spawn),
  retires obsolete children endpoint-first/process-last, runs the preserved
  provider effect, publishes in-memory status (R11) and requeues at the
  family's resync cadence; delete runs the provider stage before child
  retirement and stays retryable while a stage is pending. The six types
  join `V3_CONVERTED_RESOURCE_TYPES`.
- Converted the system-core Host/User family to the v3 plane: a new
  `SystemCoreDriver`/`SystemCoreDriverFactory`
  (`packages/d2bd/src/system_core_driver.rs`) serves `Host` and `User` over
  the bounded probe/NSS-discovery effects port. Validate checks the Host
  provider reference and decodes both specs; reconcile observes the host
  once per generation (with the degraded fallback) or discovers the local
  user, publishes the typed status in memory (R11) and short-circuits at the
  current generation; delete converges with no effects or children. `Host`
  and `User` join `V3_CONVERTED_RESOURCE_TYPES` and are served only by the
  manager plane. The family's durable status projection is consequently
  no longer store-visible; the read view serves the actor's in-memory
  phase.
- A declared device bind the host does not provide is now a named broker
  refusal: `prepare_device_binds` reports
  `device-bind-missing: <path>` (and `device-bind-unusable` for any other
  errno) instead of a bare `ENOENT`, so the broker journal and audit say which
  grant a launch needed. On a host without a render node the GPU/video worker
  launches now read
  `clone3/spawn failed: device-bind-missing: /dev/dri/renderD128`.
- `videoNvidiaDecode` selects the closed `video-worker-nvidia` worker
  template instead of being dropped or refused: the Device's video row
  declares that template (`packages/d2b-provider-device-gpu/nix/default.nix`)
  and its launch carries the reviewed bind set (`/dev/dri/renderD128`,
  `/dev/nvidiactl`, `/dev/nvidia-uvm`, `/dev/nvidia0`) from `d2b-core`'s
  posture table. A host that lacks one of those nodes refuses by name
  (`device-bind-missing: <path>`; `/dev/nvidia0` on the documented single-GPU
  default), and the ceiling below makes a persistent refusal terminal instead
  of an endless requeue.
- The Device GPU/video worker rows declared by
  `Provider/device-gpu` carry a restart ceiling
  (`restartPolicy.maxRestarts = 2`). A worker whose launch is persistently
  refused (a host device the broker cannot open, a site with no projected
  Wayland socket) previously retried forever. With the ceiling the refusal
  reaches the closed terminal classification `process-start-budget-exhausted`
  (refused, at `reconcile/launch`) instead of an endless requeue. The ceiling
  is a per-daemon-lifetime launch counter, not a window: the driver's
  in-memory `RestartBudget` counts up and never resets, so `resetAfter` has
  no effect (`packages/d2bd/src/process_driver.rs`).
- The zone schema enforces the endpoint-purpose token: the generated v3 Endpoint
  schema constrains `purpose` to the bounded token pattern
  (`^[a-z][a-z0-9-]{0,62}$`) instead of a free bounded string, so a dotted
  purpose fails schema validation where it is written rather than at a daemon
  decode.
- The store-view posture code no longer restates its per-level modes: it reads
  the `guest-store-view` rows from the declaration and fails closed on a row
  it cannot apply. The declaration also names the anchor-open rule an anchored
  walk must obey - anchor components open `O_PATH`, only an owned leaf opens
  `O_RDONLY` - so read-on-traversal-only cannot be reintroduced quietly.
- Zone authority operations - including the generation-publication barrier that
  proves one complete local generation set before the resource plane serves
  reads - are now owned by a process-local ledger
  (`d2bd_runtime::authority_persistence::ZoneAuthorityLedger`) instead of a
  durable store. This is a deliberate reduction of durability, not a
  re-homing: the admission barrier still serializes concurrent operations
  inside one daemon lifetime and still fences a conflicting generation, but a
  restart begins with an empty ledger. What re-establishes the state across a
  restart is re-derivation, not replay: the daemon's composition path recomputes
  the complete generation set from the Zone bundle and the authority identity
  and re-prepares/re-commits the publication marker on every boot, while the
  resources themselves are recovered by the drivers' probe/adopt path. The
  activation host-integration fixture proves exactly that for the controller
  half - its restart stages assert the external provider controller keeps its
  PID and its Process row keeps uuid and generation across
  `systemctl restart d2bd.service`, and that the row is still `Ready` with
  `observedGeneration == generation` after the resync window.
- Host-global claims (GPU and external NIC) recorded in
  `packages/d2bd-runtime/src/authority_persistence.rs` become process-local:
  in-flight claims are no longer crash-recovered from a durable row, so a daemon
  crash releases a claim that the durable adapter used to keep reserved until the
  owner released it. Owners are re-derived on the next boot by the same
  probe/adopt recovery path, and a claim that cannot be re-proven is refused
  rather than assumed. This is the deliberate controller-checkpoint reduction
  U14 was planned to make; it is called out here so a reviewer can weigh it
  instead of discovering it as a regression.
- The interaction runtime's audio admission no longer re-checks a dependency
  against a persisted status fence: `validate_audio_assignment` is gone with the
  persisted status it read, and `fresh_audio_dependency`
  (`packages/d2bd/src/resource_runtime/interaction_effects.rs`) now validates the
  authoritative row itself - identity, ownership and Zone - read from the
  manager. An assignment is admitted from the manager row and the committed
  provider configuration, the same authorities the rest of the interaction
  family reads; the consequence is that a stale persisted status can no longer
  veto an assignment the manager considers current - and equally, no persisted
  status can *authorize* one any more. The guest-side effect for these rows
  remains intentionally unimplemented (U13 note).
- Guest-local serving is memory-backed and target-local only. `GuestResourceStore`
  (`packages/d2bd-runtime/src/guest_resource_runtime.rs`) keeps
  controller-created Process-family rows in process for the lifetime of the
  admitted parent-Zone ComponentSession, with no database file: the guest runtime
  constructors no longer take a state directory, and there is no per-target
  database to reopen. Zone-authority types are refused at the boundary
  (`AuthorizationDenied` for their schema reads and watches) rather than
  pretended to be served, a watch answers with a receipt bound to the in-memory
  store, and a target-local restart rebuilds those rows from the host - the only
  binding domain of record. Guest-side effects for those rows remain
  intentionally unimplemented (U13 note), unchanged by this cut.
- The supervisor's launch resolver gained the Device branch: a ticket whose
  semantic owner is a `Device` resolves through
  `find_device_worker_intent`, so a Device-owned row never falls through to
  the generic template lookup (which two Devices in one Zone share).
- The broker's typed Process metadata fence admits a Device worker only with
  its `Device` owner (and requires one), and binds its template identity to
  the declared template (`intent.profile_id`) rather than the per-row role
  id.
- The TPM and GPU Device Providers now declare the sandbox posture of their
  worker rows in Nix (`namespaceClasses`, `userNamespace`, `umask` `0007`),
  which is the public half of the same posture the private binding pins and
  the broker fences: `w1-swtpm` with a `process-principal-root` user
  namespace for the long-lived swtpm worker, `w1-swtpm` without one for the
  one-shot flush, `w1-gpu` with `kvm`/`dri`/`udmabuf` binds, `w1-gpu-render-node`
  with the broker-pre-opened render node and no bind, and `w1-video` with a
  pid namespace and the DRI bind.
- Every controller launch in the daemon goes through the Process controller.
  The TPM swtpm/swtpm-flush effect and the GPU/video worker read, gate and
  retire the Device Provider's declared `Process`/`EphemeralProcess` rows
  instead of spawning through the broker surface, and the VM bring-up launcher
  has no raw-broker fallthrough left: a node that is neither provider-managed
  nor the controller-owned Cloud Hypervisor runner is refused with
  `vm-start-node-not-provider-managed:<node>`.
- The grep gate over `packages/d2bd/src` is clean: no production
  `BrokerRequest::SpawnRunner` caller remains, and the only hits are test-side
  (the composition tests' fake-broker envelope readers and the `#[cfg(test)]`
  Device-TPM reconcile simulation with its fail-closed `NoManagerChildSurface`).
  The sanctioned funnel stays `BrokerProcessBackend::request_with_fds` behind
  `ProcessLaunchEffectPort`.
- The VM bring-up launcher (`VmStartRunner`) never spawns through the raw
  broker surface. Its `SpawnRunner` fallthrough is removed and any node that
  is neither provider-managed nor the controller-owned Cloud Hypervisor
  runner is refused with `vm-start-node-not-provider-managed:<node>`. The
  fallthrough was unreachable by live nodes: both `NodeRunner` entry points
  already filter the guest-owned and durable-wayland node classes (a
  guest-owned process is launched by its Guest, a durable-wayland node stays
  readiness-only), so provider-managed sidecars keep launching through
  `ProductionProcessProviders::launch_node` and the daemon no longer holds a
  second, legacy launch path for them.
- The USBIP attach child port (`SharedRunnerUsbipChildren`) documents why it
  stays fail-closed: its attach seams belong to `BindingLifecycle`, which no
  production path constructs (it exists only in the USBIP provider's tests).
  The v3 Binding realizes its attach through the declared child resources -
  the Guest `Process/...guest-proxy` plus its Endpoint - committed as
  owner-scoped child rows of the Binding and launched by the Process
  controller, so no attach spawn is faked here.
- The TPM effect port (`tpm_effect_port.rs`) reads and retires the declared
  `Process/swtpm-<device>` / `EphemeralProcess/swtpm-flush-<device>` /
  `Endpoint/tpm-<device>` rows through the Device's manager child surface and
  ensures the controller-owned `Volume/device-<32hex>-tpm-state` child, so the
  Process controller owns the launch, restart, adoption, drain and teardown of
  swtpm and the pre-start flush. The two broker calls that remain are the
  one-time legacy state migration and the broker-owned state-directory
  preparation, neither of which launches a process. The retirement of the
  synthetic `device-<12hex>-*` refs replaces the durable-snapshot adoption
  gate: the row's owner (the requiring Device) and its published phase carry
  the classification now, which is what the port's new tests pin.
- The GPU lifecycle port (`shared_provider_effects.rs`) reads the declared
  `Process/gpu-<device>` / `Process/video-<device>` rows as the worker
  identity: the opaque process token is derived from the row's durable uid and
  generation, `observe_worker` recomputes it from the row (so a replaced row
  reads as `StaleIdentity`, not `Missing`), and `stop_worker` deletes the row
  and reports closure only once the row is gone. `open_authorized_devices` no
  longer opens device classes - the grants travel in the launch intent's
  declared posture (the closed `device_worker_posture` binds plus the
  broker's render-node pre-open).
- `SharedProviderChildSurface` gained `view` (the manager's live row view) so
  a Provider effect can gate on a child row's published phase without holding
  a broker handle; `view_phase` is the one phase projection both the driver
  and the effect ports read.
- The TPM provider's typed row builders now carry the declared template
  posture: `build_swtpm_process_spec` uses umask `0007` (the socket-ACL
  umask the broker profile selects) and `build_swtpm_flush_spec` declares no
  user namespace (the preserved user-NS-long-lived-only contract), matching
  the Nix-projected rows the compiler fences against.
- The GPU README and the driver/effect module docs describe the row-owned
  launch instead of the retired `OpenDevice`/`SpawnRunner` mapping.
- `d2b_core::site::SiteJson` (new DTO) validates the socket fail-closed at
  bundle load (exactly `/run/user/<uid>/<display>`, no parent components);
  `BundleResolver` loads it for zone-native bundles through the optional
  `sitePath` index field and, for legacy bundles, when `site.json` ships
  beside the other artifacts. Missing artifact keeps reading as "no site
  facts" - no bundle version bump and no reader change for existing bundles.
- `device_worker_wayland_sock` in the Process driver reads the projected
  value from the loaded bundle; the GPU argv composition renders it as
  `--wayland-sock`.
- Simplified the planned contributor quality gate to use one comprehensive
  discovery pass, a shared issue ledger, scoped verification, and ordinary
  repository controls instead of privileged review services.
- Added the fixture-dependent Rust contract lane to the manifest-driven local
  and pull-request test graph. It builds the fixture bundle and runs both the
  contract crate and the command-line output contract cases, which previously
  executed in no lane at all, and it acquires the shared heavy-validation
  semaphore before doing so.
- Changelog fragment parsers now require canonical ASCII dash bullets so
  malformed release-note entries fail consistently.
- Documented the guest workspace drift guard and its required fixture and lock
  updates for shared-crate dependency changes.
- Made the resource-store dependency gate explicit: redb adoption and
  performance-sensitive store work remain blocked on unexecuted feasibility
  evidence, while engine-neutral codecs, table contracts, errors, and
  transaction semantics may proceed with small-scale hermetic tests.
- Froze the ten-table on-disk schema and codec discriminants, the closed store
  error mapping, and the source-versus-integrator ownership of generated
  storage-contract artifacts.
- Aligned label and annotation keys with the canonical JSON 64-byte object-key
  ceiling.
- Kept only delivered engine-neutral contract work in the foundation delivery
  group and moved unrun feasibility, production backend, watch, storage-row,
  and migration work to their actual implementation groups without changing
  contract-consumer dependencies.
- Credential Provider runtimes preconnect their Guest-local Noise_KK backend
  before publishing readiness and execute typed acquire, inspect, refresh, and
  revoke requests through the live bounded session.
- The fd11 responder now binds the child peer credentials learned from the
  authenticated fd10 bootstrap rather than trusting the socket creator.
- Attached Secret Service, Entra, and managed-identity Credential rows to the
  shared Core source and Runner with ResourceService-backed status projection,
  bounded Provider-specific work, and exact finalizer cleanup ordering.
- Added same-Zone scoped ResourceService admission for Azure Relay credential
  reads while keeping typed ComponentSession delivery and credential custody
  inside the selected Guest.
- Finalization now accepts one durable typed revocation effect, records bounded
  revocation evidence, survives reconnect deduplication, and retains the
  Credential finalizer until managed-identity Process children are gone.
- Guest credential backend composition now selects separate Secret Service,
  Entra identity Endpoint, and managed-identity IMDS adapter boundaries by
  authenticated Provider and scope metadata.
- Production Guest mode composes the adapter-backed source; the explicit
  fail-closed supervisor remains available only for degraded and negative
  paths.
- Credential Provider Processes now receive a live Guest-local fd11 backend
  responder from the Guest supervisor, bound to the exact ComponentSession
  route and cancellable with Process/session teardown.
- Delivery keys are generated as one-use random key material by the Guest
  supervisor and transferred only through the authenticated Provider session;
  public Zone and generation identifiers no longer derive Noise_KK keys.
- Provider session bootstrap now carries the authenticated User placement
  claim separately from the Provider controller subject.
- Secret Service runtime construction requires that User claim and binds it
  into the exact Zone user-agent placement; Entra and managed-identity
  runtimes reject unexpected user-domain claims.
- Rename the privileged broker package and executable to `d2b-broker` and add
  fixed Host and Guest profiles with separate socket, state, audit, caller,
  and authority bindings.
- Keep Unix, vsock, and Azure Relay Providers transport-only with typed opaque
  carriage handles, Core-owned reconnect generations, bounded relay retries,
  and scoped Gateway Guest Credential reads.
- ADR 0046 spec set: aligned the illustrative examples across every resource,
  topology, and Provider-dossier spec with the frozen datetime, universal-status,
  outcome, and ResourceType-name decisions so every persisted-datetime literal
  uses millisecond precision (`YYYY-MM-DDTHH:MM:SS.sssZ`), every universal
  envelope carries `status.resource` and `status.update`, retry scalars use the
  `retryAfterMs` shape, and vendor ResourceType names qualify with the
  `d2bus.org` grammar.
- ADR 0046 Host/Guest execution policy: froze a single `defaultUserRef`
  invariant across the decision register, terminology, Nix, and resource specs -
  `defaultUserRef` is required whenever `allowedDomains` contains `user`.
- ADR 0046 ZoneLink bootstrap: specified IKpsk2 with an allocator-issued
  single-use PSK for initial cross-Zone enrollment and KK for the enrolled
  session, reserving the unauthenticated NN profile for local peer-credential or
  inherited-descriptor sessions.
- The policy meta-gate now executes the fixture-independent contract-test
  policy binaries directly and fails closed if any of them is skipped, filtered
  to nothing, or reports zero tests. These binaries are excluded from the
  workspace test run, so the policy gate is now their guaranteed execution
  point in CI.
- The changelog gate now classifies deletions and every executable and
  configuration surface (shell, Makefile, workflow, and data manifests) as a
  code change, so a removed module or a shell-only behaviour change can no
  longer ship without a release note.
- The heavy-lane semaphore now verifies a nested invocation before reusing a
  slot: the inherited descriptor must be an open handle on the real,
  per-uid slot file it names, proven by an atomic open file description lock
  through the inherited descriptor itself. A forged, stale, or closed marker
  no longer skips acquisition; it acquires its own slot or fails closed.
- The heavy-lane semaphore namespace is anchored to a verified directory
  descriptor. Its root and per-uid directory must be root-owned and
  non-writable by group or other; an unsafe directory is rejected rather than
  repaired or used as a fallback.
- Every public heavy lane (`test-integration`, `test-host-integration`,
  `test-hardware`, `perf`, and the umbrellas `test`, `check-ci`, `check-all`
  that invoke them) now acquires a heavy-lane slot itself; the raw work moved
  behind internal targets guarded against direct execution outside the gate.
- The hermetic runtime-ledger gate now warm-builds before timing, collects
  repeated execution-only samples at test and crate granularity, and enforces
  a complete, comparable census: a repetition floor, non-empty scopes,
  matching per-sample repetition counts, and detection of census ids dropped
  from a run. Its cargo invocations run from the workspace directory so
  the configured compiler wrapper is discovered.
- ADR 0046 spec set: advanced the universal-status sweep so more Accepted
  resource examples serialize the universal envelope with its `status.resource`
  (D107) and `status.update` (D091) currency object, including the flat-status
  shell-pool and shell-session examples that the first pass left populated but
  un-nested. This pass did not reach every complete envelope; the residual
  complete envelopes still missing `status.update` are swept in the ADR046-W0fu3
  pass.
- ADR 0046 ZoneLink profile: aligned every normative handshake statement across
  the zone-routing, Unix transport, and Azure Relay Provider specs so a ZoneLink
  consumes the allocator-issued single-use PSK exactly once under IKpsk2, persists
  the enrolled static identity, tears down or rekeys the bootstrap session, and
  only then establishes the enrolled KK session before Ready or resource traffic.
  Enrolled steady-state and credential-acquisition KK sessions are unchanged.
- ADR 0046 Host execution policy: the mixed-domain Host example now supplies the
  `defaultUserRef` its compiled output requires, and a companion rejection example
  shows the missing-`defaultUserRef` shape that D116 fails closed at Nix eval, so
  the superset invariant is illustrated from both sides.
- The ADR 0046 spec-literal drift lints now exempt a rejected literal only on
  the decision-register row that defines the rule it violates. The previous
  generic inline allow marker, which could suppress a real violation from any
  line in any spec file, is gone, so a lint can no longer be silently defeated
  where it matters most.
- The changelog gate now classifies patch and protocol-definition files
  (`.patch`, `.proto`) and every other unrecognized extension as code by
  default, exempting only an explicit prose and data allowlist. A patch-only or
  protocol-only change, including a deletion, can no longer ship without a
  release note.
- Every live and hardware test entrypoint, plus the enforcing path of every
  performance entrypoint, now routes through the heavy-gate semaphore. The
  performance advisory skip exits before acquiring a slot because it does no
  heavy work. The release smoke lanes and the aggregating and per-layer runners
  re-exec through the gate exactly once when invoked directly, and an inventory
  guard fails closed if a new live entrypoint or bare heavy make target is added
  without gating.
- The runtime execution-budget ledger now enforces a pinned closed census: it
  requires a census, records advisory per-test wall clock from warmed,
  crate-qualified libtest streams, records enforced aggregate process CPU for
  each complete crate invocation, reproduces the expected test and crate sets
  exactly, rejects census id loss and repetition mismatch, and runs as a
  required Layer-1 job. It holds no baseline and makes no
  historical-regression claim.
- Reconciled the live-host testing instructions with the semaphore routing
  that now ships: a live script invoked directly re-executes itself through
  the heavy gate exactly once, so it cannot bypass the sole-use invariant,
  and any new live, hardware, or performance entrypoint must carry the same
  self-guard block or the fail-closed inventory guard rejects it.
- Contributor docs: aligned the test quick-start with shipped behaviour.
  `tests/README.md` no longer claims that invoking a live script directly
  bypasses the heavy-gate semaphore, matching `tests/AGENTS.md` and the scripts'
  self-re-exec self-guard. `AGENTS.md` and `tests/README.md` now document the
  `test-runtime-ledger` Layer-1 job as an aggregate per-crate process-CPU
  budget gate with advisory per-test wall-clock diagnostics, no baseline, and
  no historical-regression claim, noting that the `Makefile` and
  `tests/layer1-jobs.json` are authoritative if the prose diverges.
- The ADR 0046 ResourceType and retry-scalar lints now run the repository
  scanner through the same exact validators as their unit tests instead of a
  looser set of regex substrings. The type lint extracts complete tokens from
  authoring contexts and rejects an unknown unqualified name such as
  `type: Widget`, a malformed qualified token such as `acme.d2bus.org.1Widget`
  or `acme.d2bus.org.Widget_Type`, and any token whose grammar the scanner's
  older reject set admitted but Nix would refuse. The retry-scalar lint now
  accepts only a bare decimal integer inside the frozen range, rejecting `0`,
  an out-of-range value, and non-integer values such as `true`, `null`, and
  `-1`.
- The ADR 0046 datetime lint now extracts the value from timestamp-bearing
  authoring fields and validates any value that presents as a date, catching a
  malformed instant that falls outside the lint's earlier candidate shape.
- The ADR 0046 spec-literal exemption is now bound to the exact canonical file
  and a uniquely parsed decision-register row, rather than a filename suffix
  plus a row prefix, so it can no longer be satisfied by a lookalike file or a
  non-canonical row.
- The runtime execution-budget ledger now honestly enforces aggregate
  per-crate process-CPU p95 while keeping per-test wall-clock p95 advisory. It
  records genuine repeated samples, recomputes p95 from those samples, audits
  that every scope is measured on every repetition, requires the crate and test
  census to reproduce a pinned closed set exactly, and runs the hermetic
  placement lint over the census crates' integration tests. It no longer holds
  a synthetic baseline or claims historical-regression detection; the gate now
  runs as part of the local pre-merge check as well as in CI.
  Growing the census to a real multi-crate shard inventory and adding a genuine
  cross-machine reference baseline for a true historical-regression gate is
  tracked as the deferred follow-up
  `runtime-ledger-full-census-and-real-shards`.
- Aligned much of the D094 test-runtime ledger prose across the decision
  register, validation/delivery §10.16, feasibility/spikes, streamline, and the
  generated implementation graph toward the ledger's actual scope: enforced
  aggregate per-crate process CPU, advisory per-test wall clock, no baseline,
  and no historical-regression claim. Growing the census to a real multi-crate
  shard inventory and adding a cross-machine reference baseline are recorded as
  the deferred follow-up `runtime-ledger-full-census-and-real-shards`.
  Deleted the synthetic `runtime-ledger-baseline.json`. Retired
  baseline/regression/shard references that survived in the code, the
  `Makefile`, and several docs are reconciled in a later follow-up.
- Froze the canonical per-Zone bundle digest chain (D119) as four fully
  computable members: the bundle `contentHash` over the canonical sorted
  `resources` array (which also serves as the generation identity and the
  preimage covering every bundled envelope), `providerSchemaDigests`, the
  artifact-catalog document digest carried as `catalogDigest` and anchored in
  each bundle's Nix-store-immutable `artifactCatalogDigest`, and per-artifact
  store-path hashes verified at apply. Removed the unpinned self-digest claim
  that could not detect tampering, and named the store-side
  `d2b:v3:resource-envelope` tag as explicitly outside the bundle chain.
- Closed the D101/D119 digest-domain gap: added the `d2b:v3:resource-bundle`
  domain tag to D101's frozen set and specified the bundle `contentHash`
  (`generationId`) as the D101 digest under that tag over the canonical sorted
  `resources` array, so the generation identity is domain-separated like every
  other digest. Stated in the canonical bundle contract that the
  content-addressed identity (`contentHash`/`generationId`, a `sha256:<hex>`
  string used by the D125 firewall fence and the `generation.json` pointers) is
  distinct from the host-local monotonic configuration-generation ordinal
  (`configurationGeneration`/`generationIndex`, surfaced as the redb
  `store_meta.active_configuration_revision` pointer used by the D106 policy
  recheck).
- Made provider-catalog identity decidable (D120): `spec.artifactId` must be
  unique across `d2b.providerCatalog` entries, enforced by an eval-time
  assertion (`provider-catalog-duplicate-artifact-id`), so "resolves exactly one
  entry" is enforceable rather than relying on attrset key uniqueness.
- Assigned a single durable writer for configuration activation (D122):
  `generation.json` (active pointer, prior pointer, retention metadata) is
  committed in one atomic durable write before any diff application or
  reconcile notification, and every other activation path defers that
  commit to the sole writer. Restart recovery follows ADR 0034: recover, adopt,
  or quarantine before any cleanup.
- Redefined the host-firewall projection generation fence (D125) as the
  immutable installed configuration generation (`expected_generation_id`),
  removing the unimplementable live projection counter and its
  compare-and-advance; concurrent same-projection mutations serialize on the
  ordered OFD lock over the `inet d2b` table and converge idempotently.
- Froze the ZoneLink enrollment and key lifecycle (D126): the one-time IKpsk2
  bootstrap session is terminated after enrollment and never rekeyed or
  continued, a distinct enrolled `Noise_KK` handshake from a durable
  `EnrollmentCommitted` state must complete before `Ready`, and the bootstrap
  PSK TTL, KK cryptoperiod, and every authentication-failure transition are now
  frozen protocol constants with no fallback below the enrolled KK contract
  short of durable revocation.
- Forbade cross-Zone L2 sharing on multiplexed external physical NICs (D127):
  the Host-global external physical-NIC authority binds an isolation domain
  equal to the claimant Zone UID, `bridge` multiplexing is admitted only among
  same-Zone claimants, and a cross-Zone `bridge` multiplex is rejected fail
  closed (`external-physical-nic-cross-zone-l2`) so work and personal Zones
  never share an L2 broadcast domain.
- Major test suites now report their wall-clock duration, including the Rust
  workspace test pass and the runtime-ledger gate, so a non-failing performance
  regression is visible without imposing a flaky time budget.
- The manifest-driven local Layer-1 gate now includes the changelog policy job,
  and manifest loading rejects any CI job with a local Make target that is
  absent from all local phases.
- The changelog policy gate now accepts release notes as either a `CHANGELOG.md`
  entry or a `changelog.d/` fragment, and additionally validates the structure
  of every fragment present so a malformed fragment fails on the pull request
  that introduced it rather than when the fragments are folded.
- Replaced the resource API spec's `## Limits` section, which asserted that
  bounds were frozen but listed only the axes with no values, with the frozen
  numeric tables and the derivation anchor for each value. Over-limit input now
  has a defined rejection class, so admission control is implementable and the
  section no longer claims a property it did not have.
- Closed the resource API spec's `## Errors` class set at exactly 31 classes.
  It previously read as an open list, which left the wire enum unbounded, and
  gave the bounded error `reason` an explicit 512-byte ceiling and redaction
  rule.
- Only the plain ASCII hyphen `-` may now spell a dash anywhere in the
  repository. Every non-ASCII dash codepoint is banned and every existing
  occurrence was rewritten: a dash that separated clauses became a spaced
  hyphen ` - `, and a dash that joined a range or a compound closed up to `-`.
  Documentation, specs, ADRs, comments, CLI text, and generated artifacts are
  all affected.
- The delivery workflow success JSON is now a pinned, version-coupled contract.
  Its `operation` and `status` values are typed closed domains rather than free
  strings, and a golden contract test fixes the complete wire shape (field
  names, omitted-when-empty optional fields, and both value domains). An
  incompatible change to the shape or either domain now fails the build unless
  it travels with a `schema_version` bump, so a consumer that reads this JSON
  can no longer break silently against a drifted producer.
- The ADR 0046 envelope and spec-literal lints now parse fenced YAML, JSON, and
  Nix blocks into a structural document model and assert over the parsed tree,
  instead of matching line shapes. Line-oriented heuristics were the root cause
  of a family of fail-open lints that matched the examples their author happened
  to look at rather than enforcing the format they claimed to police; a block
  the parser cannot model now fails closed rather than being silently skipped.
- The universal-status lint now checks resource envelopes written as JSON as
  well as YAML, treats a document that is missing a frame key as a candidate to
  check rather than one to skip, honours a `...` elision only as a direct child
  of `status` (not anywhere in the status subtree), and rejects an inline
  `status: {}` or `status: null` on a live envelope. Compiler-emitted bundle
  envelopes, which carry `resourceType` and an explicit `status: null`, are
  recognised as a distinct contract and are not required to carry a status base.
- The D116 Host/Guest lint now reads a multiline `allowedDomains` list the same
  as an inline one, evaluates each document in a fence independently so one
  `defaultUserRef` cannot satisfy a different document, ignores a commented-out
  `defaultUserRef`, and pins the intentional-negative-example exemption to the
  exact file and the single unique marker that needs it, so the marker fails
  closed anywhere else.
- The D103 datetime lint now validates the complete alphanumeric-delimited token
  rather than a conformant prefix, so a valid instant with trailing or leading
  junk such as `2026-07-22T00:00:00.000Zjunk` is rejected. The D104 ResourceType
  lint now validates the `type` and `resourceType` fields of a quoted JSON
  envelope and an indented Nix resource declaration, not only a bare top-level
  YAML `type:`. The D108 retry-scalar lint now fails closed on any value that is
  not a bare decimal in range, rejecting tokens such as `1e3`, `banana`, and
  `nonsense` that the previous fall-through silently accepted, while still
  exempting a Rust type annotation and a `<placeholder>`.
- The delivery workflow now binds the complete expected set of pull requests
  and heads for every repository in the snapshot, and merge eligibility
  requires the supplied pull requests to match that set exactly and to produce
  a deterministic integrated tree covering every slice. A wave of parallel
  same-repository slices can no longer be declared merge-eligible while a slice
  and its required checks are silently absent from the merge target. The
  published merge-target recipe now requires one object per pull request, and
  the persisted delivery artifact schema version is bumped accordingly.
- The delivery status domain and its serialization are now generated from a
  single declaration, and the pinned wire fingerprint serializes an exhaustive
  contract probe with every optional field populated. Adding a status outcome
  or an optional field, or populating a field a stage previously left unset,
  now fails the build unless it travels with a `schema_version` bump and a
  matching golden.
- The delivery workflow success JSON now pins a generated wire fingerprint
  keyed to its `schema_version`. The fingerprint enumerates both value domains
  from the types themselves, the full field set of every emitted object, and
  the serialized shape of a representative output for every wave stage, so
  adding a status outcome, a stage, or an optional field, or changing any
  stage's shape, fails the build unless it travels with a `schema_version`
  bump and a matching golden. A consumer reading this JSON can no longer break
  silently against a producer whose shape drifted without a version change.
- The ADR 0046 envelope and spec-literal lints now build their document model
  with real parsers instead of a hand-written multi-language reader: JSON is
  parsed by `serde_json`, YAML by `serde_yaml_ng`, and Nix by `rnix`. Each
  parse has an explicit error channel and every caller treats a parse error as
  fail closed, scoped to the check's genuine authoring trigger, so a block a
  parser cannot model is reported rather than silently skipped. The previous
  hand-written parser mis-modelled valid syntax (it discarded `rec { ... }`
  attrsets, left JSON `\uXXXX` key escapes undecoded, and mishandled YAML
  anchors, tags, and merge keys), which let real violations disappear.
- Envelope classification is now per document rather than per fence. Each
  `apiVersion` document resolves to exactly one of a live resource envelope, a
  `resourceType` bundle envelope, or an explicit unrecognised case that fails
  closed, so one recognised envelope can no longer mask an unrecognised sibling
  in the same fenced block, and a document carrying neither `type` nor
  `resourceType` is flagged rather than classified as nothing.
- The D116 negative-example exemption now binds to exactly one parsed resource
  rather than a whole fenced block. The marker comment is read from the parsed
  document and suppresses only the single resource map that lexically contains
  it, so an unmarked, genuinely violating resource beside the marked teaching
  example in the same fence is still reported. The exemption remains pinned to
  the one spec file and a single marker occurrence, and fails closed otherwise.
- The universal-status lint now scans Nix fences in addition to YAML and JSON,
  decodes JSON `\uXXXX` key escapes so an escaped `type` key is classified as a
  live envelope, folds YAML `<<` merge keys before judging the assembled status,
  and honours elision only as `status: ...` or a direct `...` marker key, never
  as a `...` value on some other status field such as `conditions`.
- The D103 datetime, D104 ResourceType, and D108 retry-scalar lints now inspect
  the complete parsed scalar in value position across YAML, JSON, and Nix rather
  than matching a line shape. A key and value split across lines, a
  punctuation-suffixed timestamp such as `2026-07-22T00:00:00.000Z_junk`, an
  over-qualified `type: "acme.d2bus.org.Widget.Type"`, a quoted JSON
  `"retryAfterMs"` key, and a non-finite `retryAfterMs: NaN` value are all now
  rejected where the previous line-regex passes let them through.
- The `test-runtime-ledger` gate now records per-test wall-clock p95s as
  advisory diagnostics and enforces aggregate per-crate process-CPU p95
  budgets. Process CPU excludes time descheduled behind unrelated machine
  load. It holds no baseline and makes no historical-regression claim. A
  genuine cross-machine reference baseline and a real multi-crate shard
  inventory are the deferred follow-up
  `runtime-ledger-full-census-and-real-shards`.
- Removed the shard dimension from the gate entirely, in both the `Makefile`
  recipe and `packages/xtask/src/test_runtime_ledger.rs`. Every shard had been
  assigned the identical per-crate aggregate and no shard target was ever
  executed, so the ledger no longer records, checks, audits, or reports a shard
  scope, and `tests/runtime-ledger-census.json` no longer pins a shard set.
  Real shards land only with the named deferred follow-up.
- Reconciled the remaining ledger prose across `AGENTS.md` and
  `tests/README.md`, so no surface advertises the removed per-test enforcement,
  baseline, historical-regression, or shard capabilities (there is no
  committed baseline file).
  Regenerating the census pin after a legitimate test change is a separate,
  supported step: `make runtime-ledger-pin`.
- Documented the envelope policy lint's D116 negative-example marker in
  `AGENTS.md` beside the existing lint guidance: the `policy_adr046_envelopes`
  lint exempts an intentional teaching block that demonstrates the D116
  eval-time failure only in the pinned documenting file and only when it carries
  the exact `d2b-lint: expect-d116-eval-error` marker. The guidance frames it as
  a narrowly scoped intentional-rejection signal rather than a general
  suppression switch.
- Redesigned the per-Network host-firewall model so it no longer maps each
  dynamic per-Network `FirewallIntent` onto the whole-table `ApplyNftables`
  broker request. Because the shipped `ApplyNftables` op discards its
  `ownership_id` and atomically deletes and recreates the entire `inet d2b`
  table, mapping per-UID Network reconciles (and per-UID deletion) onto it
  made independent Network projections last-writer-wins and erased other
  Networks' rules and the device-usbip firewall rules. The `Provider/network-local`
  and `ADR-046-resources-network.md` specs now define a new closed broker
  operation, `ApplyNftablesProjection`, that atomically applies or removes
  exactly one validated, generation-fenced ownership projection resolved from
  the private bundle, byte-preserves every other Network and USBIP ownership
  marker, fails closed on a foreign marker (`foreign-nft-rule-preserved`), and
  returns a projection-scoped digest. Decision D-NETWORK-004 records the
  rationale and the cross-provider invariant that any provider mutating the
  `inet d2b` table must use a projection-scoped op.
- ADR 0046 Nix configuration contract: bounded `ZoneId` and `ResourceName` to
  1 to 63 bytes at eval time (`^[a-z][a-z0-9-]{0,62}$`) everywhere the generated
  options and reference validators are specified, with 63-accept/64-reject/empty-
  reject boundary coverage, so a name that Nix accepts is always admissible to
  the resource admission layer instead of failing far from its cause at runtime.
- ADR 0046 Process contract: replaced the floating-point `backoffMultiplier`
  field with integer fixed-point `backoffMultiplierMilli` (multiplier x 1000),
  so every canonical rendering is float-free and round-trips through digest
  computation.
- Aligned the ADR 0046 USBIP host-firewall model onto the same closed
  `ApplyNftablesProjection` broker op that `Provider/network-local` uses, rather
  than the shipped whole-table `UsbipBindFirewallRule` op. Because the shipped op
  replaces the entire `inet d2b` table, an independent USBIP reconcile through it
  would erase Network-owned rules, violating the ownership-marker preservation
  contract (`foreign-nft-rule-preserved`). The `device-usbip` provider now maps
  `apply_firewall`/`release_firewall` onto `ApplyNftablesProjection` with actions
  `Apply|Remove`, resolving the per-Network/per-busid ownership projection from
  the integrity-pinned private bundle, fencing on a projection generation,
  treating a validated already-absent projection as idempotent success,
  byte-preserving every sibling network-local and device-usbip marker, failing
  closed on a foreign marker, and returning a projection-scoped digest. This
  conforms USBIP to decision D-NETWORK-004, whose cross-provider invariant
  requires any provider mutating the `inet d2b` table to use a projection-scoped
  op. New validation cells cover concurrent USBIP apply, concurrent independent
  release, and preservation of a network-local marker across USBIP apply and
  release.
- The removal-proof obligation for a superseded path now binds the release that
  performs the removal rather than the one that first recorded it, so a closed
  release is never asked for evidence it cannot produce. This changes which
  release owes the proof and never whether a removal needs one.
- Provider diagnostics are redacted throughout. A publisher, artifact
  identifier, digest, component identifier, configuration object, and
  installation decision now render as a discriminant or a count rather than as
  the value they were handed, so a third-party artifact cannot author text that
  reaches a log line.
- The `system-systemd` and `system-minijail` Process Providers now treat a
  Guest execution parent exactly as they treat a Host: a process launched
  under either reports the same status apart from the execution reference,
  and a user-domain launch requires the same exact user identity under both.
- Public status from the system Providers carries no user name, home
  directory, shell, numeric identity, unit name, cgroup, or path. A user
  resource's declared operating-system username is never restated in its
  status.
- The `d2b-provider-volume-local` and `d2b-provider-volume-virtiofs` READMEs are
  restructured onto the nine standard Provider documentation sections, so every
  Provider is documented in the same shape. All previous content is retained.
- The security-key family publishes no closed set of allowed same-Zone
  backing reference types, because its base names no backing resource.
  Deriving a projection factory for that family fails closed with
  `semantic-backing-ref-types-undetermined` rather than assuming one.
- The telemetry and USB common status layers publish only the field names
  their specifications state. Observations whose spellings are not yet fixed
  are rejected rather than guessed, so an implementation that needs one must
  wait for the name to be published.
- Added core planning for foreign-owned name conflicts, unchanged-resource refresh, and finalizer-safe cleanup of removed configuration-owned resources. No production store/watch adapter currently executes that plan.
- Applied the accepted Provider derivation layout amendment across the
  authoritative ADR-046 specifications and regenerated their registries.
- Enforced the semantic projection backing contract with an explicit
  no-backing or constrained declaration, deny-all empty backing sets,
  stored-envelope origin admission, declared projection-protocol versions,
  and complete generated factory metadata.
- Clarified semantic projection factories, deny-all security-key backing,
  import-owned origin rejection, and typed provider admission contracts.
- Wired d2bd Provider registration and Guest lifecycle effects through the
  shared Provider registry and typed broker lifecycle path, with fail-closed
  catalog and caller admission.
- Updated current CLI documentation and live smoke guidance to use typed Guest,
  Device, activation, and EphemeralProcess ResourceRef commands with v3
  Resource envelope examples.
- Adds capability, platform-gate, NSS observation, and bootstrap ordering
  contracts for system execution.
- Preserved legacy Nix emitters during the migration while making Zone
  resource validation reject runtime metadata, raw paths, and secret-shaped
  values before publication.
- Reuse evaluated provider projection and audited observability scenarios
  across equivalent Nix-unit assertions.
- Registered the device and observability Provider crates in the Rust
  workspace and removed the unused duplicate resource client.
- Kept the legacy CLI parser internals private while separating them from the
  native command surface.
- Routed CLI EphemeralProcess attachment through the typed Zone
  ComponentSession client, preserving bounded refusal and close behavior
  instead of sending the retired OpenTerminal request directly.
- Zone resource bundle builds now invoke the deterministic Rust resource
  compiler for Provider artifact, schema, digest, ordering, and secret-policy
  validation.
- Extended the Process reaction benchmark through the authenticated named
  stream, controller toolkit runner, Process Provider, and durable status
  path, retaining hard handler and launch p95 budgets at 1/10/100 scale.
- Narrowed the clean-break workspace retirement to unused daemon-access, host-provider, and userd stub crates while retaining live later-wave Provider and guest-control surfaces.
- Documented the required Provider crate, integration, test, and policy-check
  surfaces for the TPM and GPU implementations.
- Route modern Zone resource, endpoint, and share commands through the
  authenticated, bounded Zone resource client.
- Reunified the v3 Zone resource client around the canonical ResourceService
  methods, authenticated route/session pins, cancellation, and fail-closed
  target handling.
- The command-line client now keeps Guest, Host, and activation operations on
  the typed Zone resource path and reports unavailable Zones through the
  versioned JSON envelope.
- Cut `d2b shell open`, `attach`, `list`, `status`, `detach`, and `kill` over
  qualified ShellSession Resource requests and authenticated named streams,
  including partial-write-safe PTY I/O, resize and EOF forwarding, JSON
  create-without-attach, multi-target restart recovery, signal-safe
  detachment, and removal of the retired public shell socket protocol.
- Recorded the gated re-measurement of the disposable resource-store proof's
  whole-process resident-memory result. The corrected measurement is below the
  unchanged budget with no baseline subtraction, superseding the earlier
  failing conclusion while preserving both prior result records unchanged. The
  production store backend, watch dispatcher, and reaction benchmark are
  unaffected: each still owes its own measurement on the production engine
  before it can be accepted.
- Scope daemon coordination, configuration staging, and restart cursor
  adoption to their owning Zone and quarantine ambiguous ownership evidence.
- Standard v3 schema artifacts now carry the `core.d2bus.org_` filename
  namespace, generated CLI completions live under `completions/`, and the
  safe qemu-media evaluation example has a dedicated example directory.
- Centralized the guest static ELF smoke expression and preserved the existing
  static-binary assertions in the flake checks.
- Run standalone proof crates from writable per-test workspaces so Bazel
  execution cannot inherit the root Cargo workspace.
- Fail closed on ambiguous remote fallback failures, preserve dispatch hints
  through evidence redaction, and require a BEP `testResult` event on success.
- Fail Bazel local, remote, trusted-seed, and typed local-fallback runs on
  redacted log lines beginning with `warning:`, deny warnings for first-party
  Rust crates at compilation, isolate concurrent facade evidence, and remove
  the xtask schema, clang `--unwindlib=none`, and remote gold-linker warnings.
- Fail closed when a BEP references a non-local `test.log` URI that cannot be
  scanned for warnings.
- Cloud Hypervisor Guest bootstrap planning now derives a deterministic,
  UID-free direct-child graph and keeps VMM lifecycle eligibility pure until
  Device, Network, Volume, Export, and setup dependencies are ready.
- Converged the Cloud Hypervisor Guest package on its signed manifest,
  configuration schema, artifact packaging, and Guest-only Nix projection.
  Removed the retired controller-side VMM argv surface so launch effects stay
  behind the Process Provider.
- Bind Cloud Hypervisor Guest-session health evidence to bounded Guest,
  descriptor, schema, generation, Endpoint, and guest-seed commitments while
  keeping identity and transport details redacted.
- Pull-request validation returns faster. The gate no longer evaluates the
  Nix-unit corpus twice, and a check that has to be built rather than evaluated
  no longer queues behind two dozen short ones, so the same coverage reports in
  roughly 40% less wall time. Every check remains enforcing, and the dispatch
  fails closed rather than skipping a check it cannot classify.
- Report Rust formatting and changed-package clippy before long local validation
  jobs begin.
- The pull-request gate no longer rebuilds a patched crosvm and a patched
  cloud-hypervisor from source on every run in order to check two of their
  command-line flags. The two outputs those checks need are carried between
  runs instead, at 30 MB, so the shard that set the gate's wall time stops
  doing so. Measured on the gate: that shard falls from 1010 s to 33 s, with
  no build of either package. A carried entry that no longer matches simply
  builds as before, because store paths change with the derivation.
- The privileged broker's default and layer1-bootstrap test passes no longer
  run a `cargo check` immediately before their `cargo test`. The two are
  distinct compilation modes that share no artifacts, so the check reported
  the same errors slightly earlier at the cost of running the compiler twice.
  Measured cold, that is 153 s against 89 s for an identical result. The
  fake-backends pass never had one.
- The gate's Nix-store cache entry can now be replaced when its key changes.
  The job was already configured to purge the entry it supersedes, but the
  workflow granted no permission to delete one, so the purge failed after the
  replacement had been saved and the superseded entry stayed resident forever.
  Two such entries were resident at roughly 1.25 GiB each against a hard
  repository-wide budget, and overrunning that budget evicts entries other
  jobs depend on. The permission is granted to that job alone, which is the
  only one that deletes anything.
- The resource API's external capability seal reuses its fixture build between
  runs, keyed on the compiler that produced it, rather than rebuilding roughly
  a gigabyte of dependencies every time. Measured locally at 40 s against 2 s.
  The seal's proof is unchanged: it still demonstrates that the crate compiled
  under forced `cfg(test)`, by discarding that one crate's fingerprints while
  its dependencies stay warm, and it now discards the marker recording that
  compile at the start of every run - so if the forcing were ever to stop
  working the seal fails rather than passing without proof.
- Preserve the full validation surface while reducing pull-request feedback time, enforce compiler and documentation warnings, limit concurrent Nix evaluation to four workers, and keep the patched video command-surface contract realized.
- Materialize non-binary contract fixtures from evaluated configuration data while retaining production-shaped integrity semantics.
- Cloud Hypervisor Guest reconciliation now creates and repairs its direct
  Process, Endpoint, and Volume resources through an authenticated,
  UID-free Resource API batch before enabling the VMM lifecycle.
- Split broker and guest/public control wire contracts into the
  `d2b-contracts-broker` and `d2b-contracts-control` crates while preserving
  the existing runtime topology and wire contracts.
- Replace fake Cloud Hypervisor host acceptance with a Zone-native,
  controller-owned Guest boot, signed Provider artifacts, closure-backed store
  views, restart adoption, and deletion proofs.
- Use the Bazel-injected authenticated Provider controller for Volume host
  acceptance so both storage controller identities establish live sessions.
- Move Cloud Hypervisor Guest lifecycle ownership into the authenticated
  controller path, including restart adoption, session recovery, finalizer-safe
  deletion, and Bazel-injected host integration.
- Treat generated Guest store-view Volumes as configuration-owned watched
  dependencies instead of Cloud Hypervisor owned children.
- Moved ACA-specific effect contracts into the ACA runtime provider and removed
  stale cloud-provider contract and realm compatibility dependencies while
  preserving gateway composition and provider behavior.
- Credential Providers now use Guest-local typed lease registries for Secret
  Service, Entra, and managed identity operations. Lease metadata is
  idempotent and opaque, while delivery material remains zeroizing and crosses
  only the authenticated credential session; fail-closed backend composition is
  reserved for explicit degraded and negative paths.
- Developer Bazel commands and public Make aliases now default to BuildBuddy remote execution, while CI continues to select local execution.
- Public Make aliases now forward an explicit profile override and the repository-root test context to the pinned Bazel command.
- Consolidated desktop interaction code, binaries, contracts, argv generation,
  production Nix modules, and focused tests under their owning Providers.
- Device configuration now emits canonical, provider-bound resource
  specifications without exposing host paths or runtime management state.
- Kept public Make aliases and fixed CI jobs while moving Layer-1 scheduling,
  caching, retries, and aggregation into Bazel.
- Kept production Rust labels free of Cargo's `test-support` feature while
  retaining explicit same-crate Bazel variants for test-support graphs.
- Kept direct Cargo, nextest, doctest, feature, and manual Layer-2 workflows
  available alongside the Bazel graph.
- Declare package, deny-policy, and shell-completion sources for fixed Nix
  Bazel inputs so source changes invalidate the corresponding checks.
- Let CI use its native Bash when user namespaces are unavailable while local
  Bazel development keeps the FHS action shell.
- Keep local-only lint actions on the host platform while remote actions use
  the BuildBuddy Ubuntu GCC toolchain.
- Bound Entra credential acquisition, refresh, authorization, redacted status,
  bounded degradation, and finalization cleanup to the owning Guest and Zone.
- Require exact authenticated Guest, Provider, and Zone bindings before any
  client effect, preserve committed refresh metadata after post-commit
  validation failures, and prevent finalized credentials from minting again.
- Bind every service operation to the authenticated Credential session and
  delivery context, persist remote lease state, reject generation rollback,
  accept Unix-millisecond deadlines, and cap projected retry state.
- Revoke newly issued leases when replacement validation or local commitment
  fails, retaining degraded cleanup state when remote revocation is ambiguous.
- Keep proactive refresh available during an ambiguous replacement retry, and
  keep that replacement idempotency key after a successful refresh.
- Hardened Secret Service credential lease state, idempotency, timeout, and
  disconnect/finalization recovery while keeping credential material opaque.
- Cache source-built host-tool dependencies across acceptance VM checks so
  in-tree Rust changes rebuild only affected binaries instead of the full
  workspace.
- Make, contributor, test, and pull-request authority now use the fixed
  Bazel Layer-1 aggregate and owner-local Bazel labels; Cargo manifests and
  lockfiles remain rules_rs metadata inputs rather than gate entrypoints, and
  fixture inputs no longer make documentation-only changes rerun CLI contracts.
- Make public check and test aliases select the pinned Bazel development
  environment automatically, while keeping BuildBuddy locality and CI trust
  boundaries explicit. Bazel now owns the complete Layer-1 composition through
  nested public and package-level test suites instead of a duplicated Make
  label graph. Remove the redundant `make test` and `make check-all`
  convenience aggregates in favor of explicit conditional lanes.
- Store durability and throughput: concurrent writes coalesce before fsync
  (drain-only coalesce window), status-class commit groups use relaxed
  durability, and the read lifetime under commit load was raised from 250 ms
  to 2 s with an extended startup source-retry budget.
- Fences no longer carry assignment epochs; succession is determined by
  generations and revisions alone, and pure-stale predecessors are adopted
  instead of wedging runners.
- Control-plane silent failures now log: a telemetry sweep across 31 crates
  (~2450 sites) replaces dropped errors with tracing; slow reads/writes log
  queue depth and bring-up phases log permanently.
- Changed Volume-side attachment admission and binding minting to the neutral
  durable `VolumeBinding` resource, minted by the Volume side with deterministic
  binding identity per Volume / execution-target / named-view relationship.
- Changed `volume-virtiofs` to reconcile `VolumeBinding` with sole authorship of
  the fenced binding status projection (KTD3): readiness is fenced by UID,
  generation, and revision, stale reports are never accepted as ready, and guest
  start is gated on current binding readiness.
- Changed binding deletion to drain the virtiofsd worker and private endpoint
  (and wait for a present guest mount to clear) under the
  `volume-virtiofs.d2bus.org/volume-binding` finalizer, so no serving effects
  are orphaned.
- Changed policy-rooted Volume resolution to provision one subdirectory per
  Volume under the policy root, so sibling volumes and unrelated daemon state
  never trip the unmarked-content guard and serving scopes to the Volume's
  own tree. The policy root itself must be daemon-writable.
- Inverted foundational contract ownership so `d2b-contracts` no longer
  depends on `d2b-core` or `d2b-realm-core`, while preserving wire
  serialization, validation, and redacted diagnostics.
- Made `d2b-contracts::v3::IfName` the sole interface-name owner and migrated
  core, host, daemon, and broker consumers to it.
- `ManagerCall` gains `GetView`; `ChannelManagerEndpoint`,
  `ManagerActorEndpoint`, and the in-tree driver test doubles implement the
  new trait method. Production reads are served by
  `ResourceManagerMsg::Get`, which already answered `ResourceView` - no
  second status store and no store access on the path (KTD12).
- Build and test: split Nix checks into fixed owner-surface Bazel actions with
  explicit pure-evaluation inputs and standalone isolated evaluation.
- Tightened shared resource reconciliation around bounded single-transaction
  mutations, fresh re-entry after commits, durable effect acceptance, and fair
  per-resource scheduling.
- Preserved newer queued operation identities when an ordinary reconciliation
  retry encounters a conflict.
- Cut observability Service/Binding and NixOS generation reconciliation onto
  the shared Runner, keeping ComponentSession stream-only and activation
  metadata Core-owned.
- Added fail-closed activation trust, artifact-catalog, redaction, ambient
  credential-chain, and zeroizing buffer boundaries.
- Hardened production startup and rebind handling so U12 runner attachment,
  exact-target reconciliation, finalizer enrollment, and trust verification
  fail closed without leaking untracked work.
- Removed placeholder Provider scaffold outputs from the flake; only verified
  Provider artifacts remain package outputs.
- Retried bounded transient StoreBackpressure/Timeout during Zone startup,
  activation, Core/provider relists, and Process composition with delayed
  async backoff, waitable bounded redb read admission, preserved typed store
  diagnostics, and readiness withheld on exhaustion.
- Live Cloud Hypervisor inputs and Wayland session lookups now validate exact
  StoreList snapshots against their returned revision, so status-only Provider
  updates do not invalidate unchanged typed configuration.
- Replaced only finished Core runner identities during reconciliation, keeping
  healthy Core and Provider siblings live and ensuring every required runner
  identity is present before reporting success.
- Preserved the last successful required Core runner identity set across failed
  replacement setup, keeping readiness fail-closed until every required
  identity has a live replacement.
- Added bounded relist recovery coverage for transient source backpressure,
  including typed exhaustion and the existing watch-backpressure no-relist
  path.
- Cleared deferred watch-hint state across relist recovery, fenced delayed
  pre-relist watch tasks from resurrecting stale work, and cleaned up newly
  spawned Core runners when task ownership is poisoned.
- Bounded transient Core/Provider source backpressure retries across startup,
  watch recovery, and per-resource passes; dead required Core runner tasks now
  withhold readiness until the next lifecycle reconciliation replaces them,
  with non-sensitive Zone/resource-operation diagnostics. Core admission
  backpressure now retries the watch without relisting, while a closed store
  watch maps to disconnect recovery; watch-hint queue pressure defers through
  the runner scheduler without blocking worker joins. Source-pressure
  exhaustion keeps its source classification instead of becoming a handler
  failure, and per-resource source retries remain limited to fresh-read
  recovery rather than authorization or post-effect persistence failures.
- Finalized the Zone Provider cutover with target-scoped activation and
  telemetry reconciliation, exact 27-Provider composition proof, and
  Provider-filtered descriptor validation.
- Materialized fixed Process Provider identities, re-armed controller-session
  reconciliation on Process changes, and gated Guest sessions on live VMM
  identity and Cloud Hypervisor API readiness.
- Allowed sparse Zones to attach only shared Provider runners backed by
  committed resources, while still refusing a missing Provider that owns work.
- Applied the same fail-closed ownership check to Credential, storage,
  interaction, Guest, and observability runner startup paths.
- Made sparse interaction composition explicitly absent-aware, kept present
  U9 Providers on filtered watches while refusing incomplete identity, accepted
  schema-valid system-core Users without a synthetic providerRef, and withheld
  daemon readiness on mandatory Process runner failure with bounded diagnostics.
- Bound Process assignment fences to the Core controller identity and aligned
  system Process finalizers with the canonical Provider finalizer namespace.
- Kept authenticated system-core status and finalizer projections local to
  resource audit while retaining broker evidence for desired-state mutations.
- Preserved that broker-evidence classification when pending audit outboxes
  are normalized during crash recovery.
- Replaced the inert host acceptance controller with an authenticated fd10
  ComponentSession fixture, made controller-session shutdown ownership explicit,
  and retried bounded watch revision conflicts during relist/open-watch recovery.
- Routed the acceptance controller through the Bazel-built host-tool bundle so
  the host VM lane does not rebuild its fixture controller through Nix.
- Reconnected the fd10 acceptance controller across daemon restarts, kept
  Ready Process observation read-only, and rebased exhausted Runner status
  projections to the exact target revision without killing the healthy runner.
- Derived Core Provider readiness from fenced controller Process/session
  evidence and admitted declared qualified ResourceTypes, including the
  virtiofs Export child type, before Provider-owned child reconciliation.
- Matched controller-session Provider generation evidence exactly, preserved
  Process phase and observed-generation ownership while persisting session
  projections, and retained live controller authority for retry when durable
  session clearing fails.
- Kept qualified API catalog admission limited to trusted U6-U9 declarations;
  unknown bundle ResourceTypes now fail closed instead of becoming implicit
  universal API bindings.
- Kept exhausted status conflicts inside a bounded persistence-only loop so
  retries retain effect identity without re-running accepted Process effects.
- Classified transient persistence timeouts as bounded status retries, kept
  integrity failures fail-closed, and threaded accepted effect identity into
  the production Core/Resource API ledger update.
- Continued status mutations under the accepted effect operation identity and
  retained the authority row across durable status retry and reopen.
- Reused the supplied effect operation on capability-miss status retries,
  rejecting identity mismatches instead of falling back to projection IDs.
- Validated capability-miss retries against the durable effect row and exact
  UID/generation/revision before reusing the supplied effect operation.
- Resumed pending or retryable authority rows on capability-miss retries and
  refused to report persistence success while the ledger remained nonterminal.
- Serialized controller-session fencing before durable evidence clearing,
  rebound evidence writes to the live system-core status client, and retried
  missing evidence for active sessions without starving unrelated controllers.
- Isolated stale controller-session admission markers, separated session
  evidence operation identities from Process status writes, and required
  Volume readiness to match the current Provider owner UID and the Volume's
  own Ready/observed-generation fence; owner generation remains an
  effect/write fence rather than a read-side readiness dependency.
- Fenced controller-session evidence by Process UID and generation, cleared
  stale teardown evidence without retaining a fenced live session, replaced
  Provider dependency census with owner-scoped child reads, and persisted
  Volume owner generations through the Resource API snapshot path.
- Kept fixed system-core and system-minijail Providers on declared Zone/Host
  readiness evidence, required explicit Provider-owned Volume progress, and
  rejected stale owner UID observations without requiring a matching child
  owner generation on the read side.
- Isolated missing Process owner identities and stale controller-session
  teardown from unrelated resources while preserving fail-closed effects and
  global store/authentication failures.
- Bound Provider-owned Process and Volume dependency reads to the exact owner
  UID while retaining stale rows only for that owner, and split global Host and
  Zone evidence from the owner-scoped query.
- Propagated missing Process owner identities through plan, effect, observe,
  and finalize retry paths, retrying transient owner reads without treating
  missing identity as convergence.
- Cleared stale controller-session evidence after teardown even when the
  current Process owner no longer matches, while retaining UID, generation,
  revision, conflict, and expiry fences for retry.
- Accepted legacy Process rows without owner-generation metadata for read-side
  Provider readiness, isolated Provider identity and session-evidence conflicts
  to the affected context, and kept ingress teardown ahead of durable session
  clearing with retryable session authority.
- Accepted legacy Process rows without owner-generation metadata for active
  controller-session evidence while retaining explicit owner-generation
  mismatch fences.
- Refreshed cached Process owner UIDs at each bounded reconcile boundary so
  owner delete/recreate cannot reuse a stale identity.
- Deferred transient Process owner identity refresh failures without removing
  desired rows or stopping live effects, and fenced stale stored owner
  incarnations before adoption or replacement.
- Kept controller-session evidence reads context-local for target store
  timeout, backpressure, unavailable, and revision errors while preserving
  global integrity failures for propagation.
- Kept post-teardown controller-session evidence clears context-local and
  retryable without restoring dead transport authority or aborting sibling
  Provider fencing.
- Preserved stored Process owner UID and generation on status-only session
  evidence writes, and selected Core-owned dependencies by bounded ownerRef
  filters so stale same-name children remain visible for Core fencing.
- Routed controller-session evidence through the assigned Process controller
  status path with exact UID, generation, revision, and assignment fences, while
  retaining a distinct evidence operation identity.
- Invalidated and rebuilt the assigned Process API with each system-core
  session rebind under the controller-session guard, preventing stale
  controller-session evidence from crossing assignment or session fences.
- Classified replayed `assignment-required` failures as retryable resource
  conflicts and kept their ResourceError wire representation valid.
- Routed Cloud Hypervisor Guest status, finalizer, and child mutations through
  the current U6 assignment fence instead of the unassigned system-core
  client.
- Preserved durable controller-session evidence when later Process status
  projections are committed from stale snapshots.
- Rotated per-Provider assignment epochs when controller-session authority
  changes, preventing stale U7/U6 fences from stopping their runners.
- Scoped shared Provider assignment relists to each controller's exact
  provider selector so sibling Providers cannot rotate one another's fences.
- Added bounded Provider-effect diagnostics for failed Guest and Volume
  resource passes without weakening their retry or readiness behavior.
- Provisioned the daemon-owned volume-local marker root through host tmpfiles
  and resolved it from the daemon's authoritative state root.
- Kept broker-owned StoreView current pointers observer-only at the final
  symlink boundary, so VolumeLocal does not attempt an unsafe metadata repair
  through a symlink while parent traversal remains fail-closed.
- Added non-sensitive stage diagnostics to U7 Volume source resolution so
  descriptor, identity, closure-root, and StoreSync mismatches remain
  distinguishable during host acceptance.
- Resolved controller-owned system Volumes from the broker-validated Guest
  StoreView farm after StoreSync, avoiding unsafe cross-mount traversal of the
  host Nix store while retaining a writable lock root and closure identity and
  generation fences.
- Added stage-specific U6 Cloud Hypervisor reconcile diagnostics for
  registration, setup Volume, endpoint publication, and post-controller
  passes.
- Deferred U6 endpoint publication while the controller-owned VMM Process is
  still converging by returning a retryable capability result, without
  weakening wrong-owner or wrong-provider checks.
- Preserved structured Resource API rejection details for U7 owned-child
  batches so StoreView child fencing failures remain diagnosable.
- Exposed only the typed kind, revision, and retry class for U7 child API
  rejections, keeping error reasons redacted while distinguishing conflicts
  from authorization and integrity failures.
- Included the typed Resource API rejection reason in U7 diagnostics; the
  reason is a non-secret schema/authorization classification, not payload data.
- Initialized the qualified virtiofs Export child status with its required
  typed readiness projection before the Resource API create.
- Bound virtiofs Export provider extensions to the qualified
  `virtiofs.d2bus.org.Export` ResourceType so Resource API schema admission and
  controller parsing use the same identity.
- Added non-secret Resource API envelope-validation diagnostics for malformed
  create payloads so typed child admission failures identify their contract
  class before redaction.
- Added matching U7 local envelope validation diagnostics to distinguish child
  payload construction drift from Resource API admission drift.
- Completed the universal Export child status update projection with its
  required `status.update.observedGeneration` fence.
- Redacted raw redb durability error details from stderr while retaining the
  typed integrity failure classification.
- Kept evaluated Guest closure metadata free of import-from-derivation reads;
  the realised artifact catalog now consumes closure files at build time.
- Updated the realised Provider catalog determinism check to read the
  authoritative catalog artifact during its build step.
- Classified the test-only Provider controller crate outside the production
  Provider matrix.
- Preserved finalized Guest controllers until their owned child resources
  finish draining, and counted reconciliation attempts before guard
  acquisition for contention coverage.
- Removed UID-bearing debug formatting from a redb test failure diagnostic.
- Requeued Cloud Hypervisor endpoint convergence as Pending until the
  controller-owned VMM Process is Ready instead of exhausting Guest handler
  retries.
- Preserved accepted effect persistence identities across newer queued
  operations and retried uncertain failure persistence without consuming
  another handler attempt.
- Isolated transient post-commit checkpoint and effect persistence pressure
  from unrelated controller identities, and restarted a dead U12 sibling
  even when another U12 runner remains live.
- Kept watch-resume backpressure on the watch retry path instead of mapping
  it to relist-triggering disconnect recovery.
- Kept provider executable and package digest projections eval-safe when their
  digest derivations are not realized; build-time artifacts remain authoritative
  for the realized values.
- Restarted all U12 siblings after a mixed-liveness failure, aborting the
  remaining live task before rebuilding the runner set.
- Recovered uncertain accepted-effect persistence through a persistence-only
  retry path so handler effects are not executed again with a reset attempt.
- Distinguished expected VMM Process convergence from invalid Cloud Hypervisor
  endpoint ownership and identity failures so only the former becomes Pending.
- Skipped stale failure projections after a generation change so the
  persistence-only recovery path releases the running key for fresh work,
  while preserving HandlerExhausted as the terminal reason.
- Treated failed or deleted VMM Process phases as capability failures instead
  of endlessly deferring endpoint publication.
- Continued Pending endpoint convergence through a Degraded VMM Process phase
  while keeping Failed and Deleted terminal.
- Opened the Provider credential-delivery stream before route metadata
  processing so supervised fd10 Providers cannot race the first key handoff
  under concurrent Rust test load.
- Relaxed only test-side eventual convergence waits for Resource API and redb
  worker tests; production retry and lifetime budgets remain unchanged.
- Serialized the Resource API test binary so per-test store/session fixtures
  cannot race under the aggregate Rust lane.
- Serialized the broker fake-backend test binary so shared fake host state
  cannot race with concurrent test cases.
- Gave the redb blocking-worker hold probe a test-only lifetime separate from
  the production read lifetime, preventing hosted-runner scheduling delay from
  bypassing the worker-hold assertion.
- Yielded before advancing the paused redb hold test and extended the
  shell-supervisor output wait so hosted scheduling cannot race test evidence.
- Added an explicit test-only worker-hold handshake so the paused redb test
  advances time only after the blocking worker owns its permit.
- Added a second test-only adapter-wait handshake so the paused redb test
  advances time only after the async adapter leaves its timed startup wait.
- Relaxed the Resource API large-relist test budget for hosted-runner
  scheduling while retaining a bounded fail-fast performance assertion.
- Serialized the supervised fd10 Provider test binary because its fixed
  inherited descriptors must not race other test cases in the same process.
- Serialized local Rust-main test execution on the hosted CI
  runner to prevent unrelated redb, Resource API, and runtime tests from
  starving one another.
- Limited total hosted Rust-main Bazel jobs to eight so local compilation
  actions cannot starve the serialized test process.
- Kept the Make dispatcher contract test aligned with the optional hosted
  Rust-main test-output diagnostic flag.
- Added a manual dispatch entry point to the fixed Layer-1 workflow so a
  reviewed head can recover required checks when a pull-request event is
  not delivered.
- Wire Core controller sources to production redb snapshots, watches, durable
  result commits, owner wakeups, finalizer handling, and effect lifecycle
  evidence.
- Bind adapter mutations through the single NativeAuthorizer seal path with
  explicit identity and assignment fences, preserve stable effect identity
  across resource revisions, and prove 10,000-resource relist under 100
  watches without duplicate authority rows.
- Compose the fixed Core ResourceType runners with the production redb
  controller source, authenticated NativeAuthorizer identity, and exact
  assignment fences.
- Admit the closed 27-Provider catalog together while preserving typed
  Provider and Guest custody boundaries.
- Attach Cloud Hypervisor, QEMU media, Azure Container Apps, and Azure VM
  Guest owners to filtered shared Runner reconciliation selected by the
  committed Guest provider reference.
- Preserve Guest child identity, dependency fencing, restart adoption, and
  finalizer-safe cleanup while removing the post-publication Cloud Hypervisor
  scheduler loop.
- Drive QEMU media, Azure Container Apps, and Azure VM Guest lifecycles through
  their typed controllers, advance one child mutation per pass, and drain
  deletion-requested children without issuing a second delete.
- Keep gateway-backed Guest runtime custody inside the configured Guest
  execution boundary with no Host fallback, and retain the typed Credential
  scope boundary for CredentialSession ownership.
- Requeue pending framework Guest progress on a bounded tick and exercise
  finalizer-only enrollment, controller readiness, child pacing, and
  non-Ready deletion through production shared Runner tests backed by a real
  Resource store.
- Cut storage Providers over to the shared Runner: `volume-local` is the sole
  Volume owner, `volume-virtiofs` owns qualified Export children, and anchored
  Volume effects preserve marker, lock, atomic-content, and restart-adoption
  invariants.
- Qualify the storage Provider finalizers for Resource API registration:
  `volume-local.d2bus.org/layout` and `volume-virtiofs.d2bus.org/export`.
- Cut Network and Device provider state through fresh, assignment-fenced
  reconciliation while preserving brokered network ownership, persistent TPM
  evidence, resource-backed USB and security-key bindings, and dependency-aware
  GPU worker upgrades.
- Bind the U8 registrations to typed Provider reconcilers, keep provider
  cleanup ahead of finalizer removal, and make shared Runner startup resolve
  every accepted Provider before spawning any runner.
- Complete production Provider wiring for Network child resources, device
  authority, dependency-aware GPU lifecycle, and typed cleanup before any
  shared Runner finalizer is removed.
- Make child readiness and cleanup durable across Resource API passes, with
  Core-issued GPU authority and identity fences instead of synthetic grants.
- Persist the four rendered Network config files through the owned Volume
  projection, with exact byte, provenance, owner, mode, assignment, and marker
  fencing.
- Route Network config through volume-local's typed materialization boundary and
  require its observed digest evidence before Network reports Ready.
- Parse the typed Volume content projection during normal volume-local
  reconciliation and retain fail-closed, restart-safe materialization behavior.
- Keep Core startup available before any U8 Provider rows exist while refusing
  partial U8 Provider enrollment.
- Route display, audio, and shell resource ownership through the shared
  asynchronous Runner while retaining clipboard and notification typed
  ComponentSession services and removing the legacy audio watch path. Start
  all U9 runners before Zone publication, isolate audio reconciliation per
  binding, preserve child-mutation evidence across repair and restart, and
  re-read exact AudioBinding dependencies during scheduled repair.
- Moved semantic audio, USBIP, security-key, and telemetry Binding realization
  onto deterministic Provider-owned child intents reconciled through Core.
  Child creation, repair, and teardown preserve explicit Binding ownership,
  target placement, optimistic identity fencing, and Endpoint-before-Process
  deletion ordering without creating consumer Bindings from Services.
- AudioBinding status now persists channel grants, levels and gains,
  arbitration, enforcement posture, and the last applied host/guest path
  alongside the observed Service and realization references.
- Move Wayland display proxy lifecycle under durable Host and Guest Process
  resources managed by `d2bd`, preserving signed execution references,
  restart adoption, ordered deletion, and typed clipboard and notification
  streams.
- Keep Host process Providers from treating Guest execution references as
  local launches until authenticated remote process routing is available.
- Persist display Endpoint children through the production Redb resource plane,
  including finalizer-gated deletion and restart reopening.
- Bind display mutation idempotency keys to their request payload or exact
  resource revision, and drain Process children before deleting Endpoints.
- Remove the obsolete display-specific broker routing and Guest systemd
  fallback; production display execution now uses the signed Process path.
- Reconcile the committed WaylandSession during trusted graphics VM start so
  the Host proxy Process is resource-backed and launched before GPU readiness;
  missing display sessions fail closed without a direct-spawn fallback.
- Keep QEMU-media Wayland proxies in the legacy VM-start DAG by distinguishing
  their existing process identity from trusted graphics resource nodes.
- ZoneLink routing and Provider forwarding now consume runtime-issued sealed route admissions instead of caller-populated authorization, connectivity, capability, and time claims.
- Production Zone composition now installs child-local Gateway Guest route state, while enrolled Relay connections carry only protected ComponentSession packets.
- Gateway-backed Zones are classified before lifecycle, exec, reconcile, shell, or Resource API dispatch; unsupported and refused paths fail closed instead of falling through to host-local effects.
- Allocator topology artifacts now require explicit compiler topology input, and host acceptance boots an isolated Gateway Guest canary credential, requiring the Guest runtime's non-secret open marker before proving exact secret non-materialization.
- Scope legacy host-side VM and environment projections to gateway-backed
  realm workloads while preserving Zone resource bundles and gateway artifacts.
- Bind the Wayland display proxy to neutral provider contracts while retaining
  authenticated target routing, readiness fencing, and display capabilities.
- Exposed operation, protocol-token, workload-provider, launcher, lifecycle,
  posture, and capability contracts from the foundational contracts root.
- Updated unsafe-local helper consumers to use those canonical contracts
  directly instead of compatibility aliases.
- Moved unsafe-local launcher and persistent-shell runtime identity to the
  Zone resource contract, fencing same-name resources with Zone and resource
  UIDs plus generation while preserving same-UID user scopes, bounded streams,
  redacted diagnostics, and exact teardown.
- Migrated unsafe-local helper runtime and shell consumers to the Zone resource
  identity contract instead of the d2b-core compatibility alias.
- Cleaned up dead test code: the uncalled `wait_timeout` process builder and the unused `set_mode` helper in the `d2bd` test fixtures, the never-read verb-set parameter threaded through the `d2b-bus` session admission test helpers, and the test-only magic role string in the `d2b doctor` pre-namespace posture probe, which now evaluates only real runner roles.
- Improved validation of concurrent Process launch dispatch so independent
  resources can start without waiting for earlier launch effects to finish.
- Consolidate process, runtime, transport, and shell ownership under neutral
  and provider crates.
- Route guest execution and persistent shell lifecycle through Process-family resource intents and authenticated ComponentSession named streams while retaining bounded authorization, retention, and disconnect handling; unavailable target-session routes fail closed without a legacy fallback.
- Demultiplex concurrent named-stream responses, cancel/reset on owner disconnect, and use one correlated JSON frame codec across CLI, ResourceClient, Providers, and daemon shell owners.
- Reject the retired public `exec`/guest shell bridge in production composition and record target-local supervisor Process ownership for ShellSession resources.
- Resolve enrolled target-local ComponentSessions for real Process and ShellSession routes, and persist shell supervisor Process ownership through the daemon Resource API with exact owner and finalizer fencing.
- Move non-network Guest Nix components and desktop projections into their
  owning Provider packages.
- Render Provider-owned Process, Endpoint, activation, and private projection
  inputs from Zone resources with disabled Providers emitting no fallback.
- Preserve typed Provider and ComponentSession boundaries across reconnects,
  including bounded repair declarations, operation identity fencing, typed
  stream and transport handles, and separate Guest seed and authenticated
  ResourceService surfaces.
- Retire the obsolete realm provider, router, transport, and protobuf codec
  crates while keeping live gateway effects owned by their provider crates.
- Cut Host, User, Process, and EphemeralProcess ownership over to the shared
  Core Runner with typed system Provider handlers, bounded priority/fairness,
  exact runtime identity fencing, and asynchronous effect acceptance.
- Preserve broker-owned pidfd and cgroup adoption while making replacement,
  finalization, terminal exit, and EphemeralProcess TTL handling exact.
- `make test-host-integration` now builds the fixed eight host tools with local
  Bazel, injects them into the selected NixOS VM checks, and uploads successful
  dependency closures to a configured Attic cache without caching the VM test
  results themselves. When Attic or its configuration is unavailable, the lane
  explicitly skips the upload; invalid or unusable configured state fails the
  lane.
- Split resource, provider, credential, semantic-service, and Zone/session
  contracts into narrow `d2b-contracts-*` crates while preserving wire and
  schema behavior. Resource protocol messages and generated drift ownership
  now live with `d2b-contracts-resource`; `d2b-resource-api` retains the
  generated ttrpc adapters. Broker, provider, and Zone/session crates import
  resource-owned types from `d2b-contracts-resource` directly instead of
  compatibility umbrellas.
- Moved remaining daemon, runtime, Provider, CLI-generation, and copied Guest
  consumers onto shared Zone-neutral contracts and the Azure Relay transport
  owner while preserving lifecycle, credential, and lock behavior.
- Converged the shared Cargo, copied Guest, Bazel, and policy graphs with the
  current Zone and controller-owned Guest lifecycle.
- Regenerated locked dependency projections, policy closures, schemas, CLI
  artifacts, and current consumer examples from the active sources.
- Route signed controller and VMM Process adoption through exact stale identity
  replacement and broker-owned launch, stop, and reap lifecycle.
- Complete U9 ownership relocation for network, Volume, store-view, storage
  contract, and activation planning/Nix surfaces while retaining broker effects.
- Added exact owner-child Resource API scopes, owner UID-filtered queries, and
  same-batch incarnation fencing for controller-owned Process resources.
- Moved product Cargo authority to the repository root and added checked-in,
  context-scoped production closure approvals with independent recomputation.
- Added a standalone `make test-cargo-compat` proof for Cargo test, nextest,
  broker feature, guest feature, doctest, harness-free, bench, and fixture
  exclusion behavior.
- Made Nix author only Cloud Hypervisor Guest and Provider inputs, moved setup
  details into private artifact descriptors, and removed Nix-owned VMM Process
  projections.
- Added rustdoc compile-fail examples documenting downstream fabrication
  barriers for authenticated capability types.
- Retained the narrow external capability seal whose downstream
  `cfg(test)` trust-boundary property cannot be proved in the defining crate;
  public API and capability boundaries continue to use compiler assertions,
  compile-fail examples, and public contract tests.
- Removed unused Rust dependencies and their lockfile edges, reducing
  unnecessary dependency resolution and compilation.
- Consolidated pinned-test inventory discovery into one main-workspace Cargo
  listing, avoiding a duplicate contract-crate resolution pass.
- Replaced the slow bus capability mutation and sealed-authority fixtures with
  explicit rustdoc `compile_fail` coverage for every prohibited public trait
  and the private authority trait, while retaining the resource API's narrow
  forced-`cfg(test)` probe.
- Added an absolute 60-second per-test wall-clock ceiling to the runtime
  ledger; shorter timing thresholds remain advisory while aggregate crate CPU
  budgets remain the regression gate.
- Combined ADR-046 measurement policy checks so the documentation corpus is
  loaded once instead of once per test.
- Cached workspace Rust source contents during the tracing-contract scan,
  avoiding repeated reads for each forbidden-attribute pattern.
- Reused the enumerated and loaded source set across the CLI consumer policy's
  multiple pattern scans.
- Removed unused direct `ttrpc` and `serde` dependencies from the bus and
  relay-bridge crates, respectively.
- Documented the Rust crate-graph audit and retained the current workspace
  boundaries where change-frequency data did not support a split.
- Move Rust doctest and sealed API coverage into owning Bazel crate targets.
- Remove nested Cargo test compatibility and replace runtime tool builds with declared Bazel artifacts.
- Delete migration, successor-pin, and timing evidence shell scripts instead of translating them.
- Drop the wave-evidence hardware smoke script; physical-device validation is manual.
- The pinned Rust toolchain moves to 1.97.0.
- Rust tests execute under `cargo-nextest`. Doctests and `harness = false`
  binaries are not a nextest surface, so each workspace additionally runs
  `cargo test --doc` and a `cargo test --test` pass over any harness-free
  target, discovered from the test listing rather than pinned. The privileged
  broker workspace stays on `cargo test`, because its tests depend on the
  harness environment in a way that process-per-test execution does not
  preserve.
- `[build] rustc-wrapper` in every cargo workspace now points at a shim that
  uses sccache when it is available and plain rustc when it is not. Naming the
  binary directly made sccache mandatory for every cargo invocation, so
  environments without it had to clear `RUSTC_WRAPPER`, and that override
  spread into environments that did have sccache and silently disabled the
  compiler cache.
- Bound managed-identity Credential leases to authenticated subject, Zone,
  workload, Provider session, and bounded expiry state; restart checkpoints
  remain secret-free and finalization revokes only the owning workload's
  handles. Reacquisition replaces terminal or stale-session records without
  inflating the accepted client rotation generation, and repeated checkpoint
  restore remains occupancy-safe and idempotent.
- Restricted service admission to local authenticated transports and matching
  consumer generations, revoked superseded active or session-expired handles
  before reacquisition, ignored cleanup-only records when measuring live
  occupancy, and kept terminal lease metadata observable without reopening
  closed leases.
- Compensated invalid lease grants by revoking the newly issued handle before
  returning the invariant error, and returned terminal states discovered by a
  live metadata inspection as observations.
- Strengthen production EffectPort isolation and cloud composition evidence with deterministic Layer-1 coverage.
- Close exact recipient routing, per-recipient enrolled Noise isolation, opaque
  transport and attachment rejection, sensitive KK admission, and Credential
  delivery-binding coverage across the three production Credential Providers.
- Enforce Provider EffectPort, specification vocabulary, test-taxonomy, deterministic-time, and current threat-matrix policies.
- Extend interaction canary coverage across clipboard, terminal, notifications, and security-key CTAP surfaces.
- Add zero-secret Credential canaries for merged Entra, managed identity, and Secret Service configuration, placement, session, lease, ambiguous-completion, recovery, audit, telemetry, Debug, and support surfaces.
- Restore signed Provider controller Process projection and private template
  bindings for daemon and broker launch resolution.
- `make test-fixture-contracts` no longer reruns fixture-independent policy
  binaries already enforced by `make test-policy`, avoiding duplicated
  repository-wide source and documentation scans.
- Reduce gateway Nix-unit evaluation memory by keeping assertion-only fixtures
  free of unrelated environment and net-VM graphs.
- Reduce Nix-unit evaluation memory by sharing focused realm, workload,
  resource, gateway, niri, and observability fixtures while retaining full
  integration coverage.
- Share the configured local-VM workload predicate between limit validation
  and private workload emission.
- Derive first-party Bazel Rust sources from Cargo-conventional globs and
  third-party dependencies from the root `rules_rs` lockfile helpers, with
  explicit feature, doctest, and harness-free companion targets.
- Use `D2B_RUST_BUDGET` as the supported local Rust budget control. Top-level
  Make `-j` does not cap inner Cargo concurrency; the Rust target derives
  Cargo and nextest quotas from the effective CPU and memory budget.
- Run the complete Nix-unit corpus through one aggregate
  `nix-eval-jobs --no-instantiate` attr per current case file (45 file jobs),
  with focused toolchain self-provisioning, bounded worker control, and
  complete multi-failure reporting. Reuse the same aggregate constructor for
  the seven topical flake checks, and expose one locked inventory containing
  sorted full case names and file-job names.
- Use the operator-intent `D2B_NIX_UNIT_WORKERS` and
  `D2B_NIX_UNIT_MEMORY_MB` controls for Nix-unit resource requests, and retire
  `D2B_NIX_UNIT_JOBS` with an actionable migration error.
- Keep successful full runs concise while retaining one sanitized stderr
  attribution per real `FAIL <case>: <detail>` line, with one fallback for an
  aggregate that emits no such line. Report exact evaluated-vs-pinned case-name
  drift with the `run make nix-unit-pin` remedy; use a fixed path-free `d2b`
  flake label for command progress.
- Record Nix-unit execution evidence as the seven stable baseline leaves while
  keeping evaluation-only runs free of installables and realized checks; use
  one aggregate eval-jobs attribute per case file plus shard/pin integrity.
- Reject the seven-aggregate candidate after its 543s local four-worker
  observation and the per-case candidate after hosted memory exhaustion.
- Keep four requested local workers with a 4096 MiB default and retain exact
  case and file-job inventory checks. Keep hosted CI on the pre-change
  discovery and per-check matrix because the full eval-jobs runner did not fit
  the hosted runner envelope.
- Keep the separate enforcing fixture lane from duplicating the aggregate by
  honoring `D2B_SKIP_FIXTURE_BUILD=1` in the Layer-1 orchestration.
- Keep the measured parallel profile for warm local runs, retain its API cache
  while using a bounded prebuild plus fixture/inventory/schema chain for cold
  runs, and run each Rust leaf as a separate full-budget CI job behind the
  stable rollup.
- Reuse normalized documentation text across ADR 0046 spike-measurement policy
  checks instead of rebuilding it for every site, inventory pattern, and
  negative control.
- Run the fixture-independent Rust policy binaries through one Cargo invocation
  while retaining fail-closed evidence that every selected binary executed
  nonzero tests.
- Converted the `activation-nixos.d2bus.org.NixosGeneration` reconciler to
  the v3 `ResourceDriver`/`ResourceDriverFactory` contract: the pure
  activation policy still plans the runner, a Host target dispatches the
  preserved `ApplyHostGenerationHandoff` broker effect through a driver
  effect port, and a Guest target mints the activation-runner
  `EphemeralProcess` as an owned child through the manager so the Process
  controller owns the launch (no controller-side spawn).
- Moved the activation spec decode onto the manager-wired decoder, so the
  closed generation contract (Provider reference, Host/Guest execution
  target, artifact identifier, prior-generation type) is the validation
  fence, and retried failures keep the preserved retryable classification.
- Converted `EphemeralProcess` onto the Process driver: `ProcessDriverFactory`
  now serves both Process-family types (`Process` and `EphemeralProcess`), the
  row's own type name selects the typed spec decode, and the one-shot arm runs
  the preserved ephemeral provider effects (`launch_ephemeral_resource`,
  `adopt_ephemeral_resource`, `probe_ephemeral_resource`,
  `stop_ephemeral_resource`) through the same `ProcessDriverEffects` port. A
  refused one-shot launch is terminal (the type carries no restart policy); an
  `Exited` probe reports `Succeeded`; an over-running process past
  `runtimeDeadline` stops through the fixed 30s/30s escalation and reports
  `Failed`; and `successfulTtl`/`failedTtl` retention is a runtime-only clock
  (no durable `completedAt`/`cleanupEligibleAt` write, R11) that asks the
  manager to retire the row when it elapses, with `incidentHold` keeping a
  failed row until an explicit release.
- Deleted the last production `ResourceReconciler` implementor:
  `ProcessResourceReconciler`, `ProcessResourceRuntime`,
  `process_controller_descriptor`, the guest-local typed Runner
  (`run_guest_process_reconciliation`, `GuestProcessSource`) and the
  `serve_guest` composition that spawned it. `process_resource_runtime.rs` now
  keeps only the canonical launch-identity resolver (`resolve_launch_identity`,
  `guest_runtime_process_matches`), the generic Process list the
  controller-session fences read, and `PROCESS_RESTART_ANNOTATION`.
- Wired the `EphemeralProcess` type into the new plane:
  `V3_CONVERTED_RESOURCE_TYPES` 32 -> 33, the plane's decoder map, and the
  factory's registration; the partition and child-mutation-route tests now
  expect the type on the manager plane.
- The activation-runner mint (KTD13) is unchanged: the NixosGeneration driver
  still mints the runner as an owned `EphemeralProcess` child carrying the
  typed `activationInput`, and its settle watch now sees a manager-served row
  whose one-shot lifecycle reports `Succeeded` on exit and retires through the
  manager.
- Retired the liveness-waiter machinery (`spawn_resource_waiter`, the waiter
  poll loop, `RESOURCE_WAITER_POLL`) whose only caller was the deleted runner;
  the one-shot arm observes through the preserved probe on its own 5s resync
  requeue. The Guest-local credential backend responder composition is
  retained (marked dead-code) for the Guest-side realization follow-on.
- Retired the legacy host Process/EphemeralProcess runner: the old-plane
  `ProcessResourceReconciler` startup, its task/generation/failure fields,
  its readiness and shutdown plumbing, and the redb-backed process identity
  caches/loaders it seeded are gone. Converted `Process` rows are served
  exclusively by the new plane's `ProcessDriver`; the host in-daemon writers
  and unconverted `EphemeralProcess` rows keep their old-plane semantics.
- Split the old runner entry point: `reconcile_process_resources` becomes
  `reconcile_controller_sessions`, keeping the external-controller wake
  registration, establishment loop, and resource fences on the old plane
  (controller sessions stay there until the core-controller conversion), with
  the composition startup call retargeted and the display-host-proxy /
  cloud-hypervisor lifecycle launch nudges removed.
- Bound the catalog-bound `Guest` setup descriptor digest into the production
  Process driver effects for guest-owned rows. The private guest VMM intent
  lookup refuses a ticket without it
  (`provider-ticket:guest-descriptor-unbound`), so a controller-minted
  `Process/<guest>-vmm` row could not launch end to end; the digest is the
  same value the old runner snapshotted from the bundle's loaded Guest setup
  descriptors.
- Deleted the host-only machinery in `process_resource_runtime.rs`: the
  `ProcessResourceRuntime::new` host constructor, the redb identity-cache
  seeding and loader setters, the target-scope/lifecycle/descriptor setters
  only the host runner used, and `controller_provider_refs`.
- Converted the U8 shared host-provider family to per-kind v3 drivers: the
  `Network` Provider and the `Device` Providers (tpm, usbip, security-key,
  gpu) now run through `SharedProviderDriverFactory` on the resource plane
  instead of shared Runner rows. Each driver validates the spec against the
  owning Provider row, ensures the kind's desired children through the
  manager child API (commit before the child actor exists), retires owned
  children the desired set no longer names in the family's preserved
  endpoint-first/process-last order, runs the typed Provider effect behind
  the `SharedProviderDriverEffects` port, publishes in-memory status (R11)
  with a resync requeue while not converged, and deletes through the
  family's preserved teardown ordering. The six family ResourceTypes
  (`Network`, `Device`, `usb.d2bus.org.UsbService`,
  `usb.d2bus.org.UsbBinding`, `security-key.d2bus.org.SecurityKeyService`,
  `security-key.d2bus.org.SecurityKeyBinding`) are part of
  `V3_CONVERTED_RESOURCE_TYPES` (8 -> 14) and route to the new plane.
- Converted the telemetry semantic-binding family
  (`telemetry.d2bus.org.TelemetryService` / `telemetry.d2bus.org.TelemetryBinding`)
  to the v3 `ResourceDriver`/`ResourceDriverFactory` contract:
  `TelemetryDriverFactory` validates the Provider-declared spec envelope,
  adopts the manager-owned child set on recover, ensures provider-declared
  children through the manager (commit before spawn), retires obsolete owned
  children Endpoint-first and Process-last, publishes its status in memory,
  and requeues itself while not converged (the preserved 5s resync). The
  collector and forwarder processes are Process resources the Process
  controller launches.
- Removed the telemetry `d2b.d2bus.org/binding-children` finalizer
  machinery and the telemetry-only status-redaction helpers: the v3 manager
  already cascades owned children and holds the parent row until the last
  child retires, and nothing persists a telemetry status, so the finalizer
  and the store-side sanitizers had no work left to do.
- Deleted the old-plane U12 observability controller runner (provider table
  row, runner tasks/locks, readiness gate) now that both of its families,
  activation and telemetry, are served by the v3 plane.
- Retired the residual legacy U7 volume-leg runtime: the old-plane
  `VolumeBinding` shared runner
  (`SharedVolumeResourceReconciler` over `DaemonVolumeProviderEffects`, and
  its `volume-local` sibling registration) is gone, so the v3 plane's
  `VolumeDriver`/`BindingDriver` own the whole storage family.
- Folded the surviving legacy binding behavior into `BindingDriver`:
  `reconcile` derives the `VirtiofsdWorkerPlan` and carries it in the
  in-memory status as the serving authority (the plan never travels in a
  resource, KTD1; the worker Process child stays argv-free and the Process
  controller composes its launch, KTD13/U17), ensures the worker/endpoint
  children commit-before-spawn, retires owned children the derived set no
  longer names endpoint-first/process-last, registers the Volume and child
  dependency watches (R12/R17), and requeues while the child set is not yet
  current; `recover` adopts only when both child rows are current and the
  serving socket is listening; `delete` preserves the KTD6 drain gate (a
  guest mount observed before anything is deleted blocks the teardown, and
  the durable deleting mark plus the owned children stay for a retry) and
  the endpoint-first / process-last teardown, with the manager holding the
  parent row until the last child retires (F3) exactly as the old finalizer
  did. Terminal plan/view rejections keep their stable provider reason in
  the in-memory status (KTD5).
- Deleted `packages/d2bd/src/resource_runtime/volume_provider_runtime.rs`
  (the U7 kind/effect/reconciler/runner machinery), the `start_u7` /
  `stop_u7` runner lifecycle and its task/lock/readiness plumbing in
  `resource_runtime.rs`, and the `composition.rs` startup call. The
  `volume_effect_adapter.rs` module stays: the v3 plane's production volume
  effects build on it.
- No converted type registers Guest-side target-local effect code yet, so the
  service admits no type in production today: realize frames for types without
  effect code are refused rather than recorded. Process and EphemeralProcess
  effects keep running through the preserved `run_guest_process_reconciliation`
  path, which this service does not replace. Registering a type's effect code
  is what admits it.
- Renamed the generic Guest target-control session purpose away from the
  literal `zone-link` to `component-session`, in the session identity, the
  enrolled endpoint policy, the generation-discovery profile, the ZoneLink
  route admission's Gateway-Guest carriage profile, the guest daemon's
  `--purpose` default, and the `d2b.componentSession.purpose` module option
  (R20). ZoneLink resource semantics are untouched: ZoneLink traffic keeps its
  own purpose, roles and transport profiles, and a ZoneLink that runs in a
  guest consumes the generic target path.
  **Compatibility:** the purpose is a cross-boundary value (host-published
  `component-session/guest.json` descriptor, guest daemon `--purpose`, and the
  Noise endpoint policy on the wire), so host daemon, guest daemon, descriptor
  and Nix module must move together; a pre-rename descriptor or an old-purpose
  peer fails closed (guest-mode-purpose-mismatch / handshake purpose-mismatch)
  rather than downgrading. No wire enum value was added or renumbered.
- Centralized contributor guidance in a concise operational authority, added
  product strategy documentation, and defined the tiered skill workflow through
  reviewed pull-request merge.
- Narrowed the non-ASCII dash lint exemption to approved agent instruction and
  skill payload paths while keeping the repository-wide rule fail-closed.
- Cut ordinary Guest packaging over to the mode-bound `d2bd guest` and
  `d2b-broker guest` ComponentSession shape, including shared static artifact
  and Guest workspace coverage.
- Keep gateway parent-Guest and child-Zone daemon and broker instances on
  separate mode-bound sockets and state roots.
- The Venus Vulkan Video lab prototype no longer asserts a graphics capability
  its guest does not have. It previously set
  `gfx.blacklist.hardwarevideodecoding` so Firefox would skip a VA-API probe the
  guest could not pass; the guest now passes that probe on the merits and the
  preference is removed. Decode remains Vulkan Video through Venus, with VA-API
  answering only the capability question. Firefox is still unmodified.
- The lab's virglrenderer fork can initialise its video backend and accept the
  host's non-Mesa VA driver, each behind its own explicit opt-in that is off by
  default. Without them the fork behaves exactly as upstream does.
- Controller-owned Guest lifecycle now resolves Gateway Guests and guest-control Endpoints from committed Zone resources and establishes sessions by immutable Guest identity.
- Guest lifecycle and activation paths preserve exact Zone, Guest, Endpoint, Provider, generation, and revision fences across reconnect and restart.
- Zone resource bundles and Provider artifacts are now the only active Nix
  lifecycle inputs; root daemon/broker admission, required OS groups, Network
  gateway fields, host-tool plumbing, and v2 launcher metadata remain.
- Rehomed current CLI failure, output, framing, transport, authentication,
  audit, host diagnostics, and Guest config-staging helpers under their
  Zone-neutral CLI owners while preserving existing envelopes and atomicity.
- Move clipboard Provider runtime identity onto the shared Zone-neutral bridge contract without changing clipboard behavior.

### Removed

- Removed the unreleased host cutover feature, including its CLI, daemon and
  broker protocols, runner, Nix packaging, schemas, tests, fixtures, and
  current operator documentation.
- Removed legacy Guest system lookup aliases in favor of
  `d2b.guestSystems.<zone>.<guest>`.
- Removed stale `d2b-realm-core` Cargo and Bazel edges from the broker and
  unsafe-local helper lock graphs.
- Deleted the Core reconciler surface from `d2b-core-controller`:
  `CoreResourceReconciler` (definition, `ResourceReconciler` impl and
  tests), `core_controller_descriptors` /
  `CoreControllerDescriptorError`, `CoreResourceControllerRegistration`,
  `CORE_RESOURCE_CONTROLLER_REGISTRATIONS`, and
  `CORE_PROVIDER_API_BINDING_FINALIZER`; the crate's re-exports are trimmed
  to the surviving contract pieces (`fixed_system_core_handlers_ready`
  stays).
- Deleted `packages/d2bd/src/resource_runtime/guest_provider_runtime.rs`
  and `packages/d2bd/src/resource_runtime/shared_provider_runtime.rs`, and
  removed the U6 shared-Runner wave from `resource_runtime.rs` and
  `composition.rs` (registrations, runner tasks, readiness gate, shutdown
  drain, rebind hooks).
- Trimmed `binding_child_resource_runtime.rs` to its two live readers
  (`binding_readiness_current`, `parsed_binding_spec`); the guest-child
  reconciler machinery is deleted with the family.
- Deleted `packages/d2bd/src/resource_runtime/interaction_provider_runtime.rs`
  and the U9 shared-provider arms (six kind variants, dispatch, effect and
  child helpers) from `resource_runtime/shared_provider_runtime.rs`, plus
  the dead U9-era child machinery in `binding_child_resource_runtime.rs`
  and the audio runtime's child-ownership surface.
- Deleted the typed Host/User handler and its runner: the
  `SystemCoreResourceReconciler` + `ResourceReconciler` impl, the
  `SystemCoreHostProbe` / `SystemCoreUserDiscovery` port implementations
  (moved into the driver), the system-core descriptor/fence/API/source
  block, and the Host/User runner spawn and transient-retry loop from
  `resource_runtime.rs`.
- The persistent resource store is deleted, not deprecated: the
  `d2b-resource-store` and `d2b-resource-store-redb` crates (roughly 28k
  lines: actor, transaction, revision log, backup, ownership, value/key
  codecs, schema, audit, metrics, tracing), the redb-backed
  `RedbRegisteredControllerApi` (`packages/d2b-resource-api/src/registered.rs`)
  and the legacy watch half of `d2b-resource-api/src/watch.rs`, and the #507
  wrong-plane fence together with the plane it fenced (`LegacyPlaneFence` and
  every refusal helper in `d2b-resource-api/src/store.rs`). The manager is now
  the only execution model, so a wrong-plane read is no longer possible and the
  fence has nothing left to refuse. The store DTOs that outlived the crates -
  `StoreSlot`, `AdmittedAuthorization`, `PolicySnapshot`, `StoreOperationContext`,
  the mutation-seal and store-error types - moved to
  `packages/d2b-contracts-resource/src/v3/operations/{mod,error,seal}.rs` and are
  still re-exported at `d2b_contracts_resource::v3::*`, so a consumer of the
  types needs no change.
- The control-model machinery that existed only to route controllers through
  that store is deleted: `d2b-controller-toolkit`'s `Runner`,
  `ControllerSource`, `PendingQueue` and its queue machinery, the
  `ReconcileResult`/`MutationIntent` protocol, the reconcile-pass context half,
  and the crate's benches and integration tests (deleted rather than ported -
  nothing surviving drove them); and `d2b-core-controller`'s store-routing
  module class (`runtime.rs` with `CoreControllerSource`, the
  `configuration/{mod,bundle_apply,generation_transition}` bundle/cleanup/generation
  code, `cleanup.rs`, `audit.rs`, `authz*.rs`, `watches.rs`, `store.rs`,
  `resource_store.rs`, `export_import*.rs`, `metrics.rs`, `tracing.rs`,
  `provider_effects.rs`, `dependencies.rs`, `hints.rs`, `optional_state_admission.rs`,
  `ownership.rs`, `budgets.rs`, `quota.rs`, `emergency_policy.rs`,
  `user_session_authority.rs`) with their tests. `d2b-core-controller` keeps the
  domain modules the converted drivers and the daemon import (authority,
  controller assignment, controllers, coordinator, owner reconciliation,
  providers, rbac, zone links/status), and `d2b-provider-test-controller` - the
  signed acceptance fixture the activation lane drives - is kept: it needs only
  the two assignment constants, not the store routing.
- `d2b-bus` loses its store-era fixtures: `src/production_rss.rs`, the
  `production-rss-fixture` feature and its `[[test]]` entry, and
  `tests/production_watch_rss.rs` with the store-backed watch test and helpers
  in `src/router.rs`. The ResourceV3 watch contracts are exercised through the
  manager-served paths that survive.
- The daemon's durable status, authority and controller-checkpoint helpers are
  gone: `d2bd-runtime`'s store reads/provisioning (zone row readers, bundle
  materialization and validation, status persistence, controller-session
  evidence persistence, store identity resolution), the `Durable` arm of the
  guest store backend, and the whole `RedbAuthorityPersistence` adapter. The
  broker's zone-store file handover is not built anymore, so the daemon no
  longer opens a database file at startup.
- Every workspace manifest, `BUILD.bazel`, `Cargo.lock`, `Cargo.guest.lock` and
  the generated package-policy inputs drop the deleted crates and their
  dependency edges; the register of packages under `packages/policy-inputs`
  shrank from 357 to 354 packages (the two store crates and `redb`).
- The dead per-env usbipd spawner (`run_usbipd_perenv_autostart` and
  `BrokerPerEnvUsbipdSpawner`) is deleted from the daemon: it had no
  callers, and per-env usbipd runners are attach-owned rather than
  startup-owned. The `d2bd-runtime` spec module it used stays with its own
  tests.
- The legacy VM-start launch machinery freed by the fallthrough removal:
  `VmRunnerLaunch::Legacy`, `register_node_pidfd`, the surviving
  `cleanup_vm_start_registration` helper, and the `VmStartRunner` fields
  (`lifecycle_authorization`, `workload_identity`, `network_tap_context`)
  that only the raw `SpawnRunner` request read.
- The TPM direct spawn path: `LiveTpmEffectExecutor` (its
  `BrokerRequest::SpawnRunner` request, the pidfd reservation and
  registration, the runner-snapshot adoption classification, the launch
  tickets) and the `CoreTpmEffectExecutor`/`TpmEffectPort` implementation
  that existed to drive it, plus the now-orphaned
  `stop_unregistered_spawned_runner` / `signal_unregistered_spawned_runner` /
  `wait_unregistered_spawned_runner_reaped` helpers and their broker-double
  test. `write_runner_snapshot[_with_authorization]` survives as test-double
  seeding only.
- The GPU direct spawn path: `spawn_worker`, its `SpawnRunner` request with
  `inherited_fd_count`, `open_device_classes`, the intent lookup through
  `find_runner_intent_for_process_in_vm`, and the `gpu_processes` /
  `gpu_opened_devices` per-resource state maps with their take/retain
  helpers.
- Removed the unused `d2b-daemon-access` bootstrap crate from the provider workspace.
- Removed the retired host provider adapter crate and runtime-provider bridge
  from the host package, eliminating its obsolete realm-provider dependencies.
- Removed the unused `d2b-userd` guest service stub and its static packaging surface.
- Retired VM, realm, and legacy dispatcher routes no longer participate in
  command execution.
- Delete the central contract-test crate, migration ledgers, successor pins,
  runtime test census, legacy static runners, and repository-wide policy tests
  outside the retained source-hygiene, workspace/lock, supply-chain, and
  changelog classes.
- Removed leftover unused published Nix options `d2b.site.flakePath`, `d2b.realms.<realm>.policy.defaultDeny`, and `ch.exporter.includeTopologyLabels`. Consumer README and realm-options docs now match.
- Removed the historical v0.1.0 host migration script and write-up. Use the current v0-to-v1 and v1-to-v1.1 guides.
- Removed empty flake `apps` and `overlays.default` outputs.
- Removed docs-only `examples/personal-dev` and `examples/work-entra` alias directories. Use `examples/minimal` and `examples/with-entra-id`.
- Removed unused daemon leftover modules: realm access resolver, audit-check, realm stubs, StopDagOwner, and unused virtiofsd/wayland watchdog types. The stop-dag deliverable gate now pins the deletion test in `policy_daemon`.
- Removed unused host leftover helpers: runner-shape preflight, empty `fake` placeholder, and unused `async-trait`. The process-marker pin now retires `packages/d2b-host/src/runner_shape.rs`. The leftover runner-shape preflight test pin is gone.
- Removed the compile-only `d2b-wlproxy-spike` crate, undeclared CLI `human_render.rs`, unused `ProcessNodeBuilder`, unused workspace `rtnetlink`, and the duplicate usbip network-scoping contract test. The runtime-ledger census no longer pins the deleted ProcessNodeBuilder tests.
- Removed the unimplemented `RelayProvider` trait; relay implementations use
  the live `TransportProvider` surface.
- Stopped tracking Beads runtime under `.beads/` in the product tree.
  Gas City issue state lives in the standalone
  [d2b-gascity](https://github.com/vicondoa/d2b-gascity) city and a
  dedicated rig clone. Local `.beads/` directories stay gitignored.
- Remove the duplicate legacy VM restart acceptance fixture.
- Retire the obsolete realm-owned unsafe-local host fixture; current host
  admission is covered by the Zone resource and package-local tests.
- Removed the unused generic credential service crate; credential protocol
  ownership remains with the neutral contract and provider-local surfaces.
- Removed duplicate Cargo/Bazel authority, dynamic Layer-1 shell schedulers,
  transfer reports, and obsolete provider qualification artifacts.
- Remove the retired Layer-1 workflow generator and coverage shim, direct
  Cargo compatibility instructions, and stale package test commands.
- Removed assignment epochs from the fence contract in the same change that
  moved succession to generations/revisions (no external consumers).
- Removed the `virtiofs.d2bus.org.Export` durable attachment contract in the
  same clean break (KTD10), superseded by `VolumeBinding`: its schema, finalizer,
  status projection, child-mutation path, watches, and host assertions are gone;
  `volume-virtiofs` no longer mints bindings and owns only its virtiofsd worker
  process and private endpoint effects.
- Removed the semantic Binding watch, whole-resource activation and Wayland
  cleanup loops, legacy HostJson Provider composition, dead scheduler flags,
  and placeholder Provider scaffold tests.
- Closed the Guest workspace mirror over the daemon's current Provider
  dependencies and synchronized its manifest and lock with the activation
  Provider's cryptographic dependency.
- Retained support-only config-nixos and typed ComponentSession
  stream/transport services outside the retired daemon reconciler paths.
- Keep controller-effect ledger rows in their Resource API owner instead of
  treating them as Host-global authority claims during restart recovery.
- Removed Cloud Hypervisor Guest's direct Volume Ready projection; StoreSync
  remains an effect while `volume-local` stays the sole Volume owner.
- Stop emitting non-gateway entries from the manifest, host, process,
  closure, minijail, and activation projections.
- Remove VM and realm-target compatibility inputs from the display proxy,
  including legacy clipboard bridge metadata and retired owner dependencies.
- Removed the no-op `d2b-provider-device-gpu` `provider_lifecycle` integration test target. It declared scenarios for a Core adapter and Host/Guest harness that do not exist yet and contained no tests; the hermetic tests in `packages/d2b-provider-device-gpu/tests/` already cover the corresponding behavior.
- Removed the legacy Process snapshot/watch scheduler and direct one-shot
  completion wait from the Provider-owned lifecycle path.
- The host-integration-only sccache switch is retired. The generic
  `d2b.site.hostSccache.enable` option remains available for other Nix source
  builds.
- Removed retired Core realm controller and workload-launcher compatibility
  owners and their package-local test/build exports; active Zone, Guest,
  Process, resource, auth, audit, and support surfaces remain unchanged.
- Removed the retired Gateway crates, Gateway display wire and compatibility
  owner, legacy ACA Gateway adapter, and Gateway enrollment/display/prologue
  helpers.
  Guest-local ZoneLink credentials and Azure Relay transport remain owned by
  the current transport Provider and Guest serving path.
- Removed retired gateway and realm-core edges from the shared workspace,
  fixture, aggregate Bazel, and policy surfaces.
- Removed stale Realm and VM-first CLI schema artifacts from the current
  generated contract set.
- Removed unreachable legacy CLI routing and realm-entrypoint implementations;
  current Zone-native command and route surfaces remain authoritative.
- Removed the inert broker realm metadata path arguments and defaults while
  retaining the corresponding bundle metadata for its current consumers.
- Removed the in-repository Gas City packages, modules, fixtures, host
  integration, smoke check, flake outputs, Bazel registrations, retired ADRs,
  and contributor environment plan.
- Contributor orchestration now lives in the standalone
  [d2b-gascity](https://github.com/vicondoa/d2b-gascity) repository, while
  NixOS host distribution and installation live in
  [gascity.nix](https://github.com/vicondoa/gascity.nix). Consumers must first
  cut over to those projects, then run the standalone smoke checks and capture
  rollback evidence, before adopting the d2b revision that removes the old
  exports. Rollback means pinning the prior d2b revision; d2b does not claim
  that external proof was run locally.
- d2b does not migrate, delete, chmod, chown, or sweep existing
  `/var/lib/gascity*` or `/run/gascity*` state.
- Removed repository-owned agent and skill surfaces, panel workflow records,
  Spec Kit support files, and ADR validation tooling and documentation.
- Removed the retired contributor-only API inventory gate and its generated snapshots.
- Retired the mandatory PR checklist validator; focused evidence is now
  accepted, with broader lanes selected conditionally.
- Removed the unused v1 realm workload launcher artifact and its Nix and
  fixture registration; launcher clients now use the provider-neutral v2
  metadata.
- Moved retained host-tool package option declarations out of the gateway
  compatibility tombstone without changing host-tool resolution.
- Deleted the old U8 shared Runner machinery:
  `packages/d2bd/src/resource_runtime/shared_provider_runtime.rs` shrinks
  from 8118 to 4218 lines (the U8 runner table, kind/effect/reconciler
  types, and their previews are gone), and the U8 provider-runner
  catalogue, runner preparation, capacity term, and readiness wiring are
  removed from `resource_runtime.rs`, `composition.rs`, and the
  U8-only test pins in `tests/core_composition.rs`.
- Removed production reachability for the retired guest-control exec and
  config wires, direct exec owner, and guest-control shell and activation
  adapters. Process resources, ConfigNixos, and ComponentSession named
  streams now own those paths.
- Removed the standalone guest daemon, legacy guest protocol bindings,
  token-share broker operation, obsolete SSH readiness role, and retired
  package-policy inputs. Old peers fail closed at the ComponentSession
  boundary.
- Removed the Wayland proxy's compatibility host-terminal child launcher;
  desktop terminal processes remain owned by their signed Process or
  companion.
- Removed the pre-Zone Nix env, realm, VM, and gateway hierarchy, including
  legacy lifecycle emitters and per-realm service, socket, user, and state
  declarations.

### Fixed

- Unified ComponentSession and Zone bus operation names on typed canonical
  `Service/Member` spelling and made bus registration consume the admitted
  session capability instead of accepting cloneable claims.
- Bounded and fairly scoped bus operations, routes, streams, credits, session
  requests, reassembly, and event queues; pinned revocable destinations across
  reconnects; and made dispatch deadlines, dropped futures, stream waiters, and
  cancellation cleanup actively release capacity.
- Made Unix stream and vsock framing cancellation-safe, hardened systemd
  descriptor adoption and socket I/O semantics, redacted identifier debug
  output, and added closed-class observability and operator remediation.
- Keep shell socket tests within AF_UNIX path limits using private short-lived
  temporary roots.
- Kept missing-secret, outage, timeout, status, audit, log, and telemetry outcomes stable and free of credential material or identifiers.
- Sealed session capability minting behind one provider-owned, non-Clone authority with exact consumer and generation checks; lifecycle fencing now makes admission, inspect, disconnect, and finalization race-safe and drains admitted leases.
- Keep rejected GPU worker identities owned through finalization so failed
  starts cannot respawn or release Host-global authority before closure.
- Fail closed on ambiguous or quarantined GPU restart adoption to prevent
  duplicate workers.
- Recovered device TPM workers safely after confirmed stale-process crashes without risking duplicate workers during ambiguous liveness checks.
- Enforced persisted device ownership, zone, and physical-key exclusivity before opening security-key relays.
- Preserved typed TPM directory-hardening audit records and established the swtpm control endpoint before initialization.
- Fixed interaction composition to derive display identity from committed
  WaylandSession state and authorize multi-Guest clipboard and notification
  operations against authenticated Provider routes without weakening
  fail-closed identity checks.
- Fixed picker materialization to authorize the committed Guest, Zone, and
  route before consuming its one-use receipt.
- Fixed display reconciliation to require the committed Wayland observer User
  and preserve the authenticated display identity across restart reconciliation.
- Prevented failed bootstrap deliveries, unhealthy guest control, expired
  leases, and relay socket loss from being reported as ready or leaking
  session capacity.
- Keep notification source and host-sink lifecycle tests in the Provider crate
  so supervisor packaging stays on the closed effect-port allowlist.
- Resolve interaction ComponentSession Guest subjects and Host execution
  references from committed Zone resources, and carry committed Provider and
  controller generations instead of transport-derived identities or
  generation-one placeholders. Missing or ambiguous committed identity state
  refuses composition while preserving durable process restart adoption.
- Cover display reconciliation, notification delivery, and clipboard capture
  on the production composition path with non-default generations and
  wrong-Guest refusal.
- Fixed persistent-shell integration tests failing when nested checkout paths
  made their Unix socket paths exceed the platform limit.
- Preserve broker-first host preparation error precedence while retaining
  bundle validation before v3 NetworkManager effects.
- Fixed relay finalization, transport close budgeting, degraded observation
  retention, and end-to-end open deadlines.
- Host-tool derivations keep a bounded 10 GiB sccache and visibly reject an
  exposed but unsafe cache while retaining the plain-rustc fallback when the
  configured cache is absent.
- Make the runtime-created Zone self-resource a complete controller-owned
  envelope before Resource API admission.
- Allow system-core authorization for Credential materialization targets emitted
  by Resource API CommitBatch routing.
- Wired Cloud Hypervisor Guest lifecycle through authenticated Resource API
  composition and controller-owned VMM Process children.
- Kept legacy VM, TPM, security-key, and audio connectors off the v3 Guest
  lifecycle path.
- The daemon can create the per-Guest subdirectory of a compiler-generated
  `store-view-<guest>` Volume. The generated store view's local-path fallback
  selected the shared state root, which is owned `root:d2bd 0750` for a reader,
  so `ZoneVolumeRootResolver::resolve_root`'s `mkdirat` failed `EACCES` and the
  Volume never resolved; the fallback now selects the daemon-owned policy root
  (`daemon-state`), the same choice the spec store made for the same reason.
- The `guest-shell-service` vmCheck now boots the Guest target agent instead of
  passing while it restart-looped. It installs a fixture ComponentSession
  bundle and key pair at the production `root:d2bd 0640` owner/mode, carries a
  real virtio-vsock device, and asserts the agent is active and logs
  `Guest ComponentSession listener bound` - and that no
  `Guest process bundle validation failed` line ever appears. The acceptance
  Guest in `runtime-cloud-hypervisor-guest-preflight` carries the same
  no-bundle-failure assertion.
- The controller-session read seam resolves its Zone plane per read instead of
  snapshotting the composition's plane table at attach time. The table is
  filled only after the per-zone loop, so the eager attach always saw an empty
  table: the daemon admitted an external Provider controller session and then
  silently dropped it (`CloseReason::RoleMismatch`) after
  `controller_context_is_current` fell back to the durable store, where a
  converted `Process` row never exists. Every external controller session now
  goes live, and a rejected admission logs at WARN with the reason.
- A converted `User` row's status is read from the manager wherever the
  durable row was read alone. Since the System-core conversion the manager is
  the only status authority for `Host`/`User` (R11), and the bundle path
  materializes every row into the durable store with a `Pending` status that
  nothing ever updates. Identity resolution refused every public request
  (`IdentityUnbound`), and the policy compiler dropped the whole RoleBinding
  whose subject was that user, so subject issuance answered `NoMatchingGrant`.
  Identity resolution overlays the manager's live phase onto the rows it
  judges; the row's uid, generation and revision stay store-authoritative.
- The policy compiler binds a subject on its committed identity, not on the
  producer's status currency. It required `phase == Ready` and
  `status.observedGeneration == metadata.generation` before binding a
  RoleBinding subject, which made every compiled grant depend on a status
  stream the compiler does not own: converted rows take their status from the
  manager (KTD3) and the durable bundle path materializes them `Pending`
  without ever updating them, so the binding was dropped at every compile -
  during boot, after the projection repair, and on the controller-session and
  controller-policy-refresh paths, which never consulted the manager at all.
  A subject row now qualifies unless it is `Deleted` or `Failed`; liveness
  stays where it is judged (identity resolution for a caller, session
  establishment for a controller). The boot-time repair that recompiled the
  projection once the manager's rows settled, and the status overlay the
  policy paths needed to satisfy the old gate, are both gone.
- `Process/virtiofsd-<guest>` is no longer projected. The Process driver
  refuses a Guest-owned row that is not `<guest>-vmm`, so the projected worker
  row retried `provider-ticket:guest-process-not-vmm` forever, held its Guest
  not-Ready, and left every dependent resource unready. The live virtiofsd
  worker is minted by the binding driver, which already owned it; the
  projection test now asserts the projection emits no guest-owned `Process`,
  and the preflight fixture bounds the acceptance Guest's owned Process set to
  exactly one.
- The manager-backed API surface reports why it is unavailable: a failed
  manager policy mirror, an absent authorization policy, an unavailable
  manager plane authorizer, and a rejected subject issuance each log at WARN
  with the underlying error instead of failing silently. The controller-session
  rejection path logs its reason instead of a `debug!`.
- The daemon/broker vmChecks that attach the `d2b-state.img` fixture disk
  boot again in a sandboxed `test-host-integration` build. The drive now
  carries QEMU `snapshot=on`, so QEMU opens the read-only store image and
  writes only to an ephemeral per-VM overlay. Without it, QEMU aborted at
  machine start ("Could not open '...state.img': Permission denied" against
  the sandbox's read-only `/nix/store`), which the test driver reported as
  `MachineError: Failed to start the following machines` /
  `Connection reset by peer` and which read as an intermittent boot flake.
- `D2B_VM_CHECK=resource-operator-activation make test-host-integration`
  passes: the daemon boots with the v3 plane, ingests the Zone bundle, and
  the fixture observes the Process slice through the manager-backed API
  (identity, owner reference, observedGeneration) plus PID continuity across
  a daemon restart.
- The Process driver now names its owning resource on the launch ticket.
  It reports the manager-resolved owner key when the manager has one, and
  falls back to the owner reference the row was authored with
  (`ResourceContext::metadata()`) for owners the manager does not manage -
  every unconverted owner, and the reason the controller Process could not
  form a launch ticket.
- The Process driver binds the launch ticket's resource revision to the
  row's generation. The new store has no zone-wide commit revision, and the
  conformance ticket rejects a zero revision.
- Process provider failures are logged where they are classified
  (`ProcessDriver`'s provider-error mapping and launch effect). The launch
  effect previously discarded the provider error outright, so a resource
  whose status is memory-only (R11) failed with no observable reason.
- The Cloud Hypervisor Guest's `ch-api` Endpoint reaches `Ready` and the Guest
  converges with its controllers. The Endpoint driver's control family was
  derived from only one of the provider's two fixed child roles: it admitted
  the Guest-produced, cross-domain `guest-control` shape and refused the
  `ch-api` shape the provider commits on the VMM Process (producer
  `Process/<guest>-vmm`, materialized `locality: host-local`), so that row
  failed `validate` terminally (`endpoint-shape-unsupported`), the guest's
  endpoint-publication stage refused its `Failed` phase, and the Guest stuck
  `Pending`. The admitted family is now derived per purpose from the
  provider's own role vocabulary (producer ResourceType and materialized
  locality), and the Endpoint presence probe reads the evidence row that role
  declares: the producer row itself for `ch-api`, the guest's deterministic
  VMM Process child for `guest-control`.
- The endpoint-publication stage defers on a child's retryable `Failed` phase
  instead of refusing it. The `ch-api` Endpoint actor's bounded realize effect
  waits for the VMM evidence and fails retryably while the VMM is still coming
  up; treating every `Failed` phase as terminal failed the whole Guest effect
  (`CapabilityUnavailable`) rather than letting the endpoint's own requeue
  converge. A terminal child failure still refuses.
- The `guest-session-endpoint` read is manager-authority for the converted
  `Endpoint` type. The pre-v3 store fallback reported `ResourceNotFound` for a
  row it does not own (logged as a failed store read); a row the manager does
  not hold is now the honest not-committed answer, and a manager or plane
  failure stays a read failure the callers retry - never absence.
- A deleted Cloud Hypervisor Guest now drains instead of requeueing forever.
  Once the deleting mark was committed the Guest row served the `Deleted`
  tombstone (the closed phase vocabulary has no `Deleting` value), and the
  session-target resolver refused it, so the delete pass ran with no session
  target, the controller's `CloseSession` step answered
  `cloud-hypervisor-resource-authentication`, and the row held its durable
  deleting mark with a retryable `Delete` driver failure. A row in its delete
  pass is now admitted on its own deletion mark - it still exists and still
  owns the committed identity and live session the deletion path reuses - for
  the Guest and, while the Guest is deleting, for the guest-control Endpoint
  the deletion cascade marked with it. A terminal row that is *not* deleting
  is still refused exactly as before.
- The Guest deletion's session steps (`DrainGuestLocal`, `CloseSession`) close
  over the live session the daemon actually holds for the Guest identity, the
  same identity-scoped lookup the planning observation already uses, instead
  of requiring the committed guest-control Endpoint row to still exist: that
  Endpoint is the Guest's owned child and the manager's deletion cascade
  retires it on its own schedule, which can precede the Guest's finalization
  steps. The Guest row and its committed uid stay the fence, and the closed
  keys are recorded as before.
- A wrong-plane access is no longer indistinguishable from "row absent": the
  refusal is logged with the type and the caller and never retried.
- A row that is simply not there yet reports "not yet" instead of a terminal
  refusal, so a binding whose parent row has not been observed defers rather
  than failing its reconcile permanently.
- A volume binding whose parent volume has not been committed yet no longer
  fails terminally. The absent parent row now requeues, and the owner
  mismatch failure is reserved for a parent row that is present with a
  different owner - so a binding that is simply early no longer reports a
  permanent failure that only a later reconcile could clear.
- A delete confirmation and a no-op spec update no longer serve a stale or
  placeholder status while `get` serves the live one; both now report the
  actual observed state, and a row with no current observation reports the
  pending projection rather than status bytes that a previous actor had
  persisted.
- Restart re-links ownership before spawning actors: rows load in
  `(zone, type, name)` order, so a Volume chain's children could load before
  their owning binding and lose their owner edges; the manager now indexes
  every row and links ownership in a second pass. A crash mid-delete
  therefore resumes with the parent held until its children retire instead
  of retiring the parent while they are still tearing down.
- Manager-backed rows render a complete, strictly decodable envelope again:
  spec-shaped rows now carry `metadata.managedBy` and
  `metadata.configurationGeneration`, a complete top-level status (contract
  shape with `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`,
  `outcome`, `update` and `resource`, `Pending` at the row's own generation
  when nothing was published), the contract's `Deleted` phase for a deleting
  row, and a payload digest resealed after any status or deletion stamp.
  Before the fix every manager row was refused by the strict envelope reader
  ("unpublished status: strict decode failed"), and deleting rows rendered a
  phase the contract does not contain.
- The Cloud Hypervisor setup-Volume read is manager-first: a row the manager
  holds resolves through the plane bridge, an absent row is retryable
  (`Pending`), and a manager/store read failure stays an error - never
  absence. The Guest's controller-finalizer custody gate reports the
  provider controller's finalizer present for a manager-served row, so the
  CH controller's child batch commits instead of the reconcile returning at
  the custody gate.
- The store-view farm path is traversable by the daemon: the anchored walk
  opens intermediate path components `O_PATH|O_DIRECTORY` (leaf read-only),
  and the broker postures the broker-created ancestor chain traversal-only
  for the daemon's group (search only, never read/write), so the generated
  `store-view-<guest>` Volume resolves instead of failing `EACCES`.
- A session is no longer torn down when a readiness notification arrives a
  moment before the data does. The socket layer returned an empty read as if
  the peer had closed, so under scheduling pressure a live session could
  report `session-disconnected` on both sides - and the local side closed the
  socket for real. An empty read now parks and waits for the peer's next
  record; only a genuine close (or a real socket error) reports a disconnect.
- A peer's first record on a named stream can no longer fail the whole
  session with `invalid-channel`. Named-stream registration is local, so a
  peer that writes immediately after its handshake could be routed before the
  receiving side had registered the stream; the affected endpoints now
  register their streams before the driver task that routes inbound records
  exists.
- The store-view Volume converges: its declared `sync.lock` entry
  (`leaseClass: file-record`) is adoptable again. The broker now records its
  live ownership into the lock while it holds the exclusive `flock(2)`
  (`d2b-broker` `StoreSync` / `StoreVerify` → `hardlink_farm::SyncLockOwnerRecord`),
  and the daemon verifies that record against the kernel before adopting the
  entry (`d2bd` anchored adapter: same boot id, recorded pid still alive at
  exactly the recorded `/proc/<pid>/stat` field-22 start time, and the
  kernel-reported `comm` equal to the recorded owner). A lock with no record,
  a foreign payload, a record from another boot, or one naming a dead process
  stays ambiguous and quarantines exactly as before, so
  `adopt-with-live-owner-proof` keeps its meaning for every other lease class
  (no blanket quarantine exemption). A `file-record` lock the daemon has to
  create itself records its creator as the first live owner, so a lock no
  broker materialized is not self-quarantined. The record is deliberately
  descriptive evidence, not an attestation: it proves liveness and shape, and
  a same-uid writer could forge it - the quarantine protects against
  stale/ambiguous ownership, not against a writer in the file owner's trust
  domain.
- A legitimately reconnected Guest reports `Ready` again. The Guest
  incarnation fence pinned the session's *live accepted* generation, which
  advances on every reconnect by design (the Guest admits only a strictly
  newer generation - `session_generation_is_fresh`,
  `next_session_generation`), against the enrollment-scoped generations, so
  after a daemon restart the row served `phase: Pending` with
  `runtimeReady: false` / `bootstrapReady: false` forever. The fence now
  carries the *enrolled* identity generation the live session was admitted
  under (the generation the Guest re-verifies as the floor of every
  acceptance); the live accepted generation stays the binding's
  `session_generation`, the freshness marker the lifecycle plans consume.
- A VMM Process whose identity has not actually changed survives a daemon
  restart. The retire-before-launch pre-flight treated a per-pass identity
  input the current pass could not resolve (the owning Guest's uid, resolved
  after the adopting pass seeded the entry) as an identity *change* and
  destroyed the live `cloud-hypervisor-runner`, so every restart relaunched
  the VMM and invalidated the Guest's enrolled boot identity. Retirement now
  requires a *proven* change - both sides known and different - while the
  destructive gates (`finalize`, `stop`, `has_active`) keep the strict
  comparison, so an unresolved request still refuses rather than proceeds.
- A reconcile pass that computed a `status.resource` projection publishes it
  whatever its outcome: `InProgress` (a long effect in flight) used to drop
  the projection it had just computed, leaving the row on the pre-pass layer
  while the driver already knew better. Invalidation - a spec change or
  deletion - is the only drop case.
- The VolumeBinding driver's guest-mount gate reads real evidence again. The
  KTD6 drain gate had no observation surface after the volume-leg conversion
  and answered the documented `Ok(false)` default; it now asks the Zone target
  directory for the row's own assignment and only a target-local realization
  the Guest reports `ready` answers "mounted". No assignment, no live session,
  no recorded realization, and a stale-generation handle all answer "not
  mounted", so a drain never force-clears a serve the target cannot confirm.
- A failed pre-start flush now fails the device path. The one-shot row's phase
  cannot carry its outcome (a concluded pass publishes Ready whatever happened),
  so the TPM effect reads the row's `ephemeral` status projection: `succeeded`
  proceeds, `failed` fails the effect closed, and an unreadable projection is
  refused instead of read as success.
- A Process row awaiting its relaunch publishes `Pending`, never `Ready`. The
  retry arms return `ReconcileOutcome::RetryScheduled`, the runtime maps it to
  the pending phase, and the driver's typed status stays
  `AwaitingRestart { restart_count }` until a pass actually satisfies the row, so
  a row waiting for a restart can no longer read Ready over a process that does
  not exist.
- A Device worker runs as its trusted principal inside its namespace. The
  worker's argv now names the in-namespace socket owner (the single-entry
  mapping's uid/gid 0) for namespaced postures, and the broker grants the
  launched principal the per-runner ACL on the device socket directory
  (`grant_device_worker_launch_acls`, restricted to sockets strictly inside the
  broker runtime root). Previously the argv named the host-numeric principal -
  unmapped inside the namespace - so swtpm exited with an ownership error and the
  socket directory carried no entry for the worker.
- Provider declarations now spell their ACL entries the way the closed Volume
  contract reads them: the TPM state Volume declares `rwx`/`rx`/`rw` instead of
  the POSIX symbolic `r-x`/`rw-`, which the grant decoder rejects - the durable
  Volume spec failed to decode (`volume-spec-invalid`) and the whole TPM device
  path was stranded. The storage-path projection follows with the same
  spellings.
- Ten Endpoint purposes across eight providers now use the closed tokens the
  Endpoint contract admits - `swtpm-tpm-socket`, `swtpm-control-socket`,
  `clipboard-wayland-bridge`, `security-key-ctaphid`, `usb-guest-proxy`,
  `display-wayland-cross-domain`, `notification-desktop-sink`, `qmp-control`,
  `audio-pipewire-host-worker`, `audio-pipewire-guest-agent` - instead of the
  dotted spellings that failed endpoint admission. The provider Nix tests pin
  each declaration.
- The Cloud Hypervisor child relist (`RelistOwnedChildren`) reads exactly the
  session owner's `Process`/`Endpoint`/`Volume` children: the manager leg is
  owner-scoped through the manager's own owner filter instead of listing every
  row of those types in the Zone, so a Zone whose converted rows exceed the
  durable page bound - 85 guests' worth of children crossed 256 - no longer
  fails every guest's relist with `Truncated`, and another owner's rows never
  appear in (or bound) the answer. The manager's complete, unpaginated list is
  folded over the durable leg past the durable 256-row page bound, with the
  manager rendering still winning where both planes hold a reference; the
  durable leg keeps its paging and cap.
- An `AudioBinding` whose `AudioService` (or Guest) row is not committed yet
  defers with the `Pending` phase instead of failing terminally: canonical
  bundle order commits the binding before the service it names, and an
  API-created binding can precede its service entirely, so the old
  `InvalidResource` classification (the terminal `SpecInvalid` driver failure,
  which schedules no requeue) stranded every such binding `Failed` forever. A
  committed row that is not the named dependency keeps the terminal identity
  refusal.
- The serving worker no longer runs with the daemon's uid/gid. A binding-owned
  serving launch runs as the trusted intent's principal with per-runner ACLs on
  its socket directory and view root (`grant_serving_worker_launch_acls`), so a
  compromised worker can no longer borrow the daemon's identity for the broker's
  peer-identity check; a ticket that names no broker socket, or one outside the
  broker runtime root, is refused instead of granted.
- A credential Provider session is admitted against the policy it was built
  with: the session's transport evidence is derived from that policy's channel
  binding digest instead of a hardcoded constant, which had made the credential
  lane unadmittable while its check passed vacuously. Evidence compiled from
  another policy is refused as a channel-binding mismatch.
- Manager-plane LIST honours its filters and its cursor: owner filters
  (`owner.resourceUid`, `owner.resourceRef`) match the row's real ownership, the
  page is deterministic with an opaque keyset cursor and an honest `truncated`,
  and a malformed or selector-mismatched cursor is refused with a typed reason
  instead of being ignored.
- Manager-plane WATCH refuses with `UnsupportedCapability` (`watch-not-wired`)
  rather than returning a receipt for a stream nothing fills; the consumer-less
  producer machinery is deleted. LIST is the enumeration surface until a pump is
  wired.
- An ensure whose spec is unchanged but whose metadata or owner differs now
  writes those columns and notifies the actor instead of reporting `Unchanged`;
  a trigger that coalesced during a terminally failing pass or effect is
  reconciled instead of being lost with the failure; the SQLite side files are
  tightened under the names SQLite actually creates (`-wal`/`-shm` appended to
  the database file name).
- A durable `Process` row that reached `Ready` stayed under observation, on the
  preserved 5s resync: the pass re-reads the process through the Provider's
  liveness probe, so a controller, worker or VMM that exits leaves `Ready`, its
  `restartPolicy` decides between one budgeted restart (at the policy backoff)
  and the terminal `process-exited` report, and dependents watching the row
  learn the producer is gone instead of waiting on a row that reports `Ready`
  forever.
- A durable launch whose ticket the trusted bundle can never mint
  (`template-not-found`, `resolution-failed`, `guest-process-not-vmm`) now
  fails terminally instead of consuming the restart budget and retrying - and
  warning - forever; provider-effect and not-yet-bindable identity refusals keep
  their budgeted retry.
- A derived owned-child ensure could silently re-parent an existing child row
  to a different owner. The manager now refuses a child ensure (single or
  declarative diff) whose parent is not the row's committed owner, so the
  durable owner, generation and spec survive and only the committed owner can
  update the child in place. Binding a not-yet-owned row stays the authored
  `Ensure { owner }` decision.
- Corrected the heavy-gate specification and migration map to describe the
  protected root-provisioned runtime namespace, its two provisioning paths,
  and its fail-closed no-fallback behavior.
- Corrected the runtime-ledger documentation to distinguish enforced aggregate
  per-crate process CPU from advisory per-test wall clock, and documented the
  exact closed test census without claiming a baseline or historical
  regression check.
- Updated the delivery specification for the required complete pull-request
  mapping and delivery artifact schema version 2, and documented `.scratch/`
  as the required home for throwaway probes.
- Corrected contributor and reference documentation that named retired shell
  gates as current enforcement, distinguishing enforcing coverage from
  fixture-dependent policies and historical evidence.
- Reconciled Integration and Detailed-design contract rows across 30 of the 55
  resource specifications with the decision register, correcting roughly 35
  contradictions in lifecycle, ownership, authorization, placement, and
  provider behavior.
- ADR 0046 spec docs: corrected stale current-state prose that still described
  the spec set as `Proposed`.
- ADR 0046 topology: replaced the obsolete `W0`-`W10` program range with the
  current topology terminology in the decision register.
- The tier-0 dash scan no longer reports success when the scan itself errors
  (unreadable file, vanished file, bad pattern) or when file enumeration fails;
  it now distinguishes "no matches" from a scan error and fails closed on the
  latter.
- The changelog fold now stages the rewritten changelog on the same filesystem,
  promotes it atomically, and reserves fragments before promotion so a failed
  write or deletion leaves `CHANGELOG.md` byte-unchanged with every fragment
  intact instead of a corrupted or partially consumed changelog.
- A terminating signal arriving exactly as a heavy lane's leader process exited
  could break supervision without sweeping the process group, leaving orphaned
  descendants holding the slot descriptor so the slot was never released.
  Supervision now drains pending signals before each exit check and once more
  after the child exits while signals are still blocked, then sweeps the group
  before reaping the leader.
- ADR 0046 current-state prose: corrected the remaining current-source, evidence,
  and delta rows that still asserted no spec parser, generated graph, delivery
  machinery, heavy gate, or heavy-lane Make targets exist, distinguishing the
  tooling that has landed from the surfaces that remain to be hardened.
- The ADR 0046 datetime lint now validates that a millisecond-precision
  timestamp names a real instant, rejecting impossible calendar dates (for
  example month 13, day 31 of a 30-day month, or February 29 of a non-leap
  year) and leap seconds (`:60`), not merely the `YYYY-MM-DDTHH:MM:SS.sssZ`
  shape.
- The ADR 0046 ResourceType lint now enforces the frozen qualified grammar in
  full: a qualified token must be `<provider>.d2bus.org.<Type>` where the
  provider matches `^[a-z][a-z0-9-]*$` within 63 bytes and the type matches
  `^[A-Z][A-Za-z0-9]{0,62}$`, and an unqualified token must be one of the
  standard catalog names. A token missing the provider segment, carrying an
  extra segment, using a lowercase type, or exceeding the byte bounds is now
  rejected instead of accepted.
- The ADR 0046 retry-scalar lint now verifies that the frozen millisecond value
  is an integer, rejecting a quoted-string or floating-point value that the
  earlier key-and-shape check accepted.
- The changelog fold is now recoverable across an abrupt interruption. It keeps
  a durable transaction journal and a byte backup of the previous changelog,
  fsyncs each state transition, and preserves the original changelog until the
  promotion rename succeeds. A later run detects an interrupted fold and either
  finishes a committed one or rolls an uncommitted one all the way back, so a
  crash can never leave a half-consumed fragment set or a changelog missing the
  entries whose fragments were already removed.
- Heavy-gate nesting verification now proves the advertised inherited
  descriptor is the one holding the slot lock. It issues a nonblocking
  `F_OFD_SETLK` through the inherited descriptor itself instead of probing a
  fresh handle, so a forged nesting marker that supplies an unlocked descriptor
  for a slot another lane happens to hold can no longer run a third concurrent
  lane, and the check-then-use race is removed.
- Heavy-gate now verifies the fixed root-provisioned namespace that holds the
  shared semaphore before use. It accepts only the root-owned, non-writable
  root and per-uid directory plus the pre-created target-uid-owned mode-`0600`
  slot files, and has no user-owned or temporary fallback, so neither a peer nor
  the target uid can rename a slot name between invocations to split the
  semaphore into a second namespace.
- Heavy-gate unconditionally terminates and reaps the supervised process group
  after the leader exits, before restoring the signal mask, closing the window
  where a signal arriving between the post-exit drain and the conditional sweep
  could kill the wrapper and orphan slot-holding survivors.
- Reworded a doc comment whose prose parenthetical left a hyphen at the start
  of a line, which rustdoc parsed as a malformed list item.
- Corrected the ADR 0046 body prose, which still described a W0 to W7 launch
  range after the terminal friction-closure wave was added.
- ADR 0046 spec set: completed the universal-status sweep across every complete
  resource envelope, adding the mandatory `status.update` (D091) currency object
  and nesting type-specific fields under `status.resource` (D107) wherever a
  complete envelope still omitted them, including the Credential example that had
  used `status.credential` instead of `status.resource`. The corresponding
  claim in the earlier ADR046-W0fu2 changelog fragment, which asserted the sweep
  was already complete, has been corrected.
- ADR 0046 Host/Guest execution policy: added a valid `defaultUserRef` to every
  mixed-domain example that still omitted it or set it null (D116), retaining the
  omission only in explicitly labelled rejection fixtures, so the superset
  invariant holds across the primitive-resource-composition, system-core, and
  credential-entra specs and the remaining mixed-domain examples.
- ADR 0046 current-state prose: corrected the remaining source rows so they
  distinguish completed contracts from the binding, wiring, and hardening work
  that remains.
- Contributor docs: corrected the spec-literal lint allowlist guidance in
  `AGENTS.md`. There is no inline `d2b-lint-allow` marker; the lint rejects that
  escape hatch, and the sole exemption is the decision-register row that defines
  a rule in `docs/specs/ADR-046-decision-register.md`.
- The changelog fold's committed-transaction cleanup is now restart-safe. It
  removes the committed payload in journal-last, fsynced order so the
  `COMMITTED` marker is the last thing cleared, instead of an unordered
  recursive delete that could drop the marker while a restorable backup and the
  reserved fragments survived and cause a later recovery to roll a promoted fold
  back - duplicating or losing entries. Inline recovery failures are now folded
  into the surfaced error rather than discarded, and recovery is proven
  idempotent under interruption after promotion and throughout both forward and
  rollback recovery, so repeated recovery always leaves the changelog folded
  exactly once or fully rolled back.
- The heavy-gate teardown no longer risks signalling an unrelated process
  group. The supervisor now observes the leader's exit without reaping it
  (`waitid` with `WNOWAIT`), sweeps the process group while the still-present
  zombie pins the pid and pgid, and only then reaps the leader. Previously the
  leader was reaped before the group sweep, so an emptied group's numeric pgid
  could be recycled onto a stranger before the sweep ran.
- Removed a stale process marker from a test-harness source comment.
- Brought every complete ADR-046 resource envelope in `docs/specs/**` into the
  D088/D107 three-layer status shape. Each envelope's `status` now carries both
  the universal `status.update` currency object and the `status.resource`
  ResourceType-common base (`{}` where the type declares no common fields), so
  the D107 "resource layer present on every resource" contract holds
  universally. ResourceType-specific status containers that sat directly under
  `status` (for example `device:`) are nested under `status.resource`, and the
  Endpoint base-status fields that were authored flat are grouped there too.
- Marked the D116 Nix counter-example in `ADR-046-nix-configuration.md` as an
  intentional negative example with an explicit
  `d2b-lint: expect-d116-eval-error` marker, so the eval-time-rejection
  teaching block is exempt from the `defaultUserRef` structural lint without
  weakening detection of real declarations.
- Propagated D119 across every spec that still froze retired bundle names,
  replacing `contentId`, numeric bundle generations, `resources.json`,
  `bundleSha256`, `catalogSha256`, `BundleManifest`, and
  `retainedConfigurationMax` with the D119 `contentHash`,
  `resource-bundle.json`, no-manifest, and sole-`retainedGenerations` contract,
  and corrected D119's affected-specs column to list every reconciled spec.
- Corrected the decision register to stop describing completed work as pending:
  D121 no longer says the host/guest/process spec "must adopt"
  `backoffMultiplierMilli` (already adopted), and the resolved-decision and
  cross-provider sections of the network spec no longer cite the wrong register
  ID or mark USBIP firewall reconciliation as pending.
- Mapped USBIP apply and release firewall intent onto the shared
  `ApplyNftablesProjection` op (D124) across all sibling cells in the network
  spec, replacing the fictional `UsbipBindFirewallRule { action: Ensure |
  Remove }` shape; USBIP firewall release is documented as net-new privileged
  surface rather than an existing action.
- Required the Azure relay transport dossier to terminate the IKpsk2 bootstrap
  session after enrollment and complete a distinct enrolled KK handshake before
  `Ready`, in place of rekeying the bootstrap session into steady state, and
  added a validation case rejecting continuation or resource traffic on
  IKpsk2-derived state.
- Removed the systemd path unit from configuration bundle watching in the
  provider-state spec; the Zone daemon watches the installed bundle in-process
  and is signalled through the activation protocol, preserving the
  three-root-visible-unit contract.
- Make recipes and shared test helpers now discard inherited Bash functions
  before resolving tool names. An exported function can no longer shadow a
  PATH stub or expected system binary and silently redirect a gate.
- Runtime-ledger and heavy-gate build failures now retain actionable compiler
  diagnostics through a shared path redactor. The filter resolves symlinks,
  treats path metacharacters literally, respects path-component boundaries,
  redacts other absolute paths, and suppresses raw output explicitly if safe
  filtering is unavailable.
- The runtime ledger now refuses a crate stream with no timed test events and
  pins the exact expected test identifiers as well as crate names. A vanished
  test can no longer turn into a zero-duration crate measurement or silently
  shrink the measured census.
- Runtime enforcement now uses aggregate process CPU time for each complete
  crate suite instead of libtest wall-clock time, without raising the existing
  crate budget. Per-test wall-clock timings remain explicitly advisory, so
  unrelated machine load cannot manufacture a regression while the exact
  non-empty test census remains mandatory.
- Delivery snapshot Git failures now classify only anchored Git diagnostic
  phrases after removing quoted caller-controlled values. A keyword in a path,
  revision, or URL can no longer misreport a healthy repository as corrupt or
  select another unrelated repair reason.
- Process-marker and dash scan abort messages no longer include the absolute
  scan root.
- The directly invokable heavy-gate self-guard now removes inherited Cargo and
  Rust compiler shell functions before building its verifier, and explicitly
  bypasses function lookup for the Cargo command.
- Host inspection and CLI subprocesses now use a fixed root-owned executable
  search path, while the privileged broker invokes udevadm through an absolute
  NixOS system path.
- Process-marker ratchet failures now identify the gate and require stale
  exemptions to move from `activePaths` to `retiredPaths`, while explicitly
  rejecting any change to the frozen path universe.
- Rust tests that launch Bash or POSIX shell fixtures now route through the
  inherited-function scrubber, including direct Cargo test invocation.
- The manifest-driven Layer-1 local and CI graphs now run the dedicated
  nix-unit corpus target and require it in the CI rollup.
- The continuous-integration workflows named their environment-scrubbing
  shell by a repository-relative path. The Actions runner resolves the shell
  program against `PATH` rather than the workspace, so every job failed
  during startup before running any step. Steps now run through
  `tests/tools/ci-shell`, invoked as `sh tests/tools/ci-shell`, which keeps the
  runner's lookup on `PATH` and defers resolving the wrapper to run time. The
  dash bootstrap uses only shell builtins until the scrubber execs, so exported
  Bash functions and `BASH_ENV` are removed before any Bash process or step can
  run.
- `make check` described itself as the pull-request-equivalent Layer-1 gate
  but ran only each job's primary make target, while the continuous
  integration `tier0` job also ran the ADR index and CI coverage guards. Those
  extra targets are now declared in `tests/layer1-jobs.json` and consumed by
  both the workflow renderer and the local runner, so a job cannot run more
  in continuous integration than it runs locally.
- Layer-1 failure diagnostics now identify the exact primary or extra make
  target that failed, name retained output by its semantic job identifier, and
  redact repository, home, and other absolute paths from the printed tail. The
  in-process filter mirrors the xtask redactor because the early runner cannot
  assume that the Rust helper has already been built.
- The runtime ledger's per-crate process-CPU budget was calibrated against the
  reference development host and was red on every GitHub-hosted runner, where
  the same suite measures roughly a third more process CPU. The budget is an
  absolute ceiling rather than a regression anchor, so it now clears the
  highest observed continuous-integration sample with headroom, and the test
  that exercises it derives its sample and expected message from the constant
  instead of restating the number.
- Escaped a literal pipe inside a code span in the ADR 0046 decision register
  so the affected row renders as four columns instead of five.
- NixOS activation no longer fails when a configured lifecycle user is
  temporarily unavailable through a network-backed identity provider. Heavy
  validation remains fail-closed until that user provisions protected runtime
  slots after login.
- Diagnostic filtering now preserves a redacted tail when truncation or
  malformed bytes split a UTF-8 sequence, instead of suppressing the full
  compiler diagnostic.
- Runtime-ledger census regeneration now fsyncs a same-directory temporary file,
  atomically renames it over the pin, and fsyncs the parent directory.
- Structural JSON, YAML, and Nix policy lookups now reject duplicate direct-child
  keys instead of silently selecting the first value.
- Contributor guidance now describes the frozen active and retired
  process-marker path-universe pin and its independent checker.
- Corrected contributor guidance for the frozen process-marker path universe:
  cleaned exemptions move from `activePaths` to `retiredPaths` and are never
  deleted from the digest-protected universe.
- Documented every local Layer-1 job from the authoritative manifest, including
  the migration-ledger and changelog gates and the performance-budget advisory
  job, and
  distinguished the full pre-merge gate from the post-preflight development
  umbrella.
- Reconciled heavy-gate provisioning, inherited-slot verification, and runtime
  ledger guidance with their current fail-closed implementations.
- Removed superseded delivery-reference and policy-lint mechanism descriptions
  from earlier changelog fragments.
- Aligned every Provider Process restart-policy example with the integer
  fixed-point multiplier contract, so authored Nix specs no longer contain
  rejected floating-point values.
- Corrected ResourceName and ZoneId documentation to enforce the canonical
  1-to-63-byte bound across resource envelopes, Nix validation, and Provider
  examples.
- Made the drift driver invoke regular gate files through Bash regardless of
  their executable bit, preventing a referenced gate from being silently
  skipped after a mode change.
- Wired the frozen process-marker universe checker into generated-artifact
  drift validation so active and retired marker pins are enforced in Layer 1.
- Made the Layer-1 lint job run its mandatory disk-space preflight regardless
  of executable mode and fail closed when the tracked preflight file is absent.
- The shared heavy-gate self-guard helper is now bounded and derives everything
  from its own on-disk location. It always rebuilds `xtask` from the canonical
  checkout (so a stale binary without the slot-verification subcommand can never
  be used as-is), normalises a relative target directory against that checkout,
  ignores the caller-supplied root and target-directory variables when locating
  the binary, and enforces a fail-closed re-exec depth limit so a binary that
  keeps failing verification can no longer loop forever.
- Heavy-gate slot verification now distinguishes a genuine "no slot held"
  verdict from a verifier malfunction. Environment, permission, and unsupported
  errors during the ownership proof are returned as typed errors with distinct
  non-zero exit codes instead of collapsing into the "unheld" verdict, and the
  shell guard branches explicitly: proceed when held, re-acquire when unheld,
  and propagate anything else unchanged so a broken verifier fails closed rather
  than silently re-acquiring.
- The concurrent-candidate-creation regression test now forces the `mkdirat`
  `EEXIST` race it is meant to cover. A test-only synchronization point releases
  both racing writers only after both have observed the directory absent, so the
  test provably exercises the concurrent-creation branch and asserts both
  writers still succeed.
- Aligned the Nix resource-shape template in `ADR-046-resources-volume.md` to the
  `type = "<ResourceType>"` placeholder convention used by every sibling spec,
  in place of a bare `type = "ResourceType"` that read as a concrete but invalid
  ResourceType name.
- Completed the universal status base on the `type: Host` envelope in
  `ADR-046-telemetry-audit-and-support.md`, which showed an isolation-posture
  status subtree without the `status.update` and `status.resource` base the
  universal-status contract requires.
- The runtime-ledger gate now fails closed on any test-runner failure. It
  captures the runner exit status, requires a matching successful
  suite-completion signal, and refuses to record measurements from a partial
  or crashed run, so a compile error, a signal, or a runner-level failure can
  no longer produce a stable partial stream that satisfies the gate while the
  underlying suite never passed. Only redacted diagnostics are retained on
  failure.
- The heavy-gate helper now normalises a relative build target directory the
  same way for both the build and the execution of the freshly built binary,
  so building with a relative target directory and then running the wrapper
  from the repository root no longer looks for the binary under the wrong path.
- The heavy-lane guard now branches on the slot verifier's typed exit status
  instead of collapsing every nonzero result into one "outside the semaphore"
  exit. A held slot proceeds; a genuinely unheld slot directs the operator to
  the acquiring public lane and fails closed with the typed unheld code; and a
  verifier malfunction propagates its exact exit code unchanged, so a broken
  gate is no longer hidden behind a slot-bypass message.
- A failed git invocation during snapshotting is now mapped to a stable,
  path-free reason code (missing object, not a repository, unsafe ownership,
  permission denied, or corrupt repository), and an unrecognised failure is
  reduced to a bounded sanitised cause with paths and control characters
  redacted. The previous behaviour discarded git's diagnostic entirely, which
  left a real failure undiagnosable because git returns the same exit status
  for all of these classes.
- Corrected the static `d2b host check` refusal-contract fixtures and their
  documentation: the socket remediation names `d2bd.service`, the cgroup
  remediations name `d2b.slice`, and the fixture `docs_anchor` values resolve.
  These goldens are aspirational contract fixtures, not code-derived runtime
  output.
- The running `d2b host check` still emits finding-based output without a
  `docs_anchor` field and does not emit the code-specific socket or cgroup
  refusal envelopes. It therefore neither emits the stale `d2b-host.slice`
  remediation nor implements the corrected fixture contract.
- Corrected the AGENTS.md description of the envelope-lint negative-example
  exemption so it matches the lint: one exact, case-sensitive marker, honoured
  only in the single pinned documenting file and only when it appears once,
  with `policy_adr046_envelopes` named as the authority for the exact spelling.
- Corrected the contributor documentation for running heavy gates and folding
  changelog fragments: the `xtask` alias resolves only from `packages/`, so the
  documented invocation is now the root-safe
  `cargo run --manifest-path packages/Cargo.toml -p xtask -- ...`, with the
  `cd packages && cargo xtask ...` alternative noted for the `sccache` path.
- Added a fail-closed tier0 policy scan that rejects internal development
  markers in shipped documentation, source comments, operator-facing CLI
  contracts, workflow labels, and released changelog sections. Existing debt
  is bounded by a frozen path-universe pin: only `activePaths` are exempt, new
  paths fail the digest check, and cleaning a path moves it to `retiredPaths`
  without changing the combined universe.
- Replaced development-wave references in CLI output fixtures with descriptions
  of the actual requirements, implementation state, and remediation.
- The ADR 0046 policy lints now emit repository-relative paths in every
  diagnostic and panic message. A read failure or violation report no longer
  prints the absolute checkout root or a username-bearing path into CI logs;
  each surface renders a path under the repository root, falling back to the
  bare file name rather than an absolute path.
- Preserved a redacted diagnostic tail with an explicit dropped-byte notice when
  compiler output exceeds the safe input bound, and retained repository-relative
  context when reporting paths.
- Reported every per-test runtime threshold breach as a visible, non-failing
  advisory while retaining aggregate process-CPU budget enforcement.
- Closed Nix policy-parser gaps for structural wrappers so nested resource
  envelopes cannot silently bypass status and execution-policy checks.
- Added a deterministic runtime-ledger census regeneration target and made
  census drift diagnostics name that concrete maintainer workflow.
- Heavy test guards and shared shell helpers now invoke Bash cleanup builtins
  explicitly, avoiding confusing failures when a developer exports a tool
  wrapper function.
- Runtime census regeneration now refuses to erase any previously pinned test
  or crate identifier unless the committed census records that removal
  explicitly first.
- The pull-request and local Layer-1 graphs now run the manifest jobs for the
  performance canary and migration-ledger drift check instead of leaving them
  unreachable. The performance job is advisory: without
  `D2B_PERF_STABLE=1` on a pinned self-hosted runner it reports `SKIP`, enforces
  nothing, and is not counted as an enforcing green job.
- Shell commands in generated and handwritten GitHub Actions workflows now
  clear inherited shell functions before invoking repository tools.
- The CI coverage guard now accepts evidence only from Make targets that the
  local Layer-1 manifest executes, so legacy non-executing aggregators cannot
  hide an orphaned gate.
- Rejected resource envelopes unless they declare exactly one type discriminator,
  including duplicate keys that structured parsers previously collapsed or ignored.
- Traversed every structural Nix child that can contain a resource envelope,
  including conditions, assertion predicates, lambda defaults, interpolation, and
  inherit sources.
- Replaced the editable process-marker exemption budget with a frozen path-universe
  pin whose active exemptions can only move into the retired set.
- Closed the ZoneLink enrollment-bootstrap bypass across every
  implementation-driving surface in `docs/specs/**`, not just the canonical
  state machine. The controller algorithm in
  `ADR-046-resources-zone-control.md`, the ZoneLink resource and its validation
  cells, and the
  `transport-unix`, `transport-vsock`, and `transport-azure-relay` provider
  dossiers now all state the exact sequence
  `Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready`: the one-time
  IKpsk2 bootstrap consuming the allocator-issued single-use PSK runs only from
  `Unenrolled` (and after revocation), reconnect re-enters at `KK` from
  `EnrollmentCommitted` without a PSK, and resource-plane traffic is prohibited
  until `Ready`. Every transport sequence description now states explicitly
  that the selected transport Provider never selects, negotiates, or reorders
  handshake profiles, so no implementation-driving surface permits the
  steady-state-only KK-direct downgrade.
- ADR 0046 Nix configuration contract: froze a single per-Zone generation
  layout so the specs no longer disagree on whether the bundle is a monolithic
  document or a per-resource-type file set, whether the index and artifact
  catalog are global or per-Zone, or what the retention setting is called. The
  contract is now one monolithic `resource-bundle.json` per Zone, a single
  site-wide `index.json` and `artifact-catalog.json`, one retention option
  (`retainedGenerations`), and one explicitly enumerated integrity digest chain,
  documented in one canonical section every other spec defers to.
- ADR 0046 Nix configuration contract: removed a fourth root-visible
  `d2bd.socket` unit from the systemd mapping; the daemon binds its public
  socket itself and reports readiness through `Type=notify`, keeping the
  framework at exactly three root-visible units.
- ADR 0046 Provider contract: defined Provider catalog identity as the Provider
  resource's `spec.artifactId` and removed every reference to an undeclared
  `catalogEntryId`, so reference resolution and duplicate-install detection are
  computable from the frozen Provider spec without a hidden side table.
- ADR 0046 configuration activation: specified that the active pointer, prior
  pointer, and retention metadata are persisted together in one atomic durable
  write before reconciliation is notified, and specified continuation-event
  restart recovery, so an interrupted activation can no longer leave a new
  generation active with no durably recorded rollback target.
- Clarified the tier0 gate's message when the shell linter is absent from
  `PATH`. The previous wording read as though shell linting had been skipped
  entirely; the authoritative gate is `make test-lint`, which provisions the
  linter through nix and fails closed when it cannot.
- Corrected the ADR 0046 USBIP firewall specs, which described a broker
  capability that does not exist. The shipped `UsbipBindFirewallRule` op is
  bind-only: it carries a single bundle intent reference, has no `action` field
  and no release path, and routes through the whole-table
  `render_owned_table_replace_script`, which deletes and recreates the entire
  `inet d2b` table. The `device-usbip` provider dossier, the security and
  threat model, and the device resources reference nevertheless claimed a closed
  `UsbipBindFirewallRule { action: Ensure | Remove }` op served both acquisition
  and release with "no separate release operation" and "no new privileged
  surface". The most dangerous of these, in
  `ADR-046-security-and-threat-model.md`, asserted USBIP release was the
  existing op "with closed `action: Remove`, not a renamed or second release
  variant"; that would have told an implementer no new privileged surface was
  needed. Every such statement is now corrected to describe the shipped op
  honestly and to state plainly that USBIP firewall release is net-new
  privileged surface.
- Validate Provider artifact package output selection during Nix evaluation,
  accepting single and explicitly selected outputs while rejecting ambiguous
  multi-output and unrecognised package shapes with bounded diagnostics.
- Validate Provider projection factories against every published semantic
  schema field, including protocol version, binding targets, and
  exportability before accepting fingerprints.
- Hardened malicious Provider admission coverage for stored resource origins,
  semantic backing allowlists, and projection factory descriptors.
- Run harness-free Rust benchmark assertions with optimized release binaries so latency contracts measure production-shaped code.
- Make host capability-confinement coverage ignore unrelated process churn
  without overlooking service-owned helpers left in the host network namespace.
- Keep controller generation cleanup and audit invariants in native Rust coverage instead of rerunning Nix-linked test binaries in a foreign container.
- Resource bundle validation now reports malformed shared resources through
  Nix assertions while keeping compiled bundle generation fail-closed.
- Run assertion-bearing Rust benchmark targets in the manifest-driven test
  companion surface instead of allowing nextest discovery to skip them.
- Use canonical, explicitly framed textual inputs for Zone bundle and artifact
  catalog digests instead of raw NUL-separated Nix hash inputs.
- Isolate Wave 5 Nix evaluation shards and enforce separate aggregate RSS
  ceilings for the complete Nix-unit and flake evaluation lanes.
- Keep guest-control rejection probes on assertion records instead of forcing
  each VM's `system.build.toplevel` during eval-only tests.
- Reject v3-scale VM-closure growth as a baseline blocker rather than raising
  Wave 5 memory ceilings to accommodate it.
- Completed user-only Host posture handling with fail-closed status ownership
  and redacted ProcessEffect launch and stop projections.
- Hardened optional guest journald collection with real OpenTelemetry
  processors that remove sensitive journal metadata and redact credential and
  path-shaped message content.
- Corrected the observability Provider specification so it no longer directs
  the `observability-otel` Provider to write authoritative audit records. The
  Provider is the subject of a `SessionConnect` record, not its author: session
  admission remains the sole writer, telemetry and authoritative audit keep
  their separate writer paths, and the Provider crate takes no audit or core
  telemetry dependency. The closed metric-label policy is recorded as
  single-sourced in the public neutral contract so telemetry ingress and the
  core emitter can never disagree about which labels are forbidden.
- Single-sourced the closed telemetry label and OTEL resource-attribute
  policy in `d2b-contracts`, with distinct fail-closed emitter and
  observability Provider validators consuming the same registry.
- Install coherent v3 Zone resource bundles and artifact catalogs while
  retaining compatibility projections only for evaluation-time consumers.
- Reject secret-shaped Provider configuration during bundle compilation and
  verify catalog and bundle digests against the realised JSON.
- Signed Device extension metadata now validates bounded schema versions and
  applies provider settings bounds only when matching schema metadata exists.
- Preserve typed Zone session pins, cancellation, deadlines, and transport
  errors at the native CLI boundary without making modern calls accept realm
  target inputs.
- Hardened the optional observability OTEL Provider boundary with bounded
  ingress policy, session diagnostics, integration scenarios, and journald
  filtering through the existing observability component.
- Wire Device and Volume resource validation through the Nix evaluation path,
  including fail-closed source, layout, attachment, quota, and selector checks.
- Invalid generated Zone resource bundles now fail closed during compilation,
  while telemetry and audit bounds remain shared by the typed resource surface.
- Telemetry redaction policy checks now cover multiline field forms across the
  relevant Rust source without allowing unbounded source exemptions.
- Kept Zone bundle digest metadata coherent between evaluation and realised
  JSON, and anchored content hashes to the canonical resource bytes that ship.
- Keep persistent Rust compiler cache processes out of disposable stub-test temporary directories.
- Use the v3 auth-status readiness probe in daemon restart checks and document the typed `d2b list <RESOURCE_TYPE>` command.
- Keep audit and telemetry crates explicitly enrolled in workspace validation when dependency edges change.
- Hardened Zone audit export validation so diagnostic output remains bounded,
  redacted, and restricted to the approved public audit schema.
- Enforced append-only audit predecessor validation, bounded segment export,
  and privileged durability behavior.
- Rejected unbounded telemetry labels and redacted identity-bearing span
  fields before bounded emitter admission.
- Keep Bazel test runners local while compiling supported Rust actions with
  BuildBuddy, and retry the complete graph locally after a remote deadline.
- Keep Nix runner generation local when the developer profile uses BuildBuddy.
- Serialize the redb durability fault-injection test binary so its process-wide
  test hooks cannot race under CI.
- Allow the cold realized Nix check to use Bazel's eternal timeout instead of
  failing after the standard 15-minute test budget.
- Keep resource-compiler test schema discovery runtime-based so strict Rust
  path checks remain valid on remote workers.
- Wire the v3 Cloud Hypervisor and Volume provider crates into d2bd's Bazel
  production and test-support closures.
- Keep broker runtime tests on OS temporary storage so remote Rust artifacts do
  not embed the BuildBuddy execroot.
- Recompute v3 production-closure lock authorities after the workspace merge
  and build Nix host tools from the root Cargo workspace.
- Keep capability seals, timeout-sensitive transport coverage, and shard failures reliable and visible under loaded validation runs.
- Keep trusted CI compatible with the root Rust toolchain authority while the
  protected workflow still reads the former package-local path.
- Remove the retired `rust-local` CI shard, make broker profile tests
  runner-UID independent, and use native Bash for Bazel actions on FHS hosts.
- Give cold realized Nix and advisory performance jobs sufficient hosted-runner
  time, and keep the async blocking-adapter heartbeat below its backend delay
  without assuming sub-5-ms scheduler jitter.
- Routed Guest daemon and broker package selection through the injected
  `d2bHostToolOverrides` map so host-integration VM evaluations do not fall
  back to Nix source packages for those d2b binaries.
- Staged the Cloud Hypervisor Provider controller from Bazel separately from
  the eight host tools so VM acceptance no longer recompiles it through Nix.
- Prevent controller session and Process watch work from blocking daemon
  startup or Resource commits, and preserve exact running Process identity
  across status-only revision changes.
- Make eligible Resource deletion and final-finalizer cleanup complete through
  foreground store garbage collection.
- Keep inherited controller-session descriptors and bootstrap readiness probes
  compatible with the static Guest build's safe-Rust policy.
- Bound Cloud Hypervisor runner ACL grants to the broker-owned state root and
  avoid mutating world-traversable ancestors.
- Qualify Guest store-view intents, runner identities, and cgroup leaves by
  Zone so same-named Guests remain isolated.
- Credential service handlers now await provider operations without nested
  Tokio runtimes, and Secret Service User scope is carried per authenticated
  Credential request instead of the controller bootstrap. Guest credential
  delivery accepts only injected Guest-local source ports and fails closed when
  those endpoints are absent; test-only lease registries are no longer used by
  production composition.
- Route Credential Provider backends through Guest-local typed ports with
  session-bound Noise_KK transport, zeroizing response buffers, and supervised
  minijail bootstrap for managed-identity agent Processes.
- Stabilized Bazel external-seal fixtures by binding nested Cargo checks to
  the Bazel Rust toolchain, including its standard library and unique scratch
  ownership, instead of ambient compiler state.
- Materialized large Bazel fixture source inventories through a manifest
  instead of action arguments, avoiding local `ARG_MAX` failures.
- Start independent PR Layer-1 test suites concurrently instead of waiting for tier0.
- `d2bd-runtime`'s startup store-retry test no longer races the transient
  retry backoff against the wall clock.
  `startup_store_retry_uses_delayed_async_backoff` now asserts the 5 ms and
  20 ms retry delays against a paused Tokio clock that only the test advances,
  so `make check` stops failing on this test while the host is under load. The
  assertions are unchanged. The `tokio` `test-util` feature the paused clock
  needs is declared as a dev-dependency, so production builds resolve the same
  feature set as before.
- CI dependency installation now avoids the throttled Azure Ubuntu mirror with
  bounded HTTPS APT fetches while retaining normal repository metadata
  authentication.
- Fixed the daemon resource plane wedging at startup under the
  `runtime-cloud-hypervisor-guest-preflight` host-integration lane: controllers
  now reach `Ready`, the VMM process is created, and all runner-death classes
  are eliminated. Runner processes respawn on transient source failures
  (including session-establish and controller-identity races) instead of dying,
  and bootstrap re-arms on transient establish/setup failures.
- Fixed effect-claim lifecycle to survive requeue and writer stalls: the claim
  digest is revision-bound and the post-commit record is hardened against
  stale-owner adoption.
- Fixed store error classification so watch-open and writer-stall closures are
  treated as transient rather than integrity failures, and lagging self-status
  writes are requeued (convergence-gated) instead of suppressed.
- Fixed startup publication races by retrying process-resource start (60 s
  budget) and publishing `Ready` status for alive controllers during observe.
- Provisioned dedicated d2bd-owned Zone audit and telemetry directories beneath
  broker-owned Zone storage parents, preserving the parent ownership boundary
  while allowing the production resource plane to open its durable sinks.
- Fail Nix surface evaluation when a named case selection is empty or names cases that do not exist.
- Stop authenticated Relay display rendezvous pumps before listener shutdown,
  cancellation, or reconnect so successive local socket sessions cannot
  overlap.
- Make `test-lint` fail closed when its required shellcheck tool is unavailable and provision shellcheck through the pinned Bazel Nix environment.
- Run the Nix-unit suite once per pull request while keeping it part of the
  stable Layer-1 aggregate and its documented local Make coverage.
- Include v3 provider schemas in isolated Nix fixture sources so profile-capable
  host tools compile crates that embed their schemas at build time.
- The ProviderSupervisor production-adapter test failed spuriously on loaded
  hosts: it asserted the single worst wall-clock heartbeat tick, which the host
  scheduler can delay while the executor cadence stays intact. It now bounds
  the heartbeat cadence distribution and additionally requires every blocking
  backend call to run off the async executor thread.
- Made Cloud Hypervisor host acceptance fail fast with bounded, redacted resource diagnostics and scoped controller/session readiness checks.
- Required host restart acceptance to observe a newly authenticated Guest session generation bound to the tested Guest identity.
- Ordered live-session validation before post-restart Resource reads and enabled the Guest debug event at its crate-root target.
- Repair host-integration fixtures so the guest broker owns the `d2bd` identity
  and minimal daemon hosts emit a valid local-root topology.
- Bind controller-owned Cloud Hypervisor children to trusted runner intents,
  publish evaluated Zone/Guest closure metadata for StoreSync, and align the
  artifact-bound `nix-closure` Volume contract.
- Generated one canonical reviewer request for every staged review, binding
  the incremental and full diff ranges, validation evidence, prior verdicts,
  finding threshold, and no-rerun instructions into the prompt integrators
  dispatch verbatim.
- Keep the Bazel policy aggregate limited to source hygiene, workspace and
  lock integrity, supply chain, and changelog policy while retaining
  owner-local Rust and schema checks.
- Drivers can prove a child or dependency Ready: readiness is
  `get_view(key).observed_status() == Some(Ready)`, `Ok(None)` means absent
  (no row), and a row with nothing published means unknown - never "not
  ready" and never a fabricated `Pending`. This is the surface the volume
  leg recorded for KTD6 and the U8 child-set readiness lanes were blocked
  on; `WatchCondition::Ready` remains the readiness wake-up.
- Preserve the durable accepted effect operation identity through Runner
  persistence-only retries, including failures after the status write, so
  transient watch operation IDs cannot kill the runner or create duplicate
  ledger effects.
- Keep private Provider package, catalog, and schema paths in explicit
  resource-compiler closures while preventing realized bundle data from
  carrying Nix store context into host configuration evaluation.
- Exercise a non-empty Provider compiler closure in the Nix-unit bundle case
  instead of relying solely on source-shape checks.
- Keep transient startup and watch work IDs out of no-effect failure status
  persistence, while retaining accepted effect IDs through Runner retries.
- Recover ambiguous Store commit responses with an exact-target fresh read and
  idempotent operation replay so one resource cannot kill its shared Runner or
  duplicate status/effect work.
- Bound ambiguous Store commit recovery and Runner persistence retries so
  persistent timeout or backpressure finishes the affected resource as
  uncertain without rerunning an accepted effect or starving siblings.
- Resolve Nix closure Guest store-view Volumes through the exact Zone and
  Guest bundle intent and broker StoreSync path.
- Keep each Guest system Volume on its declared closure root instead of
  claiming the shared store-view farm.
- Process Provider controllers now validate launch tickets and exact process identity before readiness, adoption, and stop effects on Host and Guest targets.
- Regenerated the signed acceptance Provider manifest for the target-local controller contract.
- Admit Binding Endpoint children by validating their reserved `providerRef`
  with the canonical full `ResourceSpec`.
- Run the AudioBinding Guest worker as a system-domain Process without an
  unbound `userRef`, allowing the systemd Process Provider to converge it.
- Derive Binding child deletion readiness only after deletion is requested, so
  live children with no finalizers remain valid during relist.
- Kept the controller reaction benchmark on its hermetic in-memory path until
  an authenticated Resource-API write route is available, so it cannot bypass
  store-owned mutation admission.
- Preserve the development-shell PATH for local Bazel test runners without
  overriding the worker-standard action PATH used by remote profiles.
- Allocate client-initiated ttrpc correlation IDs from the valid odd-ID
  range, and keep package and manifest digests outside signed Provider
  manifests so artifact verification remains non-self-referential.
- Keep acceptance-only Provider packages under `d2b.artifacts` without
  publishing non-matrix identities through `d2b.providerCatalog`.
- Reject operation admission and rebinding from authenticated Provider route
  snapshots whose owning ComponentSession is no longer live.
- Keep named-stream data and terminal events on their owning
  ComponentSession stream when multiple streams share one session, including
  cancelled receives and local resets.
- Observe exact Process and EphemeralProcess identities after Runner relists,
  persist terminal exits, preserve restart and TTL decisions across passes,
  wait for Provider-owned OneShot DAG nodes, and durably fence Guest effect
  acceptance across reconnects.
- Keep liveness observations mutation-only, wake exact owners from retained
  pidfd authority, schedule persisted restart delays in the shared Runner, and
  fence each restart launch with a fresh lifecycle effect identity.
- Wired the network Nix-unit surface to the retained `net-vm-network`
  cases and evaluated `net.nix` as a guest module so the net VM
  `10-eth-dhcp` `mkForce` sentinel-MAC neutralizer is actually asserted.
- Fail closed when a Nix-unit surface evaluates zero cases instead of
  reporting success.
- Stabilized the pinned rules_rs Rust toolchain fetch by redirecting its x86_64 zlib package to the immutable Launchpad librarian archive while preserving the upstream SHA256.
- Revoked stale controller assignments on disconnect or replacement and retried stale lease revocations during refresh.
- Preserved owner generation and existing siblings during Binding child creation and uncertain batch recovery.
- Reused Bazel's pinned nixpkgs input across isolated Nix unit surfaces instead of resolving the Git input during every test.
- Kept routine flake checks eval-only for package exposure and verified exact Guest static package selection through lightweight evidence.
- Materialized contract fixtures with inert host-tool packages so routine tests no longer rebuild d2bd and broker services through Nix.
- Restore ACA provider error diagnostics and circuit-breaker behavior,
  including bounded retries and single half-open probes.
- Waited for the shared controller-session guard during reconciliation so a
  wake that races system-core policy refresh cannot be lost before Provider
  bootstrap completes.
- Serialized system-core policy-session refresh with external Provider
  controller admission so pending bootstrap endpoints are retained and
  authenticated Cloud Hypervisor sessions remain available during public
  Resource reads.
- Refreshed missing Process Provider identities from the authoritative Zone
  store before launching late-created VMM and virtiofsd workers.
- Read controller Provider identities and their revision from one atomic Zone
  store snapshot so concurrent status commits do not abort startup admission.
- Serialized every Zone policy projection and carried the exact external
  Provider subjects through public refreshes and ZoneBus installation so
  controller session admission cannot be clobbered mid-handshake.
- Kept public Get/List reads on the installed policy projection, preserved the
  last-known-good projection across refresh and preflight failures, and
  requeued failed controller-session policy installs without losing their wake.
- Preserved the installed projection across retryable system-core rebind
  failures, bounded controller-session retries without self-notify hot loops,
  and fenced live sessions whose bootstrap context was replaced.
- Kept rebind-pending state retryable across unchanged policy snapshots,
  rejected empty or partial post-fence session slots, and restored the fenced
  system-core and Provider runners after recovery.
- Woke the controller-session coordinator when a launch or adoption records a
  readable bootstrap endpoint, and made initial test-controller handshake
  failure terminal so one-shot bootstrap delivery cannot queue duplicates.
- Froze each controller-session admission pass to one bootstrap-context
  snapshot so late or replaced Pending controllers stay queued for the next
  non-lossy wake instead of entering a policy compiled without their subject.
- Public Network reconciliation now runs the ordered Network Provider
  controller before Guest launch, including its host-effect and child-readiness
  barriers.
- Public resource reconciliation preserves typed Get failures such as
  not-found and resource-plane-unavailable.
- Wayland version filters reject zero-valued protocol versions.
- TPM migration accepts trusted setgid-inherited file groups while rejecting
  unsafe parent directories.
- The rustdoc and compiler caches used by the capability-seal tests are keyed
  on the toolchain. Reusing a tree produced by a different rustc version could
  fail a render or leave stale capability-surface evidence.
- The mint-surface guard discards the rendered documentation of packages whose
  library and binary targets share one output directory. Against a warm tree
  Cargo re-runs only the target it considers dirty and overwrites that
  directory, dropping exactly the private items the guard checks.
- Continuous integration caches the nix store, the guest-shell-runner cargo
  target directory, and the no-bash-ast-walker target directory. None of the
  three were cached, so every run rebuilt them from source.
- Isolate unsafe-local shell supervisor tests from host login profiles so
  Layer-1 Rust checks do not flake on user startup hooks.
- Require the public Network reconciliation path to commit and re-read a
  typed child-readiness projection before publishing durable Ready or
  launching a dependent Guest.
- Preserve typed status resource projections while updating universal phase
  fields, and classify pre-upgrade restore backups as upgrade-required.
- Compare broker runner executables by their canonical targets so trusted
  NixOS symlink paths remain adoptable and liveness probes do not reject live
  runners.
- Preserve broker-owned runner adoption across UID transitions without
  CAP_SYS_PTRACE and resolve Cloud Hypervisor's wire role to its trusted
  bundle intent.
- Bound broker-audit evidence replay pages so restart recovery stays within
  the broker transport frame limit.
- Give broker-spawned swtpm runners separate control and Cloud Hypervisor
  server sockets so TPM-backed Guests can boot.
- Carry each VM's explicit autostart policy into the public manifest so
  d2bd does not boot workloads declared `autostart = false`.
- Keep runtime-ledger wall-time diagnostics advisory while enforcing hermetic placement and aggregate process-CPU budgets.
- Redact CTAPHID report bytes from frontend Debug output while retaining privileged audit fail-closed and quarantine evidence.
- Guest disconnect marks target-dependent observed state unavailable without
  deleting or moving desired resources, and reconnect re-binds the affected
  assignments through target-local discovery and adoption instead of
  inheriting the lost session's authority (F5, R18, R21).
- Require a configured non-root workload user for persistent Guest shell
  service wiring and keep ComponentSession evaluation aligned with the active
  `d2b.sshUser` field.
- Keep the editable Guest configuration in its dedicated workload-owned
  directory while granting `d2bd` group access to serve it.
- Fail non-qemu USBIP attach, detach, and VM-start reconciliation closed with
  typed `runtime-capability-unsupported` when the target-local
  `usbip-guest-proxy` Process and ComponentSession path is unavailable. Host
  bind, proxy reconciliation, and claim release are no longer reported or
  performed as successful USB attachment work; VM-stop cleanup preserves the
  claim until Guest detach is available.
- Repaired the Venus Vulkan Video lab prototype's GPU-copy presentation
  fallback, which rendered the video area green because the chroma plane was
  never copied. A blit does not go through a sampler view, so it did not reach
  the per-plane images that the preferred zero-copy route resolves through its
  sampler views, and it could not be made to: the hardware copy call derives
  formats from the texture objects and requires a shared texel size class, while
  a texture bound from an imported image reports no internal format at all.
  Sampling has no such requirement, so the per-plane blit now takes the shader
  path, joining the colour-space-conversion case already excluded from the copy
  fast path for a related reason. Blit failures fall to zero and the picture is
  correct.
- The zero-copy route remains the preferred and selected one, and is unaffected:
  it issues no blits, so the repaired path is never reached from it. Re-measured
  after the change with no failures on any surface and a correct picture.
- Corrected a measurement in the Venus Vulkan Video lab prototype that ruled out
  the obvious repair for its GPU-copy presentation path. The record stated that
  forcing that path leaves the second image plane unimported, so nothing existed
  for the copy to find. Re-measuring shows the plane is imported. The two runs
  reached the same path by different mechanisms, one by removing a graphics
  capability preference and one by turning the surface preference off, and only
  the first suppresses the separate import. The repair is therefore unresolved
  rather than ruled out. The re-measurement also confirms the failure is no
  longer catastrophic: the copy still fails, but it no longer poisons the
  rendering context or causes submissions to be refused, and playback continues.
  The prototype is unaffected either way, since it uses the zero-copy path.
- Corrected lab findings documents that described the green-frame presentation
  defect in the present tense after it had been root-caused and fixed, and that
  described the GPU-copy path as the intended target when the working
  configuration selects zero copy.
- Corrected lab flake comments that stated the opposite of the preferences
  declared beneath them, including one claiming direct export was enabled while
  the preference set it to false.
- Named the exact blocker behind the Venus Vulkan Video lab prototype's guest
  VA-API decode failure, rather than leaving it as upstream work of unknown
  size. The host driver rejects the decode at its hardware entry point with an
  invalid-value error because virglrenderer supplies no per-slice decode
  parameters, and it supplies none because the virgl video wire format carries
  only a slice count and no per-slice size, offset, type, or first-macroblock
  field. Drivers that re-parse the bitstream themselves do not need those
  fields, which is why the format never carried them and why the restriction to
  those drivers is accurate rather than merely cautious. Lifting it requires
  extending the wire format across guest and host for every codec, which is a
  protocol change and is not needed by this prototype, whose decode path is
  Vulkan Video.
- Corrected an overstated claim in the Venus Vulkan Video lab prototype. Enabling
  the virgl video backend makes the guest advertise H.264 through VA-API, and
  that is what Firefox's capability probe reads, but the advertisement was
  described as a proven capability without the measurement that would establish
  it. A guest VA-API decode has since been measured against the same decode on
  the host: the host engaged the hardware decoder at 94 to 98 percent while the
  guest engaged it at zero percent on every sample, running well faster than the
  hardware itself. Removing the preference therefore replaces a bypassed probe
  with an unverified advertisement rather than a demonstrated capability. What
  decodes is unchanged and unaffected: Firefox decodes through Vulkan Video,
  which is measured as hardware backed, and never through VA-API.
- Located where the Venus Vulkan Video lab prototype's guest VA-API decode
  terminates, closing a question the previous entry left open. Counting the
  virgl decode path the way the Venus path was already counted shows the guest
  sends decode commands and every buffer-creation and render call on the host
  succeeds, after which the call that commits the decode is rejected by the
  host driver for every frame with a decoding error. That accounts for the idle
  hardware decoder, the impossibly fast throughput, and the absence of any error
  visible to the guest.
- Reclassified virglrenderer's Mesa-only video driver restriction from
  conservative to load bearing. Exporting a decoded surface uses a standard
  interface the host driver implements, which is why the restriction looked
  removable, but consuming the renderer's picture parameters and slice data is
  the part that fails. The lab's override is retained as an investigation tool
  with a warning naming the failing call and status, not as a capability.

### Security

- Bound expedited commit evidence to the target Zone before effect permits are
  minted, preventing matching evidence from another Zone from authorizing a
  reconcile effect.
- Added `labs/wlattach`, an experimental crate exploring reconnectable Wayland
  application forwarding: a persistent session host keeps an application alive
  while a disposable window frontend can be detached and re-attached, so a real
  desktop application survives the death of the process showing its window and
  gets that window back on a fresh compositor connection. It has its own Cargo
  workspace under `labs/`, is outside CI, and changes no shipping component.
- Added ADR 0047, replacing the Wayland proxy's fixed-width identity rail with
  a reserved band containing a single accessible identity tab. The tab renders
  horizontal text at a real target size, derives its layout and its pointer
  handling from one measured list of parts, discloses labelled per-workload
  actions in two tiers, selects label colour by WCAG contrast ratio rather than
  a brightness threshold, sanitizes identity text against bidirectional
  overrides, and reports a typed failure instead of ever leaving a proxied
  window unlabelled. Adds the `proofs/window-identity-chrome/` proof for the
  geometry, parts, contrast, and label logic.
- Require both the Network resource and site-level east-west opt-ins before
  direct workload forwarding can be enabled.
- Consume sealed lifecycle authorization at Provider admission and at the
  broker executor, while keeping HostShutdown strictly stop-only.
- Mint a fresh lifecycle operation identity for every attempt and require
  current authorization-bound runner snapshots before adoption, including TPM
  runners.
- Broker-local resource bindings now fence Zone and resource identity with
  immutable UIDs, generations, revisions, and provider references. Audit
  correlation uses digest-only keys, and credential or host-path fields are
  rejected.
- Secret Service User scope is carried as a separate authenticated claim from
  the Provider controller subject, while Zone, Process, Provider, User, and
  session generations remain independently fenced.
- Credential delivery remains bound to exact Zone, Guest, consumer, operation,
  and reconnect evidence; ambient cloud SDK credential-chain names are rejected,
  lease and delivery diagnostics stay redacted, and sensitive buffers are
  explicitly zeroized.


- Managed-identity reconciliation now creates and observes the exact
  `mi-agent-<credential>` Process child through the Resource API, requires
  current Ready status before projecting Credential readiness, and drains
  owner-matched Process children before releasing the Credential finalizer.
  Owner-child mutations reuse the parent assignment fence, and Azure Relay
  Guest composition accepts an authenticated scoped Credential client without
  taking transport ownership of Resource rows or credential registries.
- Provider-filtered runners are now attached for all three providers before
  any Credential row exists, while standalone provider binaries refuse
  unauthenticated or ambient-chain startup instead of reporting readiness.
- U10 runners now receive a reconnect-aware typed ComponentSession Credential
  adapter from daemon composition; confirmed and uncertain revocation evidence
  remains durable and provider service entrypoints expose the supervised typed
  Credential session lifecycle.
- Credential Provider controller sessions now use the dedicated typed
  `d2b.credential.v3` endpoint, and the service runtime drains registrations
  before reporting shutdown.
- Credential Provider Process children now use the inherited fd 10 supervisor
  handoff, authenticated route bootstrap, typed serving loop, readiness gate,
  and shutdown drain; revocation metadata is selected from the live Provider
  session route and rebinds safely across reconnect generations.
- Provider controller subjects remain Provider identities while Secret Service
  User placement is carried as a separate authenticated scope claim. Missing,
  malformed, cross-domain, or mismatched scope claims fail closed without Host
  credential fallback.
- Secret Service, Entra, and managed-identity backend requests are route- and
  operation-checked, use the enrolled Noise_KK policy, and fail closed as
  unavailable when the Guest-local source cannot answer. Sensitive response
  buffers and key handoff objects remain zeroizing and no Host backend peer is
  retained.
- Secret Service subprocess and fd11 backend routes keep Provider identity,
  User scope, Zone, Process, and generation fences separate and fail closed
  when the User claim is missing or malformed.
- ADR 0046 release automation: the host-binary release workflow now fails closed
  when unfolded fragments remain under `changelog.d/`, so a release can no
  longer omit changelog entries.
- ADR 0046 ZoneLink bootstrap: closed the documentation gap that let an
  implementer read ZoneLink sessions as all-KK and skip IKpsk2 enrollment, so the
  initial cross-Zone handshake and its `bootstrap-ikpsk2` evidence can no longer be
  bypassed by following a stale spec statement.
- ADR 0046 live-test docs: the documented live-test path now acquires the
  heavy-gate semaphore rather than instructing operators to run the live
  entrypoints ungated.
- The runtime ledger validates a short closed runner-label grammar, bounds
  printable test identifiers, row counts, and libtest input size, and rejects
  control characters both when emitting and when loading ledgers, so host
  paths, multi-line log injection, and unbounded artifact cardinality
  can no longer reach the recorded or printed output.
- ADR 0046 ZoneLink: specified the crash-safe
  `Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready` session state
  machine on the owning ZoneLink handler and every mirror, replacing
  the prior generic `Pending/Established/Disconnected/Reconnecting/Revoked`
  model that had no enrollment or PSK-consumption semantics. Resource traffic is
  now prohibited before the enrolled KK session reaches Ready; each PSK consume,
  enrollment persist, and bootstrap teardown crash window has a defined recovery;
  and revocation invalidates both the sealed enrollment record and the active
  session, requiring a fresh single-use PSK and a new IKpsk2 enrollment before
  reconnect. An implementer can no longer retain a revoked enrollment or bypass
  bootstrap evidence by following the stale state machine.
- ADR 0046 threat model: replaced the `spec.childStaticKeyFingerprint`
  reconnect trust anchor, which contradicted the six-field ZoneLink schema, with
  the private sealed enrollment-record and child key-pin authority bound to the
  child Zone uid and the allocator enrollment, and updated the detection and
  reconnect-validation text to match. The schema no longer implies an illegal
  seventh field or an omitted key-pin check.
- Every shell and Make heavy-lane guard now proves a genuinely held slot before
  running heavy work instead of trusting the presence of the `D2B_HEAVY_GATE`
  environment variable. The live lanes, the hardware smoke, the enforcing
  performance-budget path, the aggregating runner, the layer dispatcher, and
  the `heavy-lane-guard` Make target all call `heavy-gate verify-slot` (via a
  shared self-guard helper) and re-acquire a real slot when it fails. The
  performance advisory skip returns before the guard because it does no heavy
  work. Exporting `D2B_HEAVY_GATE` alone no longer bypasses the sole-use
  semaphore - the guard detects the unverified marker and acquires a real slot
  rather than running raw heavy work.
- The heavy-entrypoint inventory guard is now closed-world. It walks the live,
  hardware, benchmark, and cloud directories recursively (catching nested and
  non-`.sh` executable entrypoints), requires an executable self-guard on the
  performance-budgets canary, and parses the Makefile so that every
  `heavy-lane-*` work target must both depend on the guard and be reachable only
  through a public gate-acquiring delegation. Adding a new heavy entrypoint now
  fails the guard until it is gated. For `tests/static.sh`'s direct performance
  invocation, the advisory path performs no heavy work and the enforcing path
  self-gates.
- The heavy-gate semaphore now uses the single protected
  `/run/d2b-heavy-gates/uid-<uid>/` namespace. Its root and per-uid directory
  are root-owned and non-writable by unprivileged users, and its two slot
  files are pre-created for the target uid at mode `0600`. It never honours
  `XDG_RUNTIME_DIR` or `TMPDIR` and has no fallback, so neither a foreign uid
  squatting a temporary name nor the invoking uid renaming a user-owned
  namespace can deny service or obtain an independent two-slot pool.
- The container integration lane now holds a heavy-gate slot. The aggregating
  `tests/test-integration.sh` runner and the standalone
  `tests/integration/containers/ubuntu-host-check.sh` entrypoint both prove a
  genuinely held slot before doing any podman or Nix work, closing a lane that
  previously ran entirely unsynchronised against the shared Nix store and cargo
  target.
- The heavy-entrypoint inventory guard is now closed-world across every heavy
  lane. It classifies each lane directory as gated or explicitly out of scope
  and fails when a lane is neither, so a new heavy lane cannot appear
  unsynchronised without being classified. Sourced support libraries (for
  example `tests/integration/containers/lib.sh`) are distinguished from runnable
  entrypoints and are not required to hold a slot.
- Delivery-state evidence reads, directory listings, and writes now all resolve
  fd-relative from the verified root on the same inode chain, matching the
  hardened write path. Reads and listings are no longer path-based
  check-then-open, so an attacker who controls a writable ancestor can no longer
  swap trees during the read phase and seal forged evidence into legitimate
  state.
- Diagnostics from the delivery workflow, heavy-gate slot verification, and the
  wave-snapshot Git path now name components by semantic role and
  repository-relative key only. They no longer interpolate absolute host paths
  (including `HOME` and the per-user runtime directory), the caller's numeric
  uid, or raw Git subprocess stderr, so an error surfaced to operator stderr or
  a CI log no longer discloses host filesystem layout or user identity.
  Negative-output tests now assert that a forced failure in each of these
  surfaces emits no absolute path and no uid.
- The heavy-entrypoint inventory guard is now closed-world with no free-text
  escape hatch. Every heavy lane, including the nightly distribution-matrix
  lane, is classified by a checked property (whether any script in it performs
  build, container, VM, privilege-elevation, or device work) rather than by an
  assertion in a comment, and the guard also classifies files directly under a
  lane parent and distinguishes each file as a runnable entrypoint or a sourced
  library per file rather than by basename. A genuinely heavy lane can no
  longer be exempted by wording, and a lane cannot escape classification by
  sitting at the top of a lane directory or by being named like a library.
- The heavy-gate semaphore now resolves a single stable per-uid location that
  does not depend on whether a runtime directory exists, and fails closed when
  its shared parent is not in a safe shape. Two lanes started on either side of
  the runtime directory being created or removed can no longer land in two
  independent slot pools, and a same-uid actor can no longer rename the shared
  parent out from under lanes that hold locks to make later invocations create
  a fresh, independent pool.
- Delivery-state candidate directories are now pinned once and every operation
  goes through the retained directory descriptors for the whole invocation.
  Snapshot and seal reads, evidence traversal, and the final write share one
  pinned chain, and the candidate address is derived from the validated
  state-relative reference rather than a supplied path, so a same-uid actor can
  no longer present a forged tree for the reads, restore the legitimate tree
  before the write, and land a seal derived from forged evidence in the
  legitimate candidate.
- The heavy-gate self-guard helper no longer trusts its build environment. It
  builds and runs the wrapper only from this checkout's own target directory,
  ignoring a caller-supplied target directory; it strips build-affecting Cargo
  and Rust environment variables before building; and it reports only a
  bounded, path-free label and an exit status on failure instead of forwarding
  the build tool's output. A hostile continuous-integration environment can no
  longer point the target at a planted binary whose slot check returns success,
  and a build failure no longer discloses the checkout location or a
  username-bearing path.
- Absolute filesystem paths no longer reach the runtime-ledger, the
  delivery-evidence, or the heavy-gate diagnostics. Failure messages name the
  artifact role, or the offending file's leaf, instead of the absolute path a
  caller supplied, so the checkout layout and any username-bearing directory no
  longer leak into operator output or continuous-integration logs.
- The heavy-gate semaphore now requires a root-provisioned runtime namespace
  whose directories cannot be renamed by either a peer or the invoking user,
  fails closed with no weaker fallback when that namespace is unavailable, and
  performs every slot operation relative to pinned directory descriptors.
  Self-guard regression tests also start from an empty environment and close
  inherited descriptors, so parent gate state cannot silently authorize a test
  child.
- The heavy-entrypoint census now accepts source relationships only from
  executable regular shell entrypoints. Inert sibling text can no longer claim
  to source a heavy script and hide that script from the self-guard check.
- The NixOS module now provisions the protected heavy-gate root and private
  slots for configured lifecycle users that NSS can resolve during activation.
  A named Make target provisions deferred network-backed users after login and
  provides the same setup for other development hosts. Missing provisioning
  reports a stable observed-state diagnostic with the exact remediation.
- The adapter's Provider-facing diagnostics and status omit process identifiers, descriptors, unit and cgroup identity, paths, arguments, environment, and numeric user identity. Existing broker audit and journal fields still require separate hardening.
- Contract and Nix validation refuse physical-NIC bridge multiplexing across Zones before any host effect; executable external-NIC host integration remains pending.
- Service contracts keep token, signature, lease source, and Credential identity material out of outer DTOs and diagnostics, with explicit zeroization for plaintext delivery records; these guarantees are currently exercised only in hermetic service tests.
- The live broker handlers preserve sibling and foreign firewall rules and fail closed on ownership-marker conflicts, while Provider-level cross-Zone and lifecycle behavior remains proven only through hermetic admission tests.
- Provider contracts keep credential bytes behind injected secret-holding clients and prevent Providers from selecting the consumer, audience, route, or delivery limit. Production authenticated delivery sessions and telemetry sinks are not wired yet.
- Controller and broker policy reject stale generations and foreign ownership markers while preserving sibling Network, device-owned, and foreign firewall state; executable container and mDNS lifecycle coverage remains pending.
- Enforced Provider integration package layout through the existing policy lane and added hermetic checks for bounded, redacted Volume status shapes; this does not make Provider state production-reachable.
- The new admission contracts scope USBIP authority to its declared Zone and refuse sharing one bridged physical NIC across Zones; they do not claim a newly production-wired USBIP lifecycle.
- Added hermetic coverage for ownership-scoped bridge, persistent TAP, and firewall lifecycle rules that preserve foreign state and reject stale generations or foreign markers; executable container and host integration remains pending.
- Added hermetic regression coverage for local Volume state policy, including missing or replaced roots, ordered migration and relocation decisions, status-first sealing, and ambiguous snapshot retention. A production Volume effect adapter and real filesystem scenarios are still absent.
- Added broker-owned Zone store opening with signed opaque-row validation,
  anchored marker and inode checks, and exactly one close-on-exec database
  descriptor handoff.
- Provider configuration and Device extension settings now fail closed when
  secret-shaped keys or values would be embedded in the resource bundle.
- Sealed route-admission evidence is issued only from runtime-owned identity
  and clock state, rejects forged claims, and fails closed on stale or revoked
  ZoneLink generations.
- Route admissions now require current ready-session and cursor ownership,
  bind the committed ZoneLink identity and policy, and reject stale,
  revoked, conflicting, aborted, or restarted state before evidence issuance.
- Committed route operation IDs are durably retained without eviction and
  recover only for their exact ZoneLink identity and controller generation.
- Gateway Guest Relay credentials and sealing material now use zeroizing
  storage, transactional generation fencing, exact acquire/revoke tracking,
  bounded lease cardinality, and redacted transport errors without exposing
  secret bytes to diagnostics.
- Rejected credential material, raw locators, and host fallback placement in
  gateway Provider and Guest settings while keeping relay credentials as
  Guest-scoped ResourceRefs.
- Route admission verifies exact ZoneLink identity, topology edge, controller and reconnect generations, Zone identities, immutable operation, capability, policy revision, session profile, and bounded expiry at current route use, with single-use invalidation after consumption.
- Host d2bd no longer loads legacy gateway configuration or credential paths; Relay credential custody remains inside the Gateway Guest.
- Legacy realm identity fields are rejected by the new Zone contract, and
  stale Zone or resource UID and generation tuples cannot match.
- Fenced broker-backed Process launch, adoption, cgroup placement, pidfd control, and audit data to the exact resource incarnation and private Zone runtime scope.
- Preserve incomplete executable observations for exact replacement and retain
  registered pidfd and metadata authority until targeted reap completes.
- Added fail-closed source-policy, store-view, ACL, and virtiofs sandbox
  validation without exposing host paths or filesystem descriptors.
- Lifecycle identity mismatches are isolated instead of superseding another Guest generation, and Gateway composition retains relay credentials and Provider effects inside the Gateway Guest.
- Host shutdown remains a distinct stop-only capability.
- Route admission revalidation fences stale or revoked lanes, while
  purpose, role, service, target, Zone, peer, and generation substitutions
  fail closed before driver traffic is admitted.

## [1.4.1] - 2026-07-12

### Added

- Added ADR 0045, defining parent-owned workload-hosted realm
  controllers; explicit runtime, infrastructure, transport, substrate,
  credential, and display provider responsibilities; type-first sortable
  provider crates; generic Unix/vsock/direct/Azure-Relay byte transports;
  Noise-authenticated component sessions with ttrpc/protobuf control services;
  Entra and YubiKey credential placement; and policy-authorized peer shortcuts
  over inherited shared transport fabrics.

### Fixed

- Fixed unsafe-local persistent shells inheriting `TERM=dumb` from the systemd
  user manager by supplying a fixed true-color terminal baseline while
  preserving the rest of the manager environment and login-shell startup.
- Made the mkfs diagnostic bound test exercise the formatter directly instead
  of depending on unrelated existing-image repair stages.
- Made output-ring wake coverage observe data and EOF as separate valid
  notifications instead of racing both producer events into one read.
- Made disk-init test directories use exclusive process-local IDs so parallel
  tests cannot silently share and remove each other's scratch state.
- Made failed-fd-send coverage track the original pipe identity so concurrent
  numeric fd reuse cannot produce a false leak report.
- Stabilized shell-supervisor teardown coverage by allowing its asynchronous
  socket cleanup the same bounded reconciliation horizon used by the runtime,
  waking the supervisor accept loop so its owned listener unlinks before forced
  scope teardown, and ensuring a missing/replaced control socket cannot block
  verified scope collection or ledger cleanup.

- Fixed the provider-neutral `launch` command missing from the public
  authorization matrix and generated privileges schema. Configured launches
  remain scoped per workload/realm to launcher or admin callers, audited, and
  broker-free.

## [1.4.0] - 2026-07-12

### Added

- Added the realm-native control plane under `d2b.realms.<realm>`, including
  canonical `<workload>.<realm-path>.d2b` targets, provider-neutral workload
  identity, realm network and UI metadata, generated realm artifacts, bounded
  realm/operation inspection commands, and metadata-only topology, access, and
  resource-allocation layers.
- Added the explicit, default-denied `unsafe-local` provider for host-user
  workloads. Generic typed `exec` and `shell` launcher items now work across
  local VMs, qemu-media, and unsafe-local targets through `d2b launch`, with
  persistent host shells, same-UID helper supervision, Wayland identity rails,
  and visible no-isolation posture.
- Added daemon-owned serial-console and audio operations, including
  provider-capability dispatch and host/guest mute and volume controls.
- Added an opt-in FIDO2/WebAuthn security-key proxy that presents a host device
  to opted-in guests as virtual HID over vsock without transferring USB
  ownership, plus status, session, cancellation, test, and notification
  commands.
- Added explicit USB attachment for any physically present device to an
  eligible VM, with preflight validation, audited ownership, and rollback.
- Added the opt-in `d2b-clipd` clipboard authority, picker-driven cross-realm
  paste, virtualized guest clipboard transport, and Niri focused-window
  attribution.
- Added a compositor-agnostic UI color contract rendered as
  `/etc/d2b/ui-colors.json` and `/etc/d2b/ui-colors.css`, with a Niri VM-border
  backend and per-realm accent metadata.
- Added macvtap-backed external network attachment for env net VMs, including
  independent egress NAT, port forwards, and mDNS/`.local` reflection.
- Added generated storage and synchronization contracts, read-only startup
  validation, and `d2b host doctor --read-only`.
- Added provider-aware graceful VM shutdown with configurable global and per-VM
  timeouts, plus explicit `--force` lifecycle overrides.
- Added experimental remote full-host and provider-managed Azure Container Apps
  adapters with capability matrices, bounded backoff/circuit behavior, and
  redacted diagnostics. Production remote transport remains out of scope.
- Added release automation that creates a version tag and GitHub release with
  host binaries, checksums, and a Nix hash manifest when a dated changelog
  section reaches `main`.

### Changed

- **Breaking:** Renamed the project to **d2b: Double Dutch Bus**. Commands,
  packages, services, sockets, Nix options, paths, schemas, and telemetry now
  use only `d2b` naming; no legacy aliases are provided.
- **Breaking:** Removed the legacy `d2b.gateways` and nested gateway/ACA sandbox
  configuration surfaces. Configurations must migrate to `d2b.realms`.
- **Breaking:** Explicit `d2b://` targets must include the reserved `.d2b`
  suffix; omitted suffixes no longer fall back to local VM routing.
- **Breaking:** Unsupported constellation streams and operations now return
  typed unsupported errors instead of falling back to generic byte streams.
- **Breaking:** VMs using `usbip.yubikey = true` must enable guest control;
  USBIP attach and detach no longer have an SSH fallback.
- Advanced the public manifest schema to version 7 and the private bundle
  contract to version 11. The release adds runtime/provider capabilities,
  graceful-shutdown metadata, realm artifacts, configured launcher items,
  unsafe-local helper policy, and storage/synchronization contracts.
- Renamed the Wayland proxy package and binary to `d2b-wayland-proxy` and the
  configuration surface to `graphics.waylandProxy.*`. The former option path is
  retained as a compatibility alias for this release.
- Moved audio process identity entirely into the daemon-managed audio runner and
  retired the former audio service path.
- Changed live VM activation to a broker-prepare, guest-control activation, and
  broker-commit flow. Offline activation now fails closed except for explicit
  boot staging.
- Changed daemon list/status handling to use request-scoped artifact snapshots
  and parallel per-VM status probes, improving consistency and desktop-client
  latency.
- Changed runtime/state creation to rely on tmpfiles-owned parents and
  narrowly-scoped ACLs instead of activation-time permission repair.
- Changed `d2b-priv-broker.service` default logging from `debug` to `info`.

### Fixed

- Fixed unsafe-local graphical launches by supervising the proxy and configured
  app in one verified user scope with private runtime paths, typed readiness,
  first-client gating, bounded socket names, exact child reaping, canonical
  realm colors, and no direct-compositor fallback.
- Fixed picker/clipboard protocol compatibility, focus restoration, proxied
  virtual-keyboard replay, endpoint payload handling, cancellation, and
  backpressure while preserving picker-only transfer authority.
- Fixed USBIP claim, bind/unbind, firewall, ACL rollback, restart reconciliation,
  and revocation races; hardened security-key UHID framing, socket lifetime, and
  udev behavior.
- Fixed console and audio session ownership, QMP chardev handling, PipeWire
  targeting, and provider dispatch.
- Fixed daemon restart and host-switch continuity: `d2bd.service` reports ready
  only after socket bind and runner adoption, while running VMs remain alive.
- Fixed guest exec and GUI launch establishment timeouts under heavy virtiofs
  load.
- Fixed runtime, state, guest-control, observability, and per-role ACL ordering
  so daemon and runner access survives reboot and host switches without local
  overrides.
- Fixed net-VM cold-boot host preparation, qemu-media synchronization contract
  rendering, broker child reaping, and existing disk-image validation.
- Fixed realm controller and workload identity JSON field names and nesting to
  match their Rust DTOs.
- Fixed realm workload CLI routing, bare-VM migration hints, persistent-shell
  owner framing, and guest journal sizing.

### Removed

- Removed `d2b usb enroll`; qemu-media USB boot selection now uses
  `qemuMedia.source.usbSelector.byIdName` and `d2b usb probe`.

### Security

- Kept realm relay/provider credentials, remote registries, and realm audit out
  of the host daemon, broker, and bundle; relay identity is never mapped to
  local lifecycle authorization.
- Enforced same-UID unsafe-local helper registration, private proxy/readiness
  sockets, immutable proxy binaries, operation fingerprint parity, fail-closed
  group eligibility, and explicit logout/login after new group assignment.
- Enforced picker/clipd-only cross-realm clipboard transfer, strict bounded and
  redacted protocol metadata, destination-focus verification, and proxy-safe
  synthetic paste ordering.
- Tightened broker, runtime, qemu-media, observability, and per-role path
  ownership so diagnostics remain redacted and mutable host state stays within
  its declared authority.

## [1.3.1] - 2026-06-18

### Fixed

- Nix packaging now keeps legitimate source files whose names contain
  `target` (for example `d2b-constellation-core/src/target.rs`) while
  still filtering Cargo `target/` build directories out of package sources.
- USBIP lock acquire is now idempotent for the same VM: when a VM is
  restarted (`d2b down` + `d2b up`), the broker no longer
  refuses to re-bind a busid that the same VM already owns. Previously,
  every VM restart required a manual `d2b usb detach` + `d2b usb
  attach` cycle because the lock file persisted across the stop/start.
## [1.3.0] - 2026-06-18

### Fixed

- `tpm.enable` first-run: enabling TPM on a VM with no pre-existing
  `/var/lib/d2b/vms/<vm>/swtpm` state directory no longer wedges
  the VM's start. The privileged broker now provisions the per-VM
  swtpm state directory (owner `d2b-<vm>-swtpm`, mode `0700`) on
  first start, so swtpm no longer dies with a fatal NVRAM `ENOENT`.
  The documented manual `install -d … swtpm` workaround is no longer
  needed.
- A required per-VM runner that exits during VM start (e.g. swtpm)
  now fails the start fast with a typed, actionable error instead of
  blocking the daemon for the full readiness budget (~300 s). The
  swtpm control-socket readiness now waits for an active listener
  rather than the bare socket inode.
- The daemon handles client connections concurrently (bounded), so a
  slow or failing VM start no longer stalls unrelated clients
  (e.g. host status feeds). Mutating lifecycle operations are
  serialized per-VM/globally; read-only requests run in parallel.
- Per-VM NixOS evaluations now inherit the host's `nixpkgs.overlays` in
  addition to `nixpkgs.config`, so consumer security overlays patch VM
  closures as well as host closures.

### Security

- The per-VM state root `/var/lib/d2b/vms/<vm>/` is now `3770`
  (setgid **+ sticky**) so a non-owner per-VM role UID cannot rename
  or replace the principal-owned `swtpm` NVRAM directory. The swtpm
  state directory's inherited ACLs are cleared to owner-only `0700`
  on provisioning.
- TPM state-loss is fail-closed: a previously-provisioned swtpm state
  directory that goes missing or is replaced fails the VM start with
  `previously-provisioned-swtpm-state-missing` (bound to the
  directory identity via a root-owned marker outside the
  role-writable tree) rather than silently re-creating an empty TPM.

### Changed

- `bundleVersion` 4 → 5: adds the audited `PrepareSwtpmDir` broker
  operation for per-VM swtpm state-directory provisioning.

- CI: the `pr-l1-static-fast` x86_64 flake check is now sharded one job per
  flake check via a dynamic matrix (`make test-flake-list` enumerates the
  names; each shard runs `make test-flake` with `D2B_FLAKE_CHECK=<name>` to
  instantiate a single check in its own evaluator process). This replaces the
  monolithic `nix flake check`, which evaluated every nixosSystem toplevel in
  one process and OOM-killed the 16 GB runner (kept alive only by a 14 GB
  swapfile, ~41 min). A companion `flake-eval-x86-outputs` job evaluates the
  non-`checks` x86 outputs (`packages.*`, via `D2B_FLAKE_OUTPUTS=1`) that the
  per-check shards don't cover and the aarch64 leg (which only evaluates aarch64
  outputs) would miss. A stable `test-flake-x86` aggregator job gates on all of
  them to preserve the required status context, and a fail-closed drift gate
  (`tests/unit/gates/flake-check-matrix-sync.sh`, run by `make test-drift`,
  regenerate the pin with `make flake-matrix-pin`) keeps the CI shard matrix in
  sync with the flake's check set. The aarch64 leg still runs the full
  monolithic check.
- CI: the `test-rust` gate now restores/saves an sccache **local-disk** cache
  via `actions/cache` (opt-in through the new `D2B_CI_SCCACHE=1`, honored by
  `tests/test-rust.sh`; the pinned `sccache` is put on `PATH` since hosted
  runners ship rustup and skip the nix-shell that would otherwise supply it).
  We deliberately avoid sccache's native GitHub Actions backend: it exports
  `ACTIONS_RUNTIME_TOKEN` into the job shell environment, where the untrusted
  crate code the gate compiles and runs could read and exfiltrate it;
  `actions/cache` keeps that token inside its own action process. The broker's
  per-feature-pass target dirs are now deterministic siblings (not `mktemp`) so
  `CARGO_TARGET_DIR`, which sccache hashes, doesn't churn the cache key.

### Removed

- CI: deleted the redundant `pr-cargo-workspace` workflow, which re-ran
  `make test-rust` + `make test-proofs` already covered by `pr-l1-static-fast`'s
  `test-rust`/`test-proofs` jobs. Its `ci-uses-make` allowlist entry is removed
  too, and `cargo-ubuntu` is dropped from `main`'s required status checks.

### Added

- Internal v2 constellation provider-abstraction crates
  ([ADR 0032](docs/adr/0032-d2b-v2-constellation-control-plane.md)),
  with **no user-facing behavior change**: `d2b-constellation-core`
  (the pure, codec-neutral model - strongly-typed identifiers with
  fail-closed deserialization, the capability model, the semantic
  `ConstellationFrame` with a trusted per-operation required-capability
  mapping, the redacted audit envelope, and a bounded trace context) and
  `d2b-constellation-provider` (the async provider trait surface -
  runtime/workload/display/transport/stream-mux/codec/credential/
  daemon-access providers - with typed capability descriptors, structured
  capability-denial errors, byte-carrying transport sessions, and
  fail-closed mock/conformance fixtures). The same change adds the
  remaining foundation crates: `d2b-constellation-codec-protobuf`
  (a `prost` codec behind the `ProtocolCodec` trait, with frame-cap and
  fail-closed decode validation), `d2b-constellation-transport`
  (an in-memory loopback transport for conformance),
  `d2b-constellation-router` (the codec-neutral operation router +
  single-owner idempotency/dedup store keyed by the full operation
  namespace), `d2b-daemon-access` (the transport-neutral CLI↔daemon
  semantic API with its current local-Unix binding), `d2b-host-providers`
  (byte-identical local adapters over the existing Cloud Hypervisor and
  cross-domain Wayland argv generators), plus compile-only constellation
  peer-module skeletons inside `d2bd`. These crates are the foundation
  for later ADR 0032 work; they do not change any CLI, daemon, or
  on-host behavior.

- Documentation for the v2 constellation control plane
  ([ADR 0032](docs/adr/0032-d2b-v2-constellation-control-plane.md)):
  the threat model in `docs/explanation/design.md` now describes the
  realm-gateway trust boundary - the host daemon and broker hold no
  realm relay/provider credentials, remote node registries, or realm
  audit (those live inside a per-realm gateway guest VM); a realm relay
  is an untrusted, ciphertext-only rendezvous transport; relay identity
  is never local authorization (`SO_PEERCRED` + the `d2b` group
  remain the only local lifecycle authz surface); and work and personal
  realms never share a gateway guest or an L2 bridge. `SECURITY.md`,
  `docs/reference/privileges.md`, `docs/reference/daemon-api.md`, and
  `docs/reference/daemon-audit-check.md` are updated to state the same
  relay-is-not-local-auth and no-host-held-realm-credentials boundary.

- Host OTel collector parity (ADR 0033). New
  `d2b.observability.host.*` options bring the host edge collector to
  parity with the per-VM guest collector: `host.scrapeJournal` adds a host
  `journald` receiver (severity-mapped, restart-resuming `file_storage`
  cursor) and `host.otlpIngest.enable` adds a host-local OTLP ingest
  endpoint (a Unix socket in a dedicated `/run/d2b/otel/ingest/`
  subdirectory, isolated from `host-egress.sock`) plus a `traces` pipeline
  and `otlp` on the `metrics`/`logs` pipelines. Both default off and ship
  over the existing host → `sys-obs` vsock bridge (never a LAN).
  `host.otlpIngest.clientGroup` optionally widens the ingest socket from
  `0600` to a `0660` group. See
  [ADR 0033](docs/adr/0033-host-collector-parity.md).

### Changed

- All Rust workspaces (main + `d2b-priv-broker`) moved to **Rust
  edition 2024**; the pinned toolchain remains 1.94.1 and `unsafe_code`
  stays `forbid` (no `unsafe` was introduced for the migration).

- `deployment.environment` is now machine-and-env aware: the central
  collector stamps it `<hostName>` for host telemetry and
  `<hostName>-<env>` for workload VMs (e.g. `ddbus`, `ddbus-work`,
  `ddbus-personal`), instead of the bare host name for everything.
  `host.name` remains the per-source name (the host's name for host
  telemetry, the VM's name for workloads). See
  [ADR 0033](docs/adr/0033-host-collector-parity.md).

- Host-origin telemetry now carries the **hostname** as `vm.name` /
  `host.name` (via `d2b.observability.host.identityName`, default
  `networking.hostName`), assigned at the trusted ingress boundary, rather
  than the literal `"host"`. `vm.role` stays `"host"`. This is a default
  label change for observability-enabled hosts even with the new receivers
  off; set `d2b.observability.host.identityName = "host"` to keep the
  old labels. See [ADR 0033](docs/adr/0033-host-collector-parity.md).

- `ReadGuestFile` guest-control RPC: a single-shot, bounded, enum-keyed
  (initially `GuestConfig`-only) RPC for the host to read a small,
  trusted in-guest file over the authenticated vsock channel.
  `d2b-guestd` resolves the path with a safe `openat` from a trusted
  directory fd (`O_RDONLY | O_CLOEXEC | O_NOFOLLOW`, rejecting symlinks /
  `..` / non-regular files) and enforces a size cap before allocating;
  the response is bounded below both the ttRPC and `public.sock` frame
  budgets. The capability is negotiated as
  `GuestCapability::ReadGuestFile`, and an authenticated guest that does
  not advertise it fails closed. File-specific typed errors
  (`FileNotFound` / `FileTooLarge` / `PathUnsafe` / `ReadDenied`) carry
  operator-actionable remediations rather than a blind retry. The
  guest-control protocol version is bumped accordingly. See
  [ADR 0029](docs/adr/0029-framework-ssh-to-typed-guest-rpc.md).

- Production guest-control transport bridge: the host daemon now drives
  the authenticated vsock channel to guest-control VMs end-to-end. A
  broker-backed signer forwards each guest-control sign request verbatim
  to the privileged broker over a timeout-bounded dispatch, and a probe
  orchestrator resolves the per-VM vsock socket and peer credentials from
  the trusted bundle, connects to the host CID, and runs the
  authenticated Hello / Authenticate / Health handshake on a dedicated
  runtime with per-attempt timeouts. Spawning a guest-control VM's
  cloud-hypervisor runner now grants the unprivileged daemon a minimal,
  single-uid ACL (`--x` traversal on the per-VM state dir, `rw` on
  `vsock.sock`) scoped to the current socket inode. Because there is no
  CH-stop teardown hook carrying the socket path, the ACL is refreshed as
  a revoke-then-grant on each cloud-hypervisor (re)spawn - any stale grant
  left on a replaced or disabled socket inode is revoked before the live
  grant, so a prior generation cannot retain access (stop-time teardown is
  future work). Both the revoke and grant are audited with hash-only
  fields (no raw paths).

- New admin-only `public.sock` verb `ReadGuestConfig { vm }`: returns the
  editable guest config working copy of a guest-control VM as a bounded
  base64 string over the authenticated bridge. The daemon enforces the
  admin role before any probe / sign / read, recomputes size and sha256
  from the received bytes (never trusting guest-reported values), and
  bounds the encoded payload below both the ttRPC and `public.sock`
  frames.

  `tty=true && detach=false` now routes to a PTY-backed,
  connection-owned, non-durable attached exec. PTY setup keeps
  `unsafe_code = "forbid"` via a helper-exec pattern - a new `--tty-exec`
  mode of the static `d2b-exec-runner` performs the
  `setsid` + `TIOCSCTTY` + `tcsetwinsize` + `dup2` + `execve` handshake in
  safe `rustix`, so `d2b-guestd` never acquires a controlling
  terminal. stdout/stderr are merged onto the stdout stream
  (`ReadOutput(stderr)` returns a typed stderr-unavailable error);
  `CloseStdin` injects VEOF (`0x04`) and keeps the master open;
  `TtyWinResize` and `ExecSignal` are serialized through the per-exec
  control sequence, with signals restricted to the foreground process
  group (resolved via `tcgetpgrp` at delivery) and the
  `INT/TERM/HUP/QUIT/WINCH/USR1/USR2/KILL/TSTP/CONT` allowlist. An absent
  `initial_terminal_size` defaults to 24×80. Interactive sessions run
  indefinitely by default; teardown drops the master (SIGHUP), waits a
  bounded grace, then SIGKILLs the whole TTY session (in-session
  no-orphan; a `setsid`/double-fork escapee is a documented trusted-root
  limitation). Interactive detached exec remains unsupported; use
  non-TTY `d2b vm exec -d` for detached commands. See
  [`docs/reference/cli-contract.md`](docs/reference/cli-contract.md#exec)
  and the interactive-exec section of
  [ADR 0028](docs/adr/0028-guest-control-plane-over-vsock.md). The
  guest-control wire contract is unchanged (the TTY surface was already
  present).

- New per-VM option `d2b.vms.<vm>.guest.exec.interactiveMaxRuntimeSec`
  (default `0` = unlimited) caps interactive TTY exec runtime
  independently of the non-interactive attached ceiling. It is mirrored
  read-only into the guest config and forced from the host module, and
  emitted to `d2b-guestd` as `--interactive-max-runtime-sec`
  alongside the detached exec surface.

- Guest exec now accepts bare command names and relative program paths in
  both attached and detached modes. `guestd` passes `argv[0]` through the
  workload user's login shell (`exec "$@"`), so the command is resolved
  by that user's login `PATH`; invalid program names get the distinct
  `INVALID_PROGRAM` / `guest-control-invalid-program` error. The
  console replacement is `d2b vm exec -it <vm> -- bash`.

- Detached workload-user exec is enabled with
  `d2b vm exec -d <vm> -- <cmd>` and VM-first management verbs:
  `d2b vm exec <vm> list`, `logs <exec_id>`, `status <exec_id>`,
  and `kill <exec_id>`. Detached jobs are non-TTY, run as the workload
  user (never root), stay inside guestd rather than adding a broker op,
  and survive host client disconnect. Retained stdout/stderr use bounded
  ring buffers with dropped/truncated accounting and per-stream offsets;
  `kill` maps to idempotent two-phase `ExecCancel` (graceful terminate,
  bounded grace, force kill). Guestd reconciles detached runner/workload
  units at startup, cleans orphaned workloads, and reaps terminal records.

### Fixed

- The OTel host-bridge runner (`socat UNIX-LISTEN:host-egress.sock,...`)
  now self-heals across obs-VM restarts. `socat` does not unlink a
  pre-existing socket path before binding, so a stale `host-egress.sock`
  left by a previously-drained bridge made the freshly-spawned bridge
  exit immediately ("address in use"); the readiness probe only checks
  the socket *file* exists, so the failure was masked and host telemetry
  silently stopped reaching `sys-obs`. The broker now drops a
  provably-stale (non-listening) `host-egress.sock` before each
  `OtelHostBridge` spawn - mirroring the existing cloud-hypervisor / video
  socket preflight - so restarting the obs VM no longer wedges the host
  telemetry path. A live listener is never removed, and only sockets under
  `/run/d2b/otel/` are eligible.

- The privileged broker now compiles under the `layer1-bootstrap`
  feature (and thus `--all-features`): the guest-control `GuestControlSign`
  audit-redaction arm in `request_fields_value` is gated to the real-wire
  build, since under `layer1-bootstrap` `BrokerRequest` aliases to the
  bootstrap `BootstrapCall`, which has no such variant. The `Read` and
  `FileTypeExt` imports it uses are gated the same way so the bootstrap
  build stays warning-clean.

- The broker's non-socket-activated (test-mode / legacy) self-bind path
  now constrains the creation umask so the socket is materialized at
  `0o660` directly. `fchmod()` on an `AF_UNIX` socket fd does not change
  the bound path's mode on some kernels/filesystems, so the prior
  `fchmod`-only approach could leave the socket world-traversable
  (`0o755`). Production is unaffected (it uses socket activation, where
  systemd owns the socket mode).

- Guest-control chunked stdio docs now account for protobuf `bytes`
  allocation before handler entry by specifying ttRPC receive caps,
  bounded post-decode byte semaphores, and per-exec stdin permits for
  malicious concurrent `WriteStdin` fan-in.

- TPM-enabled guests now flush stale loaded/saved TPM sessions during
  early boot before SRK provisioning. This prevents swtpm session-handle
  exhaustion from breaking TPM-bound credentials while preserving NVRAM
  and persistent handles.

- Detached exec (`d2b vm exec -d`) now works end-to-end. Three faults in
  its initial implementation are fixed: the per-VM exec runner verified the
  workload's cgroup placement against a top-level `d2b-exec.slice` path
  even though systemd nests it under `d2b.slice`, so every detached
  command was killed at spawn; the daemon panicked (taking down `d2bd`)
  when a detached management verb (`list`/`logs`/`status`/`kill`) was
  dispatched, because it built a nested async runtime on the request thread;
  and the guest reconciler matched a running workload's command against
  `systemctl show` output using exact, quote-aware argv tokens, but systemd
  renders `ExecStart` argv as a literal, unescaped, space-joined string - so
  live jobs (and any command containing a space, quote, backslash, or
  semicolon) were misclassified as foreign and reaped as `lost-guestd`
  shortly after starting. Workload identity is now matched against systemd's
  raw rendering, and a failed runner-side spawn verification logs an
  actionable guest-journal diagnostic. (Detached command arguments may not
  contain a newline or carriage return, which `systemctl show` cannot render
  on one line; such commands are now rejected at create as an invalid argument
  rather than starting and then being reaped.)

### Added

- `d2b vm exec <vm> -- <cmd…>` (and `-it` for an interactive TTY):
  an admin-only operator command that runs a command inside a running
  guest over the authenticated guest-control transport - CLI → daemon
  `public.sock` → authenticated guest-control vsock → `guestd` exec
  RPCs. There is no SSH and no host PTY (the guest owns the PTY); the
  host only flips termios via an RAII raw-mode guard restored on every
  exit, error, disconnect, or panic. Non-interactive mode streams
  stdout/stderr separately; `-it` allocates a guest PTY, merges stderr
  into stdout, and forwards `SIGWINCH`/`SIGINT`/`SIGQUIT`/`SIGHUP`/
  `SIGTERM`/`SIGTSTP` to the guest foreground process group (signal
  handlers enqueue only). The daemon holds an in-process exec session
  table whose per-session workers own one persistent authenticated
  guest-control client with fresh per-op deadlines; session-table caps
  (global / per-UID / per-VM) and `Start` rate limiting are enforced
  before connect/auth, and an old or non-guest-control generation fails
  closed with exit `70` (no proxy, no SSH fallback). Guest exit status
  passes through unchanged (`128+N` for signal death); transport, auth,
  capacity, protocol, old-generation, and internal failures map to
  reserved CLI exit codes that `--json` disambiguates from a guest exit
  code via `source`/`reason`/`guestExitCode`/`transportExitCode`. `-it`
  is human-only and is rejected together with `--json`; non-interactive
  detached commands use `d2b vm exec -d <vm> -- <cmd>`. Attached exec
  establishes one redacted kind=critical audit event (vm / peer uid / tty
  only), and detached create/kill adds redacted daemon audit carrying only
  vm / peer uid / result / exec id. Opaque session handles, argv, and
  stdio/env/cwd/paths never reach any log, span, audit record, or metric
  label.

- Detached guest exec: `ExecCreate(detach=true)` runs a non-interactive
  command that outlives the originating connection, supervised by the root
  guest daemon through slot-based `systemd-run` transient units
  (`d2b-exec-<NN>.service`, scoped to a guest-internal `d2b-exec`
  slice). Unit names and argv carry only the slot index - never the exec id,
  argv, environment, or cwd. stdout/stderr are retained in slot-keyed files
  under a root-owned, 0700, boot-scoped `/run/d2b-exec` parent with
  drop-oldest truncation accounting: 4 MiB per stream, an exact 256 MiB
  VM-global quota (32 retained slots × 2 streams × 4 MiB), and 8 active
  execs per VM. Detached execs run indefinitely by default
  (`guest.exec.detachedMaxRuntimeSec = 0`), with an optional per-VM runtime
  ceiling. Cancellation is a two-phase, control-file mechanism with no
  in-process signal handler. Terminal records are retained for 30 minutes
  then garbage-collected; a running detached job is never reaped. guestd
  re-adopts live detached execs across a guestd restart within one boot,
  reconciles valid runner/workload units before advertising detached
  capability, and cleans orphaned workloads. The operator CLI exposes the
  substrate as `d2b vm exec -d <vm> -- <cmd>` plus
  `d2b vm exec <vm> list|logs|status|kill` management verbs.

- `ExecList` RPC (guest-control protocol version 2): a minimal, read-only
  discovery call that enumerates the caller's detached execs for the same
  VM token + boot (bounded ≤32). Each entry carries the exec id, slot,
  state, create time, an argv SHA-256 hash (never raw argv), and per-stream
  truncation/dropped-byte counters. The CLI and public daemon DTOs do not
  expose the argv hash.

- `ExecExpired` guest-control error kind, distinguishing a retention-evicted
  detached record from `StaleSession` (boot mismatch) and `ExecNotFound`
  (unknown id).

- Host VM option `d2b.vms.<vm>.guest.exec.detachedMaxRuntimeSec`
  (unsigned, default 0 = indefinite) plumbed through to the guest exec
  runtime as a per-exec `RuntimeMaxSec` backstop when non-zero.


  `packages/d2b-contracts/proto/guest_control.proto` - generated schema plus
  protobuf source for the ADR 0028 ttRPC contract, covering health, Hello,
  capabilities, exec lifecycle, chunked stdio RPC shapes, bounded health
  labels, bounded string identifiers/payload metadata, oneof-style terminal
  status, structured stdio error results, and descriptor-shape drift checks.

- Initial guest-side Rust crates for the guest control plane:
  `d2b-guestd`, `d2b-userd`, and `d2b-exec-runner`, with
  fail-closed binaries, fakeable daemon/user/session traits, and bounded
  runner input validation.

- Bootstrap/fail-closed guest-static package outputs `d2b-guestd-static`,
  `d2b-userd-static`, and `d2b-exec-runner-static`, plus an ELF check
  proving the guest binaries have no interpreter or dynamic dependencies.
  Guest VM evals now consume these static outputs through the guest-control
  module, with a static-fast eval gate proving the package references.

- Opt-in guest-control auth token delivery wiring: per-VM runtime token path
  option, framework-owned materialized token file, read-only guest credential
  share, and guestd `LoadCredential` wiring with eval coverage.

- Host-owned Cloud Hypervisor vsock allocation now uses the manifest's
  base socket path for every VM, reserves distinct CIDs for env net VMs and
  workload VMs, and rejects consumer `--vsock` overrides so observability and
  guest-control port reservations share one authoritative per-VM vsock device.
  This bumps the public manifest to `manifestVersion = 5` because the existing
  `observability.vsockCid` / `observability.vsockHostSocket` fields now define
  the base Cloud Hypervisor vsock device. (`5` unifies this base-vsock change
  with the SigNoz observability metadata that landed as `4` on a sibling
  branch; the shipped parser/daemon/broker accept only `5`.)

- `d2bd` now has an internal Cloud Hypervisor CONNECT helper for the
  guest-control transport port. This is transport groundwork only: it does not
  change VM readiness, status output, CLI help, or exec behavior.

- `packages/d2b-contracts/src/generated/guest_control.rs` now contains committed
  protobuf message bindings generated from
  `packages/d2b-contracts/proto/guest_control.proto` via
  `cargo run --locked --manifest-path packages/Cargo.toml -p xtask -- gen-guest-proto`.
  The new
  `tests/guest-proto-bindings.sh` gate verifies the generated bindings are
  deterministic, unsafe-free, and message-only (no ttRPC runtime stubs).

- Guest-control protobuf now has an authenticated `Authenticate` handshake:
  `Hello` is challenge-only, authenticated health/capabilities are returned
  only after proof-of-possession, and `d2b-guestd` has a pure auth core
  with fixed-size HMAC transcript tests. No listener, readiness, or exec CLI
  behavior is enabled yet.

- `d2b-guestd` now owns generated ttRPC service bindings and a dormant
  `--serve --vm-id <vm>` service mode for Hello challenge, Authenticate, and
  authenticated Health/Capabilities. The guest service remains opt-in manual-start only
  (`wantedBy = []`) and does not enable host readiness or exec behavior.

- The privileged broker now exposes a structured guest-control HMAC signer, and
  `d2bd` has a host-side authenticated Health probe helper. The helper
  produces daemon-local health evidence only; it does not replace SSH readiness
  or enable exec.

- Guest exec policy option `d2b.vms.<vm>.guest.exec.enable` gates guest
  exec (off by default). This is dormant policy wiring only; no exec
  runtime/CLI behavior is enabled by this option yet.

- Guest-control retained-log security requirements and canary-based
  redaction test coverage for stdout/stderr logs, telemetry, health, and
  CLI JSON.

- `proofs/chunked-stdio-conformance` - executable safe-Rust proof for
  the selected Kata-style chunked stdio exec I/O protocol, covering
  byte-exact offset reads, idempotent stdin writes, slow-consumer bounds,
  concurrent attached fairness, stale sessions, EOF, resize, and signal
  exit mapping.

- Strengthened PTY/job-control proof coverage for guest-control exec,
  including session leadership, controlling-terminal foreground process
  groups, PTY close/drain behavior, SIGWINCH resize semantics, and
  protocol-side TTY `CloseStdin`.

- `docs/reference/guest-control-exec-io-credit-window.md` - bounded ttRPC
  duplex-stream exec I/O design using d2b `TerminalFrame` messages,
  explicit byte credit, close/EOF, resize/signal/exit/error frames, CLI
  behavior, conformance matrix, risks, and required tests.

- Guest systemd-journal log collection. The per-VM OpenTelemetry
  collector now follows the guest journal through the contrib `journald`
  receiver and forwards it to SigNoz as logs tagged with the VM's
  `vm.name` / `vm.env` resource attributes, with the journal `PRIORITY`
  mapped to a readable OTel severity (`INFO`/`WARN`/`ERROR`/…) and a
  `file_storage` cursor so a collector restart resumes without dropping
  entries. `d2b.vms.<vm>.observability.scrapeJournal` now defaults
  to `true` (previously a reserved no-op) and the guest collector user
  is granted `systemd-journal` read access plus `journalctl` on its
  unit PATH. Ingested telemetry's `deployment.environment` resource
  attribute is the physical host machine name (from the host's
  `networking.hostName`, settable via `d2b.observability.hostName`)
  so SigNoz groups VMs by the host they run on; the per-VM env stays on
  `vm.env` / `service.namespace`.

- Native, container-free SigNoz observability backend packages and ADR.
  The bundled observability path now targets SigNoz, the SigNoz OTel
  Collector, schema migrator, ClickHouse, and ClickHouse Keeper as native
  NixOS services.

- `d2b.site.niriVmBorders.{enable,outputPath}` - opt-in niri KDL
  window-rule include generator. When enabled, installs a KDL file at
  the configured path (default `/etc/d2b/niri-vm-borders.kdl`)
  containing a crosvm scanout-window hide rule and one
  `window-rule` per enabled graphics VM. Rules match the
  `d2b.<vm>.` app-id prefix that the host Wayland filter proxy
  writes onto guest windows. Include the file from niri config with
  `include "/etc/d2b/niri-vm-borders.kdl"`. Requires niri ≥ 0.1.9.
- `d2b.vms.<vm>.graphics.niriBorderColor` - per-VM active border
  color override for the generated niri rules, as a six-digit CSS hex
  color (`#rrggbb`). Defaults to `null`, which uses a deterministic
  palette color derived from the VM name.
- `d2b.vms.<vm>.graphics.waylandFilter.{enable,denyGlobals,allowGlobals,maxVersions}` -
  host-side Wayland filter controls for graphics VMs that opt into
  cross-domain forwarding. The filter is enabled by default when
  `graphics.crossDomainTrusted = true`, denies unknown/high-risk globals
  by default, and exposes explicit allow/deny/version-cap overrides.
- `d2b.vms.<vm>.graphics.waylandFilter.{byteLogging,dmabufAllow,dmabufDeny}` -
  default-off diagnostics and dmabuf format/modifier controls for the
  host-side Wayland filter. The filter preserves compositor dmabuf
  feedback by default and lets operators hide known-bad format/modifier
  pairs while keeping buffer creation requests fail-closed against the
  same policy.
- `docs/how-to/niri-vm-borders.md` - how-to for enabling the niri
  include, customizing colors, verifying the setup, and understanding
  the `crossDomainTrusted` requirement for app-id matching.
- `docs/how-to/migrate-to-wayland-proxy.md` - migration guide covering
  app-id renaming, Xwayland fail-closed behavior, `crossDomainTrusted`
  requirement, niri rule updates, and rollback procedure.
- `docs/reference/wayland-filter-warnings.md` - reference warning
  catalog for `graphics.waylandFilter` listing every warning condition,
  the triggering option or global, why the warning exists, and how to
  override intentionally.

- StoreSync-only observability JSONL export. The privileged broker now
  writes a positive-allow-list projection of each terminal StoreSync
  attempt to `<stateDir>/observability/store-sync/store-sync-<utc-date>.jsonl`
  (`0640`, daily-rotated, best-effort). The export carries exactly the
  allow-listed fields (`schema_version`, `target_vm`, `vm_id`,
  `target_env`, `generation_id`, `generation_token`, `sync_status`,
  `error_stage`, `cleanup_status`, `cleanup_reason`, `authz_outcome`,
  closure/linked/skipped/swept counts, `fast_path`, and the flattened
  `*_ms` timings) via a dedicated `StoreSyncObservabilityRecord` struct
  so no serializer ever receives the full host audit record; host-only
  fields (`caller_principal`, `retained_generations`, host/store paths,
  `db.dump`, marker payloads) are redacted by construction. Host Alloy
  follows only this export glob (`local.file_match` + `loki.source.file`,
  following rotation) and the `alloy` identity receives focused
  read/traverse ACLs to the export directory only - never the unified
  broker audit log, the privileged daemon socket, or d2bd state. The
  Loki stream stays a host singleton (`vm="host"`, `env="host"`,
  `role="host"`, `source="store-sync-audit"`); `target_vm`/`target_env`
  remain JSON content. `target_env` is resolved from the trusted manifest
  when present (and remains a JSON field, not a stream label). New gate
  `tests/store-sync-export-eval.sh`;
  `tests/loki-label-cardinality-eval.sh` now also parses
  `local.file_match` `path_targets` label maps. See
  [ADR 0027](docs/adr/0027-store-view-hardlink-live-pool.md) and
  `docs/reference/store-sync.md` § "Observability export".

- `d2b store verify <vm> [--repair] [--json]` - explicit
  broker-backed live-pool integrity verification for the ADR 0027 split
  store-view. The CLI is thin and never reads `store-view` directly;
  `d2bd` sends a typed `BrokerRequest::StoreVerify` to the privileged
  broker, which verifies `state/current`, `meta/current`, the host marker,
  zero-length live marker, and every manifest top-level basename in
  `live/`. It writes host-only integrity state under
  `store-view/state/generations/<generation-id>/integrity.json` (or
  `state/integrity-unknown.json` when generation identity is unavailable)
  and returns the signed JSON envelope documented in
  `docs/reference/cli-output/store-verify.md`. `--repair` now delegates to
  StoreSync as a forced non-fast-path republish, then verifies again before
  returning `repaired`; incomplete repairs remain exit-4 `drift`/`unknown`
  instead of a success-shaped result.
- `d2b store verify` now performs deep recursive live-pool verification
  against trusted source closure paths (file type, executable bit, symlink
  target, and hardlink identity or byte equality for copied fallback files).
  Existing top-level packages with internal drift are repaired by staging clean
  replacements and swapping them into `live/` with same-filesystem
  `RENAME_EXCHANGE`, so the served basename is never absent.
- StoreSync success audit/export records now populate available phase timings
  (`lock_wait_ms`, `lock_hold_ms`, `probe_ms`, `verify_ms`, `stage_ms`,
  `metadata_ms`, `cleanup_ms`) in addition to `total_ms`.
- StoreSync now performs conservative cleanup/retention when no virtiofsd
  process appears to be serving the VM's `store-view/live` path. Offline-safe
  cleanup removes unretained live basenames, stale meta/state generation dirs,
  and stale gcroots; online or uncertain serving state defers cleanup.
- Cross-mount store-view materialisation no longer shells through
  `unshare ... /bin/sh -ceu ...`. The broker now execs
  `d2b-activation-helper private-store <verb>` directly; the helper
  unshares its own mount namespace, makes propagation private, lazily detaches
  `/nix/store`, then runs the selected build/replace verb from stdin JSON.

- `d2b config` verb group - the host-side review/approve workflow
  for a VM's guest-editable `guestConfigFile`: `config sync` pulls the
  in-guest edited file over the existing per-VM SSH key into a
  user-local staging copy; `config diff` shows a unified diff against a
  live file; `config approve` atomically writes the staged copy onto an
  operator-chosen target; `config reject` discards it; `config status`
  reports pending stagings. The CLI only writes its own staging area and
  the operator-named `--to` target - it never auto-touches the config
  tree. `approve`/`reject` are host-operator-only and are the
  authoritative containment boundary (the host only ever evaluates an
  operator-approved guest file); an eval-time namespace lint on
  `d2b switch` additionally rejects guest-set host-owned options as
  defense-in-depth. No new privileged surface (no virtiofs, no new
  socket); the untrusted pull is bounded (size cap + timeout). `d2b
  up` / `start` and `d2b status` also print a human-output note when
  a VM has a pending un-approved staged config.
- `d2b.vms.<vm>.guestConfigFile` - a dedicated, **guest-editable**
  per-VM NixOS module for the in-guest OS layer (packages, services,
  in-guest users, files). It is merged into the guest like `config`,
  but is **contained**: a best-effort eval-time namespace lint rejects
  it if it sets any host-owned `microvm.*` (runner substrate) or
  `d2b.*` (framework) option, naming the offending option(s)
  (detected by definition-existence over the real NixOS module set, so
  `imports`/`builtins.toFile`/`_file`-spoofing are caught). The lint is
  defense-in-depth, not a sound sandbox - operator review/approve is the
  authoritative boundary; see
  [ADR 0024](docs/adr/0024-in-vm-guest-config-sync.md) for the trust
  model and the deferred sound-evaluator work. This is the foundation
  for the in-VM config-sync workflow - an operator can edit this file
  from inside the VM and sync it back for review. Host-owned settings
  stay in `config`, which the guest cannot edit. When set, the current
  approved guest config is also seeded into the VM (read-only at
  `/etc/d2b/guest-config.nix`, plus a writable working copy at
  `/var/lib/d2b-guest/guest-config.nix`) so it can be edited from
  inside the VM. See
  [`docs/how-to/edit-vm-config-from-inside.md`](docs/how-to/edit-vm-config-from-inside.md).

### Removed

- `d2b vm konsole` is removed. The subcommand was a thin wrapper that
  re-exec'd `d2b vm exec -it <vm> -- <login-shell> -l` inside a host
  terminal emulator; operators now invoke `d2b vm exec -it` directly.
  All references (CLI surface, shell completions, manpage, and reference
  docs) are dropped accordingly.

### Changed

- `d2b vm exec` now runs the requested command as the VM's
  configured workload user (`ssh.user`) - **never root** - inside a real
  PAM login session (`systemd-run --property=PAMName=login
  --uid=<user>`). The command sees the same environment an interactive
  SSH login would (`XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, the login-shell
  profile), so graphical and login-shell workflows (e.g. launching a
  browser) work unchanged; operators elevate with `sudo` inside the
  session. `guestd` host-fixes the exec identity and ignores the wire
  `user` field. The per-VM `guest.exec.allowRoot` and `guest.exec.users`
  options are removed - enabling `guest.exec.enable = true` on a VM with
  a workload user is sufficient, and a VM whose `ssh.user` is unset,
  `root`, or otherwise invalid disables exec at eval time with a typed
  message. See
  [ADR 0030](docs/adr/0030-guest-exec-as-workload-user.md).
- Framework readiness for a guest-control-capable VM is now the
  authenticated guest-control Health probe rather than a raw TCP-22 SSH
  connect. The per-VM DAG node `guest-ssh-readiness` is replaced by
  `guest-control-health` (`ProcessRole::GuestControlHealth` +
  `ReadinessPredicate::GuestControlHealth`), which fails closed: a VM is
  ready only once the daemon completes the full authenticated handshake
  and the guest reports `Healthy` or `Degraded`. Old-generation /
  unreachable / auth-failed / timed-out guests are never marked ready.
  Per-VM guest sshd and host keys remain for the SSH compatibility
  window but no longer drive framework readiness. See
  [ADR 0029](docs/adr/0029-framework-ssh-to-typed-guest-rpc.md).
- `d2b config sync` on a guest-control VM now pulls the editable
  guest config over the authenticated guest-control bridge (the new
  `ReadGuestConfig` daemon verb) instead of an SSH transfer. The host
  computes size and sha256 from the received bytes and keeps the existing
  atomic temp+fsync+rename staging. `--dry-run` reports
  `transport: "guest-control"` and the planned target without reading any
  guest bytes or printing an SSH command, and SSH-only flags
  (`--host` / `--user` / `--key` / `--known-hosts` / `--guest-path`) are
  rejected on the guest-control path with a remediation pointing at the
  operator SSH compatibility transport. Old-generation VMs that predate
  guest-control fail closed with `guest-control-unavailable-old-generation`.
- The framework readiness label is now the canonical `guest-control-health`
  (no per-VM suffix) across `status`, `vm list`, and the start preview;
  the start-preview DAG no longer hard-codes an `ssh-ready` node.
- The default observability VM name is now `sys-obs`. The old
  `sys-obs-stack` state is not deleted automatically; keep it for
  rollback until the new stack is validated.
- Observability metadata in `vms.json` moves to manifest version 5 for
  the SigNoz backend shape (unified with the base-vsock change; the
  intermediate `4` was never shipped on its own). Historical v3 fixtures
  remain frozen.
- Host and guest telemetry collection is moving from Alloy pipelines to
  OpenTelemetry Collector services that export OTLP over d2b's
  broker-supervised Unix/vsock transport.
- Retired Grafana credential-file options are now documented as
  compatibility shims; native SigNoz credentials can be sourced from
  `d2b.observability.signoz.{jwtSecretFile,rootPasswordFile,clickhousePasswordFile}`.
- `retention.*` and `sampling.*` remain compatibility shims for the
  retired Tempo/Loki backend and warn when changed; native
  SigNoz/ClickHouse retention is operator-managed.
- Per-VM store isolation is moving to the Rust-owned `store-view/live`
  hardlink pool
  ([ADR 0027](docs/adr/0027-store-view-hardlink-live-pool.md)). The
  broker `StoreSync` path is the canonical writer for store-view
  metadata and live pool updates; host activation no longer
  builds/sweeps store-view closures. The guest readiness marker
  `store-view/live/.d2b-marker-<vm>` is a zero-length file, and each
  generation publishes a guest-safe `meta.json` authored by an
  independent allow-list serializer (`schema_version`, `generation_id`,
  `generation_token`, `sync_status`, `closure_count`) that never
  receives the full host audit record. The broker `StoreSync` wire
  response now carries the collision-free `generation_id` alongside the
  u32 `generation_token` (request + response renamed `generation` →
  `generation_token`); the token is display/wire only and is never used
  as the on-disk layout key. Each StoreSync attempt that reaches the
  broker handler emits exactly one terminal structured broker audit
  record under the signed `StoreSyncAuditFields` schema
  (`schema_version = 1`) with invariant-enforcing constructors and
  `validate()`: success records use `ok_fast_path` / `ok_non_fast_path`,
  and a failure emits a `failed` record carrying the classified
  `error_stage` (the failure surfaces as `BrokerError::StoreSyncFailed`
  and is never double-audited). Authorization-deny emission
  (`error_stage = authz`) is modelled by the `denied` constructor but is
  not yet reachable from dispatch, pending a per-VM StoreSync
  authorization policy.
- Graphics VMs that opt into cross-domain forwarding use
  `wl-cross-domain-proxy` in the guest and a host-side
  `d2b-wayland-filter` proxy instead of the former
  `wayland-proxy-virtwl` guest relay.
- `d2b.vms.<vm>.graphics.xwayland.enable = true` now fails eval
  during the Wayland-only migration. X11 application support will return
  through a separately validated helper path.

### Security

- Graphics VMs that opt into cross-domain forwarding now route guest
  Wayland traffic through a host-jailed `d2b-wayland-filter` process
  before reaching the real host compositor. The GPU sidecar connects to
  the per-VM filter socket; the dedicated `d2b-<vm>-wlproxy`
  principal is the VM-specific role with compositor socket access.
- Per-VM store isolation: the daemon-native virtiofsd `ro-store` runner
  served the host's entire `/nix/store` to every guest, so a guest's
  `/nix/store` exposed all host store paths instead of only the VM's own
  closure. virtiofsd now serves the per-VM closure-only hardlink farm
  (`/var/lib/d2b/vms/<vm>/store`), restoring the isolation the legacy
  `BindReadOnlyPaths /nix/store -> per-VM farm` provided; a guest's
  `/nix/store` now contains only its own closure.
- StoreSync observability export confinement: Grafana Alloy is granted
  focused POSIX ACLs (`u:alloy:--x` traverse on `<stateDir>` and
  `<stateDir>/observability`, `u:alloy:r-x` + a `default:u:alloy:r--`
  ACL on the export dir) to read the StoreSync export and nothing else
  under the broker state dir. Alloy is never added to the `d2bd`
  group and gets no read access to the unified broker audit log
  (`<stateDir>/audit/broker-*.jsonl`) or the privileged daemon socket.
  The export itself is a redacted projection, so a host-Alloy compromise
  exposes only the allow-listed StoreSync fields already destined for
  Loki, not the host-confidential audit stream.

### Fixed

- The host OTel bridge is now represented as a daemon/broker process role
  (`otel-host-bridge`) so readiness can track the broker-spawned runner.
- Observability relay ACL setup now excludes the host bridge principal
  from broad obs-VM state directory grants and uses the d2b-owned OTel
  runtime path for the bridge egress socket.
- TPM-enabled guests now flush stale loaded/saved TPM sessions during
  early boot before SRK provisioning. This prevents swtpm session-handle
  exhaustion from breaking TPM-bound credentials while preserving NVRAM
  and persistent handles.
- VM start (`d2b up` / `switch`) no longer aborts with
  `SpawnRunner failed ... broker-error` ("Invalid cross-device link")
  while building the per-VM store-view hardlink farm on hosts where
  `/nix/store` is bind-mounted read-only on top of itself (the stock
  NixOS layout). `link(2)` is rejected across that vfsmount boundary
  even when both paths share the same underlying filesystem, so the
  broker's in-process farm build failed with `EXDEV`. The broker now
  builds the farm inside a private mount namespace where `/nix/store`
  is lazily detached (mirroring the existing activation-time
  `d2b-store-sync` workaround), via the `d2b-activation-helper
  build-store-view-farm` subprocess, and only falls back to that
  namespace path when an in-process build actually hits the cross-mount
  case (so same-filesystem hosts and tests stay in-process). A raw
  `EXDEV` at the `link(2)` site is now classified as a recoverable
  same-filesystem cross-mount (retried in the namespace) versus a fatal
  genuinely-different-filesystem error (propagated).
- VM start no longer fails while building the per-VM store-view farm on
  a `nix-store --optimise`d store. Deduplicated empty/tiny store files
  share a single inode that reaches the filesystem hardlink ceiling
  (ext4 `EXT4_LINK_MAX` = 65000); the farm builder now falls back to a
  byte copy for those already-saturated (read-only) inodes instead of
  failing with `EMLINK`.
- VM start no longer leaves the per-VM state/runtime root
  (`/var/lib/d2b/vms/<vm>`, `/run/d2b/vms/<vm>`) owned by a
  transient runner principal with a clipped POSIX ACL mask. The
  vm-start directory prepares now preserve the ownership + mode that
  host activation establishes (`d2bd:users 2770` plus per-runner
  ACLs) on an existing directory, so runners (virtiofsd, gpu, video)
  keep write access to their per-VM runtime dir and the ownership-matrix
  preflight no longer trips.
- `d2b switch` / `boot` / `test` no longer fail with `broker-error`
  ("no store-view intent in the trusted bundle"). The per-VM closure
  artifact now emits a populated `hostGeneration` (a deterministic,
  content-derived store-view generation), so the broker builds a
  store-view intent for every VM instead of skipping it. Previously
  live activation was impossible and the only way to apply a new
  generation was `d2b down <vm> --apply` followed by
  `d2b up <vm> --apply`. The per-VM `/nix/store` hardlink farm now
  also fails closed on a store-view generation collision (two distinct
  closures of one VM mapping to the same generation number) instead of
  unioning them, by pinning the closure identity in the generation
  marker.
- VM start no longer aborts with `SpawnRunner failed ... broker-error`
  on the first runtime-directory step. The broker's path-safe directory
  opener resolved every path from `/` with `RESOLVE_NO_XDEV`, which
  fails with `EXDEV` ("Invalid cross-device link") the moment it must
  cross a mount boundary - and the per-VM runtime dir lives under the
  `/run` tmpfs, the tap device under `/dev`, cgroups under `/sys`, etc.
  Resolution now walks component by component and follows a *real*,
  pre-existing mount crossing (still refusing symlink / magic-link
  components and `..` escapes at every step), so legitimate
  cross-filesystem paths resolve while the load-bearing symlink
  protection is preserved.
- Broker spawn/host-prep failures are no longer opaque. The broker now
  logs the live-handler root cause (errno / path / stderr) to its
  journal, the daemon includes the broker's `message` in its
  `vm start node spawn failed` log, and failure remediations point at
  the working `journalctl -u d2b-priv-broker` instead of the
  `d2b audit --strict` command (which returns `not-yet-implemented`).
- GitHub Actions PR hardening keeps fork PR code off self-hosted
  runners, makes the privileged oracle workflow manual-dispatch only,
  and repairs the affected CI validation gates so the hardening can
  merge through the normal PR checks.

## [1.2.0] - 2026-06-03

Primarily a stabilization release per
[ADR 0022](docs/adr/0022-stabilization-mode-releases.md): deferrals
from the v1.x cycle close out and a live-VM smoke gate is now
required before tagging. It also lands two default-off, opt-in
graphics video-decode paths and unifies the lifecycle Unix group
into a single `d2b` group - a breaking change for consumer
configs that referenced the legacy group names (see
**Changed (breaking)** below).

### Added

- `d2b vm start --apply` readiness split into `process-alive` +
  `api-ready` DAG nodes. `--no-wait-api` opts into exit-0 once the
  process is alive; the strict-API default is preserved.
- `d2b vm status --json` surfaces the new `api_ready` field
  (`yes` / `pending` / `timeout` / error).
- `d2b host doctor` ships four new probes
  (`check_seccomp_bpf_loaded`, `check_pre_ns_posture`,
  `check_broker_reap_health`, `check_bridge_ipv6_sysctl`); see
  [`docs/reference/doctor.md`](docs/reference/doctor.md).
- `writableStoreOverlay` re-enabled. The broker provisions the per-VM
  overlay disk via the new `SpawnRunnerPlanOp::DiskInit` op
  (`mkfs.ext4` on first spawn). Size override via
  `d2b.vms.<vm>.writableStoreOverlaySize` (default 1 GiB).
- `tests/integration/live/live-vm-smoke.sh` (`--lite` / `--full`) is the maintainer
  pre-tag gate (`make pre-tag` / `make smoke-lite`); results land in
  `${TMPDIR:-/tmp}/d2b-smoke-run-log.txt`.
- New ADRs:
  [ADR 0022](docs/adr/0022-stabilization-mode-releases.md)
  (stabilization-mode releases) and
  [ADR 0023](docs/adr/0023-runner-role-lifecycle-matrix.md)
  (runner-role lifecycle matrix).
- New runbooks:
  [`docs/how-to/recovery-pre-ns-role-failure.md`](docs/how-to/recovery-pre-ns-role-failure.md),
  [`docs/how-to/route-conflicts.md`](docs/how-to/route-conflicts.md).
- Graphics VMs can opt into the daemon-spawned virtio-media H264 decode
  path with `d2b.vms.<vm>.graphics.videoSidecar = true`. The path uses
  the vendored patched Cloud Hypervisor `--vhost-user-media` support and a
  patched crosvm `device video-decoder --backend vaapi` runner; no per-VM
  systemd unit or stock-binary fallback is introduced.
- Graphics VMs can opt into experimental guest VA-API video forwarding with
  `d2b.vms.<vm>.graphics.virglVideo = true`. The switch is default-off
  and surfaces a status readiness marker because it enables
  `VIRGL_RENDERER_USE_VIDEO` in the crosvm/virglrenderer GPU path.

### Changed

- **Seccomp BPF programs are now compiled from `ioctl_policy.rs`**
  and loaded by the broker before `execve`; the per-role allowlists
  are the source of truth.
- **Broker pre-NS user namespace** extended to the `swtpm` role
  (full), the `gpu` role (render-node only via `SCM_RIGHTS` fd
  passing), and the `audio` role (owned net-NS). Long-lived sidecars
  now run with zero host capabilities inside the broker-established
  user namespace. See
  [ADR 0021](docs/adr/0021-broker-user-namespace-for-virtiofsd.md).
- **Broker now reaps spawned children** via tokio signalfd +
  `waitid(P_PIDFD)` and reports `ChildReaped` to `d2bd`.
- Bridge IPv6 sysctls (`disable_ipv6 = 1` on `br-*-up`) are now
  applied at boot via `boot.kernel.sysctl`.
- `d2b-priv-broker` may drop `CAP_NET_ADMIN` from its minijail
  bounding set when pre-created TAP fds are passed through.
- `umask` is plumbed end-to-end through `MinijailProfile` →
  `RoleProfile` → `SpawnRunnerPlan`; sidecar profiles default to
  `0o007`.

### Changed (breaking)

- Unified the legacy `d2b-launcher` and `d2b-launchers` Unix
  groups into a single `d2b` group. The activation script re-chgrps
  state files automatically on the next `nixos-rebuild switch` using a
  fd-safe numeric-gid migration helper. Consumer NixOS configs that
  reference the legacy group names in `users.<name>.extraGroups` must
  update to `"d2b"`. Required post-switch step:
  `sudo systemctl restart d2bd.service`. See
  [docs/how-to/migrate-d2b-v1-1-to-v1-2.md](docs/how-to/migrate-d2b-v1-1-to-v1-2.md).
  The broker caller-role audit label remains `"d2b-launcher"` for
  audit-format stability; see
  [docs/reference/naming-conventions.md](docs/reference/naming-conventions.md#broker-caller-role-audit-labels).
  `OperationFields::DeregisterRunnerPidfd { vm_id, role_id }` now
  appears in broker audit logs on successful `vm stop` cleanup for
  per-VM-UID runners; scripts that previously matched the old broker
  error exit see the corrected successful behavior instead.

  Note: the legacy `d2b-launcher` and `d2b-launchers` Unix
  groups remain on the system as empty v1.2 migration tombstones (zero
  membership, gid preserved in `/etc/group`). `getent group
  d2b-launcher` will still return a record with an empty member
  list. They are slated for removal in a v1.3 follow-up.

### Fixed

- Disk-init dispatch: `d2bd` now invokes `BrokerRequest::DiskInit`
  before `SpawnRunner` when the plan node carries plan-ops.
- Overlay disk gets the same CH disk-arg defaults (`direct`,
  `image_type`, `num_queues`) as regular volumes.
- Guest fstab: ro-store virtiofs share mounts at `/nix/.ro-store`
  and the overlay backing disk mounts at `/nix/.rw-store` (both
  `neededForBoot = true`) so initramfs assembles the overlayfs
  correctly.
- `net_route_preflight` now tolerates `NO-CARRIER` state.
- `tests/principal-uid-collision-eval.sh` verifies the
  `stablePrincipalId` hash produces unique UIDs.
- Declared `microvm.volumes` now get stable virtio-blk serials and matching
  guest `fileSystems` mounts. This fixes guests whose persistent `/var`
  volume was attached but not mounted, causing identity-bearing state such as
  `/etc/machine-id`, systemd credentials, and Himmelblau cache data to live on
  tmpfs and change after each VM restart.
  Existing Entra-joined VMs affected by the old behavior may need one final
  enrollment after upgrading if their previous `/var` identity state only ever
  lived on tmpfs; after the persistent `/var` volume is populated, restarts
  should not trigger re-enrollment.
- `d2b vm stop` no longer fails with `pidfd_table SIGTERM failed`
  when the runner runs as a per-VM dedicated UID: the daemon falls back
  to a broker-mediated signal on EPERM and deregisters the broker-side
  pidfd registry after successful termination.
- `d2b vm konsole` no longer reports `ssh key not found` when the
  parent directory is unreadable: the CLI distinguishes ENOENT from
  EACCES and emits an actionable error pointing at `d2b` group
  membership.
- `/var/lib/d2b/` now grants execute-only ACL traversal to the
  lifecycle group so the CLI can resolve keys and bundles without
  widening read access.
- Video sidecars now run as a dedicated `d2b-<vm>-video` principal, and
  activation/broker ACL refreshes deny that principal access to host
  Wayland, PipeWire, and Pulse sockets while preserving GPU cross-domain
  access for `d2b-<vm>-gpu`.

### Documentation

- ADRs 0003, 0011, 0021 received "Updated v1.2" subsections
  describing the broker-pre-NS extensions and reap responsibility.

### Deferred

- Drop the empty `d2b-launcher` and `d2b-launchers` Unix group
  declarations introduced as v1.2 migration tombstones, after one
  release of confirmed clean migration.

## [1.1.2] - 2026-06-02

v1.1.2 closes the v1.1.1 → live-VM bring-up gap by retiring the
`virtiofsd --sandbox=namespace + requiresStartRoot = true` carve-out
from [ADR 0003](docs/adr/0003-minijail-provisioning-and-sandbox-interface.md)
in favour of a broker-pre-established single-entry user namespace
([ADR 0021](docs/adr/0021-broker-user-namespace-for-virtiofsd.md)).

### Changed

- **virtiofsd runs with zero host capabilities** inside a broker
  pre-established user namespace. The broker uses
  `clone3(CLONE_NEWUSER)` and writes `/proc/<pid>/uid_map` before
  execing virtiofsd. `--sandbox=chroot` replaces `--sandbox=namespace`.
- TPM socket moved from `/run/swtpm/<vm>/sock` to
  `/run/d2b/vms/<vm>/tpm.sock`; both halves of the wiring update
  in lockstep on rebuild.
- Sidecar UIDs are now derived from the `stablePrincipalId` hash so
  on-disk owner, ownership-matrix entry, and broker setuid target
  all agree.
- Cloud-Hypervisor 52 is now the required version (variadic
  `--fs sock1,tag1 sock2,tag2` argv form).
- `MinijailProfile` gains an optional `umask: Option<u32>` field;
  sidecar profiles (`swtpm`, `audio`, `gpu`) use `0o007` so bound
  Unix sockets land mode `0660`.

### Fixed

- `ssh_host_key_preflight` accepts mode `0440` when a POSIX ACL
  xattr is present.
- Variadic CH argv emission, absolute `vsockPath`, dev/net/tun bind
  inside the CH sandbox, and several broker child-process robustness
  fixes (tmpfile race in `PidfdTable::snapshot`, zombie detection in
  `wait_for_one_shot_exit`).

### Notes

- `microvm` is no longer required as a consumer flake input.

## [1.1.1] - 2026-06-01

Closes every v1.1 deferral.

### Added

- **`StatusServicesOutputV3`** wire schema with broker-spawn-aware
  fields (`hypervisor`, `virtiofsd_per_share`, `audio`,
  `otel_relay`, `otel_host_bridge`, `usbip_backend_per_env`,
  `usbip_proxy_per_env`). A `from_v2()` conversion shim is exported
  for incremental adoption; the CLI emit-side flip lands in v1.1.2.
- **`d2b vm konsole <vm>`** - opens an SSH session to a VM in a
  host terminal. Resolves the key from the bundle's
  `managed_keys.effective_key_path` and detaches via `setsid`.
- **Atomic cgroup placement** via `clone3(CLONE_INTO_CGROUP)`. New
  per-VM `<slice>/<vm>/<role>/` taxonomy (the per-VM interior node
  stays process-free).
- **USBIP guest attach/detach** routed through hardened SSH argv.
- **Pidfs runtime self-probe**: `d2bd` hard-refuses to start on
  kernels without pidfs unless
  `D2B_ALLOW_PIDFS_PROBE_SOFT_FAIL=1` is set.
- **`RenderDnsmasqEnvConf`** pure-Rust dnsmasq config renderer as a
  broker host-prep op.
- A real syn-based AST walker
  (`tests/tools/no-bash-ast-walker/`) backs
  `tests/no-bash-exec-eval.sh`.

### Fixed

- `fchownat(AT_EMPTY_PATH)` replaces broken `fchown` on `O_PATH`
  descriptors in the cgroup module.

## [1.1.0] - 2026-05-31

Daemon-only follow-through. D2b now owns its per-VM microVM
substrate end-to-end; the `microvm.nix` flake input is gone.

### Added

- **`nixos-modules/vm-options.nix`** declares the per-VM option set
  (hypervisor, vcpu, mem, kernel, shares, devices, volumes, …).
- **`nixos-modules/vm-evaluator.nix`** evaluates per-VM modules with
  the upstream NixOS evaluator (`eval-config.nix`). The
  `d2b.vms.<vm>.computed` option exposes the result.
- Rust runner-argv generators in `packages/d2b-host/`
  (cloud-hypervisor, virtiofsd, swtpm, gpu, audio, usbip,
  vsock-relay, otel-host-bridge) are now the canonical argv source.
- Typed CLI envelopes for `daemon-down` (exit 1) and
  `not-yet-implemented` (exit 78). The Rust CLI never invokes bash.

### Removed

- `microvm.nix` flake input dropped from `flake.nix`. Consumers who
  only inherited the input via `d2b.inputs.microvm.follows = …`
  need no flake change; consumers who declared `microvm.url`
  themselves can drop the input if they don't use microvm directly.
- `d2b.vms.<vm>.supervisor` option removed. Setting it now
  fails eval with a typed friendly message.
- `d2b-vfsd-watchdog@.{service,timer}` retired (wedge detection
  moved into the broker's virtiofsd `SpawnRunner` pidfd supervisor).
- `host-otel-relay-acl.nix` retired; OTel host-bridge ACL moved
  into the broker pre-spawn pipeline.

### Changed

- Kernel floor uplifted to **Linux ≥ 6.9** (`pidfs`-backed pidfd
  identity is required). See
  [ADR 0008](docs/adr/0008-supported-platforms-and-rejected-targets.md).
- `d2b.daemonExperimental.enable` is now obsolete and a no-op;
  the broker socket/service are enabled by default. The option name
  remains evaluable, with a warning when set.
- New invariant gates: `no-bash-exec-eval`,
  `supervisor-option-absent-eval`, `broker-systemd-unit-eval`,
  `daemon-experimental-warning-eval`, `state-dir-acl-eval`,
  `otel-acl-migration-eval`, `vfsd-watchdog-retired-eval`,
  `processes-json-eval`, `vm-submodule-eval`,
  `kernel-modules-parity-eval`, `vm-submodule-cutover-eval`,
  `v1.1-kernel-floor-eval`, `microvm-nix-absent-eval`.

## [1.0.0] - 2026-05-31

Daemon-only end-state per
[ADR 0015](docs/adr/0015-daemon-only-clean-break.md). Clean break
from the v0.x bash CLI + per-VM systemd templates: `d2bd` and
`d2b-priv-broker` are the only persistent root surfaces.

### Removed (breaking)

- **Bash CLI deleted.** `nixos-modules/cli.nix`, the
  `share/d2b/cli.sh` entrypoint, and every bash subcommand are
  gone. The Rust `d2b` binary is the sole CLI; there is no
  fallback bridge. `D2B_LEGACY_BASH_OPT_IN` and
  `D2B_NATIVE_ONLY` are no-ops.
- **Per-VM systemd templates retired.** `d2b@<vm>.service`,
  `d2b-<vm>-{gpu,swtpm,video,snd}.service`, and
  `d2b-known-hosts-refresh@<vm>.service` are deleted. Every
  per-VM lifecycle step runs inside `d2bd`'s DAG executor;
  spawned runners (cloud-hypervisor, virtiofsd, swtpm,
  vhost-user-sound, USBIP attach) are launched by the broker's
  `SpawnRunner` op and handed back as pidfds via `OpenPidfd` /
  `SCM_RIGHTS`.
- **Host singletons retired.**
  `d2b-audit-check.{service,timer}`,
  `d2b-ch-exporter.service`,
  `d2b-net-route-preflight.service`,
  `d2b-otel-host-bridge.service`, and per-env
  `d2b-sys-<env>-usbipd-*` units are deleted. Their work moved
  into `d2bd` (Prometheus exposition, net-route preflight,
  USBIP state machine) or into broker ops (`ExportBrokerAudit`,
  `UsbipBindFirewallRule`, `SpawnRunner{role: Usbip}`).
- **Polkit per-VM allowlists removed.** `d2b-launchers` group
  membership + `SO_PEERCRED` on `public.sock` is the only lifecycle
  authorisation surface.

### Changed (breaking)

- **Manifest `manifestVersion`: 2 → 3.** No compatibility window;
  the daemon and CLI reject v2 bundles with
  `manifest-version-mismatch`. Operators must rebuild the manifest.
- **Cgroup v2 slice** consolidated to a single `d2b.slice`
  delegated to the `d2bd` uid by the broker; see
  [ADR 0011](docs/adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md).
- `d2b_host::DeviceClass` gained `Udmabuf` for GPU sidecar
  ioctls; `modules_disabled` is fail-closed in the broker's
  `ModprobeIfAllowed` path.

### Added

- **`d2b host validate` / `host reconcile`** - host-side
  preflight + degraded-mode recovery for the daemon's net-route
  monitor.
- **Broker audit** (`OpAuditRecord`) at
  `/var/lib/d2b/audit/broker-<utc-date>.jsonl`
  (`0640 root:d2bd`, append-only, daily rotation, 14-day
  retention by default; override with
  `d2b.site.audit.retentionDays`).
- **`docs/how-to/migrate-d2b-v0-to-v1.md`** is the
  operator-facing migration guide.

## [0.3.0] - 2026-05-24

Minor release adding **hardware-accelerated H264 video decode** for
RDP sessions inside graphics VMs. A new virtio-media pipeline
offloads H264 decode from guest CPU to host NVDEC hardware via a
multi-component stack: guest ffmpeg h264_v4l2m2m → /dev/video0 →
chromeos/virtio-media kernel driver (device ID 48) → Cloud
Hypervisor `--vhost-user-media` → crosvm vhost-user video-decoder →
VA-API → nvidia-vaapi-driver → NVDEC. The pipeline activates
automatically when the RDP server negotiates AVC420/AVC444 codec;
ClearCodec sessions fall back to software decode transparently.

### Added

- **Dedicated CH `--vhost-user-media` device type**
  (`0003-vhost-user-media-device.patch`, 1104 lines across 10 CH
  source files). Modeled on the GPU device's VirtioDevice
  implementation with BackendReqHandler for shmem_map/shmem_unmap,
  memfd-backed 256 MB SHM PCI BAR, read_config proxying, and a
  vring_bases fix that forces `SET_VRING_BASE(0)` on initial
  activation - working around a CH bug where it reads `avail_idx`
  from guest memory, skipping buffers the driver pre-queued before
  `DRIVER_OK`.
- **Crosvm vhost-user video-decoder backend**
  (`pkgs/vhost-user-video/`). Implements `VhostUserDevice` for
  virtio-media, wrapping `VirtioVideoAdapter` + `VideoDecoder` with
  `VirtioMediaDeviceRunner`. Worker loop matches crosvm's built-in
  media.rs reference. Supports VA-API and FFmpeg decoder backends.
- **virtio-media guest kernel module**
  (`pkgs/virtio-media-driver/`). Builds chromeos/virtio-media
  out-of-tree for kernel 6.18, pinned to commit `ebcef1a`.
- **Video sidecar systemd service** (`video/host.nix`). Per-VM
  `d2b-<vm>-video.service` running as the GPU sidecar user with
  VA-API environment (LIBVA_DRIVER_NAME=nvidia,
  NV_VAAPI_BACKEND=direct). Lifecycle bound to GPU service via
  `partOf`.
- **FreeRDP h264_v4l2m2m integration** (work-aad.nix). Patches
  FreeRDP to prefer `h264_v4l2m2m` decoder with fallback to software,
  removes YUV420P format override, adds thread-local NV12→YUV420P
  deinterleave for v4l2m2m's NV12 output.
- **devbox-connect AVC enablement**. Injects `use video codec:i:2`
  into .rdp files, adds `/gfx:AVC420:on` to FreeRDP command line,
  and auto-sets Windows registry keys for AVC444 software encoding
  via `/shell` on connect.

### Fixed

- **EventQueue deadlock** in vhost-user mode. Upstream
  `EventQueue::send_event()` blocks with `event().wait()` on the
  event queue kick eventfd. Fixed by adding a non-blocking
  `reset()` + `pop()` before the blocking wait.
- **SET_VRING_BASE race**. CH reads `avail_idx` from guest memory
  at activate time, but the virtio-media driver pre-queues 16 event
  buffers before `DRIVER_OK`, making them invisible. Fixed by
  forcing `vring_bases = vec![0; N]` in the media device's
  `activate()`.
- **Video socket startup race**. The GPU service's socket wait loop
  now exits non-zero if the video socket doesn't appear within 10
  seconds, preventing CH from starting with a missing socket.
- **crosvm decoder_adapter panics**. `ResetCompleted` and
  `NotifyError` events now log and continue instead of `todo!()`
  crashing the sidecar.

### Removed

- Dead files from abandoned approaches: virtio-video driver
  (device ID 31), 4 kernel compat patches, USERPTR patches for
  ffmpeg and virtio-media, old crosvm/FreeRDP patch files,
  kernel-v4l2-m2m-prompt.patch (10 files, 977 lines).

### Security

- NV12 scratch buffers in FreeRDP decompress changed from `static`
  globals to `_Thread_local` to prevent data races between
  concurrent decoder contexts.
- Video sidecar socket wait hardened with non-zero exit on timeout.
- Video sidecar lifecycle bound to GPU service via `partOf`.

## [0.2.0] - 2026-05-20

Minor release introducing the **observability subsystem**: a new
opt-in component category that provisions a single-host telemetry
sink VM (`sys-obs-stack`) wired over virtio-vsock - no IP between
the observer and the observed VMs, no shared SSH credentials. The
release ships per-VM Alloy agents, a Cloud Hypervisor metrics
exporter, host-side journald forwarding, 6 provisioned Grafana
dashboards, 8 Prometheus alert rules, and `otel-cli` helpers that
stamp local trace IDs onto CLI lifecycle events for correlation.
The stock host setup still keeps the OTLP receiver on a Unix
socket, so Tempo export remains an opt-in follow-up rather than a
default-on path. Manifest schema bumped from version 1 to 2 to add the
`_observability` reserved sentinel and per-VM `observability`
block. A new `AGENTS.md` policy documents the repository's contributor
validation expectations for multi-phase plans.

### Added

- **Observability subsystem** (`d2b.observability.enable`,
  default `false`). When enabled, the framework auto-declares the
  `obs` env (default `lanSubnet = 10.40.0.0/24`,
  `uplinkSubnet = 203.0.113.0/30`) and the `sys-obs-stack` VM that
  runs Grafana + Prometheus + Loki + Tempo + a central Alloy OTLP
  receiver. Retention defaults: metrics 30d, logs 14d, traces 7d
  (all per-knob configurable via
  `d2b.observability.retention.{metrics,logs,traces}`).
- **Per-VM guest agent** (opt-in via
  `d2b.vms.<vm>.observability.enable`). Each monitored guest
  runs Alloy scraping node metrics + journald (each
  individually toggleable via
  `vm.observability.{scrapeJournal,scrapeNodeMetrics}`), receives
  in-VM OTLP on a UDS, and exports over virtio-vsock through the
  hardened `d2b-otel-vsock-out.service` (socat sidecar:
  `RestrictAddressFamilies=[AF_UNIX AF_VSOCK]`,
  `DeviceAllow=/dev/vsock`, `restartIfChanged=false`).
- **Host-side forwarder** (`services.alloy` on the host, forwarder
  mode, no storage). Scrapes d2b sidecar units' journald + node
  metrics + the loopback CH-exporter `/metrics`. Pushes all signals
  through `d2b-otel-host-bridge.service` to the obs VM.
- **Cloud Hypervisor metrics exporter**
  (`d2b-ch-exporter.service`, pure-Bash + jq + curl + socat -
  no new language runtime in the host closure). Polls each VM's CH
  REST socket (`/vmm.ping`, `/vm.info`, `/vm.counters`), exposes
  Prometheus text on the historical loopback collector URL. Counter allowlist
  pinned to Cloud Hypervisor v50 device IDs (`_net*`, `_disk*`,
  `_fs*`, `_pmem*`, `__rng`, `__balloon`, `__console`); unknown
  schema rolls into `d2b_vm_unknown_counters_total`. Topology
  labels (`bridge`, `tap`, `tpm`, `graphics`, `audio`,
  `usbip_yubikey`) are off by default to keep the security-posture
  surface narrow - flip
  `d2b.observability.ch.exporter.includeTopologyLabels` on for
  debug. Detects both `microvm@<vm>.service` and
  `d2b-<vm>-gpu.service` so graphics VMs are reported running.
- **Vsock transport** - no IP between VMs, no SSH credentials
  between observer and observed. Cloud Hypervisor `--vsock cid=N,...`
  is appended to every observability-enabled VM and to
  `sys-obs-stack`; a per-VM `d2b-otel-relay@<vm>.service` (socat
  host relay, `RestrictAddressFamilies=[AF_UNIX]`) stitches
  workload-VM vsock to obs-VM vsock at the host. Relay is wired
  via `microvm@%i.service.wants` for headless VMs and via
  per-VM `wants` on `d2b-<vm>-gpu.service` for graphics VMs
  (graphics VMs do not use `microvm@`).
- **CLI lifecycle telemetry** - `d2b up/down/switch/boot/test/
  rollback/gc/usb/audio` emit OTel spans via `otel-cli` and
  structured JSON journald events for every high-value lifecycle
  step. Spans are populated with allowed labels only (`vm.name`,
  `vm.env`, `vm.role`, `d2b.subcommand`, `systemd.unit`, `tap`,
  `bridge`, `static_ip`, `generation`) - never command output, key
  paths, or Nix store paths. `d2b_span_start` generates `trace_id` +
  `span_id` locally via `/dev/urandom` so Loki↔Tempo correlation
  works even when no upstream OTLP collector endpoint is configured;
  honors otel-cli's traceparent when one is. `otel-cli` is
  module-time-gated into `runtimeInputs` via
  `d2b.observability.cli.traces.enable` (default `true`); hosts
  with observability disabled pay zero closure cost.
- **6 provisioned Grafana dashboards** under the "D2b" folder:
  D2b Overview, VM Resources, Lifecycle Traces, Logs, Per-VM
  Store, Obs VM Health. Default refresh 30s. Tempo→Loki
  trace-to-logs correlation via `derivedFields`.
- **8 Prometheus alert rules**: `D2bVMDown`,
  `D2bNetVMDownWithRunningWorkloads`,
  `D2bObsVMUnreachableFromHost`, `D2bVsockRelayDown`,
  `D2bCHAPISocketMissing`, `D2bStoreSyncFailure`,
  `D2bGuestTelemetryMissing`, `D2bObsVMStackUnhealthy`.
  Each rule individually toggleable via
  `d2b.observability.alerts.<name>.enable`. Notification
  channels are intentionally unconfigured - operators choose
  Alertmanager / Grafana contact-points.
- **Grafana auth**: defaults to authenticated access as
  `d2b-admin`. Password is generated at activation and stored
  at `/var/lib/d2b-observability/grafana-admin-password` inside
  `sys-obs-stack`, or sourced from sops/agenix via
  `d2b.observability.grafana.adminPasswordFile`. Session signing
  key follows the same pattern via
  `d2b.observability.grafana.secretKeyFile`. Anonymous Viewer
  is opt-in only for trusted single-host LANs via
  `d2b.observability.grafana.anonymousViewer.enable`; the login
  form remains available even in that mode.
- **Eval assertions**: vsock CID uniqueness across enabled VMs
  (reserved CID 1000 for `d2b.observability.vmName`),
  per-VM-without-framework rejection, reserved-prefix exemption for
  `cfg.vmName`, env uplink CIDR materialization check.
- **Tests**: `tests/observability-eval.sh` (23/23 cases, 1 promtool
  skip when absent - covers option schema, auto-declaration,
  CID allocation, per-VM toggle defaults, name/prefix collisions,
  CLI-traces closure gating, relay ACL wiring, stack VM guest
  surface, dashboard schema validation, rule-file `promtool`
  validation, metric-reference coverage, scrape-job exact-set,
  and the graphics-VM runner wiring path).
- **Examples**: `examples/with-observability/` minimal consumer
  flake validated by the per-example flake-check loop.
- **Docs**:
  - `docs/reference/components-observability.md` - option schema,
    port/CID/UDS table, naming conventions, systemd unit
    inventory, dashboard inventory, alert severity table,
    security boundaries, label conventions, retention defaults,
    opt-out paths.
  - `docs/how-to/enable-observability.md` - step-by-step recipe
    including sops/agenix examples for both the Grafana
    secret-key and admin-password.
  - `docs/explanation/design.md` - appended Observability section
    explaining the vsock-vs-reverse-SSH-vs-guest-init trade-off,
    the two-bridge necessity, the alternatives-considered list,
    CLI attribute hygiene, and the trust-concentration risk on
    the obs VM.
  - `docs/reference/manifest-schema.md` - `manifestVersion = 2`
    rationale.

### Changed

- **`manifestVersion` 1 → 2** (breaking under pre-1.0 minor-bump
  policy). The manifest now ships a top-level `_observability`
  reserved sentinel and a per-VM `observability` block
  (`enabled`, `vsockCid`, `vsockHostSocket`). Existing consumers
  who do not enable `d2b.observability.enable` see the new
  fields populated with `enabled = false` defaults - the
  manifest still describes their VMs deterministically.
- **`docs/reference/manifest-schema.{md,json}`** updated to
  describe the v2 schema.

### Security

- Telemetry sidecar trust posture: dedicated locked system users
  (`d2b-otel-relay`, `d2b-otel-bridge`,
  `d2b-ch-exporter`) with execute-only ACLs on per-VM state
  directories and `rw` ACLs only on the per-port vsock sockets
  they need (`vsock.sock_14317`, not the base `vsock.sock`).
  Activation-time ACL refresh is idempotent and revokes stale
  grants when an observed VM is later disabled.
- `d2b-otel-acl-refresh` rejects symlinked state paths,
  validates resolved paths stay under the state root, and uses
  `setfacl --physical` when available - closes the TOCTOU
  window on a group-writable state tree.
- Grafana `secret_key` and admin password are never written to
  the world-readable Nix store. Both are generated atomically at
  activation (write-to-tmp + `mv -f`) and loaded via systemd
  `LoadCredential` into `/run/credentials/grafana.service/`, or
  sourced from operator-supplied files via
  `d2b.observability.grafana.{secretKeyFile,adminPasswordFile}`.
- Loki query selectors in shipped dashboards never default to a
  whole-namespace scan: every variable-driven selector requires
  a non-empty match (`.+`, not `.*`), and the trace-to-logs
  derivedField is scoped by trace-derived `vm`/`env` labels.
- Alert annotation templates carry `vm` and `env` only; full
  unit/job names stay inside dashboards (not exported to
  whichever notification backend an operator wires up).
- CLI span attribute extras are filtered through an allowlist
  in `d2b_filter_attrs`: caller-supplied keys outside
  `{step, result, systemd_unit, tap, bridge, static_ip, generation,
  vm_role}` are dropped with a journald warning, as are values
  matching common secret/store-path patterns.
- The guest UDS→vsock relay is fork-bounded
  (`max-children=16`, `TasksMax=32`, `MemoryMax=64M`,
  `LimitNOFILE=1024`) to bound in-guest DoS surface.
- The host telemetry bridge runs as `alloy` with
  `SupplementaryGroups=[kvm]` (no over-broad `d2b-otel-host-bridge`
  user) and connects to a narrowed
  `/run/d2b/alloy/` subdirectory rather than the shared
  `/run/d2b/` root.
- Documented trust-concentration risk: `sys-obs-stack` has read
  access to every monitored VM's telemetry; treat as privileged
  infrastructure. Single-host single-VM by design (multi-host
  is explicitly out of scope for v0.2.0).

### Deferred to v0.3.0

- **`D2bVMStuckWithoutSSH` alert** - needs a new
  CH-exporter metric (`d2b_vm_ssh_ready`) before the rule
  can be defined non-trivially.
- **`d2b_vm_store_path_count`** - the Per-VM Store
  dashboard references this metric today but it is currently
  **future-work absent**: no exporter emits it yet. The dashboard
  panel renders empty until a future store-path-count exporter
  lands (planned for v0.3.0). The `obs-metric-references`
  test gate treats it as a documented future-work exception
  rather than an unknown metric.
- **`d2b_vm_counter_net_tx_bytes` and
  `d2b_vm_counter_net_rx_bytes`** - referenced by the VM
  Resources network panel for legacy compatibility; the actual
  emitted metric names are `d2b_vm_counter_virtio_net_*`
  (CH v50 device naming). Documented as **future-work absent**
  pending dashboard query simplification - both legacy and
  modern names will resolve via Prometheus `or` until the legacy
  names are removed.
- **Stable relay-binary interface.**
  `d2b.observability.transport.relayPackage` still
  requires a `bin/socat`-compatible CLI today. Non-socat
  relays need a dedicated compatibility interface before the
  socat-compatible path can be removed.
- **VM-runner abstraction.** Today the framework leaks the
  runner-unit name (`microvm@<vm>` for headless,
  `d2b-<vm>-gpu` for graphics) into the relay wiring, and
  the observability code has to wire to both. A runner-agnostic
  abstraction is required before per-VM sidecar wiring can stay
  on a single name.


### Changed

- **sshd host keys are now generated on the HOST and shared into
  every guest read-only via virtiofs.** A new module
  `nixos-modules/host-ssh-host-keys.nix` provisions per-VM ed25519
  host keys at host activation under
  `${d2b.site.stateDir}/vms/<name>/sshd-host-keys/` (mode 0400
  root:root). `nixos-modules/store.nix` shares the directory into
  the guest at `/run/d2b-sshd-host-keys/` (virtiofs tag
  `d2b-ssh-host`). A new `nixos-modules/guest-sshd-host-keys.nix`,
  imported into every enabled VM by `host.nix`, points
  `services.openssh.hostKeys` at the shared path and disables the
  NixOS `ssh-keygen -A` activation hook. **Why**: pre-v0.2.0 each
  guest regenerated its sshd host keys on first boot and stored
  them on the tmpfs overlay over the read-only nix store, so they
  were ephemeral. Every VM restart regenerated them, the host's
  `known_hosts.d2b` pinned the first observed set and refused
  to overwrite subsequent ones (correctly: from the host's point
  of view, a host-key change IS a possible MITM/swap), and
  operator SSH from the host would soft-brick until manual
  `ssh-keygen -R` + a refresh-service kick. Host-managed keys
  eliminate the drift class entirely.
- **`nixos-modules/host-known-hosts.nix`**: the refresh script
  now reads the host-side `.pub` file directly instead of probing
  the live VM with `ssh-keyscan`. Faster (no boot wait), immune
  to the live-vs-pinned drift the old logic had to handle (a VM
  restart used to regenerate the in-VM key every time).
- **Observability admin password + secret key are now generated
  on the HOST, not inside `sys-obs-stack`.** A new module
  `nixos-modules/observability-host-secrets.nix` provisions both
  files at host activation under
  `${d2b.site.stateDir}/observability/` (default
  `/var/lib/d2b/observability/`, mode 0400 root:root) and
  shares them read-only into the stack VM via virtiofs at
  `/run/d2b-obs-secrets/`. The in-VM activation scripts that
  used to generate these secrets in
  `/var/lib/d2b-observability/` (inside `sys-obs-stack`) have
  been removed. **Why**: putting both secrets inside the VM
  pointed the trust flow the wrong way - anything on the host
  that needed the Grafana admin password (a launcher, a health
  probe, a backup) had to cross the VM boundary to read it, which
  in practice forced consumers to add an SSH-able operator
  account + sudoers rule inside `sys-obs-stack` just to claw the
  password back out. With this change, host-side
  `sudo cat ${d2b.site.stateDir}/observability/grafana-admin-password`
  is the supported path; no operator account inside the stack VM
  is required. The `d2b.observability.grafana.{secretKeyFile,
  adminPasswordFile}` overrides still work for sops-nix / agenix
  users.
- **Consumer extensions of the auto-declared observability VM are
  now allowed.** The pre-v0.2.0 assertion that rejected any
  user-side definition under `d2b.vms.<obsCfg.vmName>` was
  removed. The framework's auto-declaration block uses
  `lib.mkDefault` for every value, so a consumer override
  (e.g. `d2b.vms.sys-obs-stack.ssh.user = "root"`) merges
  cleanly. The matching `assertions-eval.sh` test was renamed to
  `observability-vmname-extension-allowed` and asserts the new
  behaviour.
- **Default obs-VM memory bumped 512 M → 2048 M.** Grafana
  alone wants ~200 M RSS on idle; the full
  Grafana+Prom+Loki+Tempo+Alloy stack in a single VM tripped the
  in-VM OOM killer within seconds of boot at the previous 512 M
  default. 2 GiB is the minimum that lets the whole stack come
  up with default retention windows on a single-host install
  monitoring ~tens of VMs. `lib.mkDefault` so operators can
  override either way.
- **`services.alloy` /run/d2b/alloy via `RuntimeDirectory`,
  not tmpfiles**, on host + every guest + stack VM. The previous
  tmpfiles rule could not chown to the DynamicUser-allocated
  `alloy` UID at activation time; the directory either never
  appeared or was owned by `nobody:nogroup`, breaking
  `d2b-otel-host-bridge` setfacl + alloy's writability
  expectations.
- **Alloy `labels = { ... }` map literals updated with trailing
  commas** in `components/observability/{host,guest}.nix`. Alloy
  DSL distinguishes between newline-separated *blocks* (no `=`)
  and comma-separated *map literals* (with `=`); the latter were
  emitted without commas and rejected by Alloy's parser at boot.
- **`host-otel-relay-acl` + `host-ch-exporter`**: added
  `excludeShellChecks = [ "SC2034" ]` for bash namerefs and
  positional placeholders in `read`. Both scripts use shell
  patterns shellcheck cannot follow; the warnings became fatal
  the moment `writeShellApplication` actually built them in a
  consumer rebuild.
- Eval test `obs-stack-vm-guest-surface: grafana LoadCredential
  wires secret_key credential file` updated to assert the new
  in-VM source path
  `/run/d2b-obs-secrets/grafana-secret-key` (was the in-VM
  `/var/lib/d2b-observability/grafana-secret-key`).

### Migration

- Fresh installs land on the new layout with no operator action.
- Pre-existing installs that booted v0.2.0 with the in-VM
  observability secret generator will see a **password rotation**
  at the next `nixos-rebuild switch`: the new host-generated
  secret displaces the old in-VM one. Operators should fetch the
  new password via
  `sudo cat /var/lib/d2b/observability/grafana-admin-password`
  on the host.
- Pre-existing installs that had ephemeral in-VM sshd host keys
  pinned in `/var/lib/d2b/known_hosts.d2b` will see a
  **one-time host-key change** for every VM at the next
  activation+restart: the host now generates a stable ed25519
  host key per VM and the refresh service swaps the pinned entry
  on the next `microvm@<vm>` start. The framework handles this
  automatically; operator SSH clients (outside the framework)
  may need a one-time `ssh-keygen -R <ip>` against their personal
  `~/.ssh/known_hosts` if they manually trusted the old key.


### Fixed

- **`nixos-modules/host-keys.nix`**: per-VM `.desktop` launchers
  failed with "Permission denied" on the SSH private key because
  the keys directory (`/var/lib/d2b/keys/`) lacked a traverse
  ACL for `d2b-launcher`. The directory had a
  `group:d2b-launcher:--x` ACL entry, but both the tmpfiles
  rule and the activation script's `install -d -m 0700` set the
  directory mode to `0700`, which forces the POSIX ACL mask to
  `---` and neutralizes the named-group entry. Fix: add
  `setfacl -m "g:d2b-launcher:--x"` on the keys directory
  in the activation script, after the `install -d`, so the mask
  is recalculated to include `--x`.

- **`nixos-modules/host-known-hosts.nix`** + **`nixos-modules/cli.nix`**
  (`vmLaunchScript`): graphics-VM per-VM `.desktop` launchers
  silently did nothing when the pinned host key in
  `known_hosts.d2b` was stale. Two coupled bugs:
  1. `d2b-known-hosts-refresh@%i.service` was wanted only by
     `microvm@%i.service`, but graphics VMs bypass that template
     (the GPU sidecar runs cloud-hypervisor directly). The
     refresh therefore only fired during `nixos-rebuild`
     activation - often tens of minutes before the user actually
     launched the graphics VM - and every one of those
     activation-time refreshes timed out because the VM wasn't
     running yet. The pinned key stayed stale across rebuilds.
     Fix: also `Wants=d2b-known-hosts-refresh@<vm>.service`
     from `d2b-<vm>-gpu.service` for graphics-enabled VMs,
     with a matching `After=d2b-%i-gpu.service` on the
     refresh template.
  2. `vmLaunchScript` (`cli.nix`) ran a 30 s ssh-readiness probe,
     discarded its stderr, did not track success/failure, and
     unconditionally `exec`'d `konsole -e ssh …`. With a stale
     pin every probe failed silently with
     `Host key verification failed!`; konsole then exec'd into an
     immediately-failing ssh and closed - observed by the user as
     the launcher "doing nothing" whether the VM was up or down.
     Fix: track probe success, classify the failure on timeout
     (host-key mismatch vs. unreachable), and surface
     `notify-send` with the exact remediation command (host-key
     case points at
     `sudo systemctl start d2b-known-hosts-refresh@<vm>.service`).

## [0.1.7] - 2026-05-19

Patch release. Review of v0.1.6 caught a silent bug in the
v0.1.5 lifecycle policy: three of the six per-VM sidecars used
`unitConfig.X-RestartIfChanged = false` instead of the top-level
NixOS option `restartIfChanged = false`. The two forms LOOK
equivalent and both compile to a setting on the unit file -
but NixOS's `switch-to-configuration` logic only reads
`X-RestartIfChanged=` from the `[Service]` section. The
`unitConfig.X-RestartIfChanged` form emits under `[Unit]`,
where it is silently ignored. Result: pre-v0.1.7, every
`nixos-rebuild switch` that touched the GPU, swtpm, or snd
sidecar config STILL cycled those sidecars under the running
VM, defeating the v0.1.5 policy on the exact services whose
restart causes the most damage (CH termination, TPM socket
loss, audio sidecar disconnect).

### Fixed

- **`nixos-modules/host-sidecars.nix`** (swtpm + GPU sidecars):
  replaced `unitConfig.X-RestartIfChanged = false` with
  top-level `restartIfChanged = false`.
- **`nixos-modules/components/audio/host.nix`** (snd sidecar):
  same fix.
- **`tests/restart-policy-eval.sh`** (regression added in v0.1.6):
  tightened the predicate to REJECT `unitConfig.X-RestartIfChanged`.
  The previous version accepted either form, so it would have passed
  against the v0.1.5/v0.1.6 broken setup. Now any service using the
  broken form fails the test with an explicit message pointing at
  this CHANGELOG entry.
- **AGENTS.md** "Adding new per-VM units" guidance: explicitly
  forbids `unitConfig.X-RestartIfChanged`; mandates the
  top-level `restartIfChanged = false` form.
- **`docs/reference/components-{graphics,tpm,audio}.md`**:
  updated lifecycle subsections to reference the corrected
  form. The Lifecycle section subheaders still call this
  v0.1.5+ behaviour because the policy was always v0.1.5;
  v0.1.7 is just making the v0.1.5 intent actually work.

### Verification

The three sidecar files now match the pattern already used in
`host-wrapper.nix`, `host-known-hosts.nix`, and `store.nix`
(`restartIfChanged = false` at the top level). The
`tests/restart-policy-eval.sh` gate now asserts the correct
form on all 6 services and would have caught the v0.1.5 bug
at landing time. All other v0.1.6 gates remain green.


## [0.1.6] - 2026-05-19

Docs catch-up release. The v0.1.1-v0.1.5 patches shipped fixes for
five framework bugs surfaced during the first real consumer
migration, but the public docs hadn't been updated to describe the
resulting behavior changes. This release brings the docs in sync
with the code, plus a small audit-strict fix that completes
`v0.1.4`'s skip-stopped-VMs work, tightens the autostart wiring,
and adds regression tests for every v0.1.x patch.

### Changed

- **`d2b list` status label**: `[pending switch]` →
  `[pending restart]`. The label tracks the *recommended action*,
  and the recommended action for unit-file drift after a host
  `nixos-rebuild switch` is `d2b restart <vm>` (clean down+up
  cycles the running closure over the staged unit files); `d2b
  switch <vm>` is the heavier per-VM-closure-rebuild path for
  VM-NixOS-module edits. CLI messages in `d2b status` and the
  `d2b list` trailer updated to match.

- **`systemd.targets.microvms.wants` is now `lib.mkForce []`** on
  every consumer. Previously v0.1.3 narrowed the list to
  autostart=true VMs; v0.1.6 narrows further to `[]` so all
  autostart wiring goes through `systemd.targets.multi-user.wants
  -> d2b@<vm>.service` exclusively. Removes the duplicate
  boot path (target.wants pulling `microvm@<vm>` directly,
  bypassing the framework wrapper).

### Added (assertions)

- **`graphics.enable + autostart` is now an eval-time error.** A
  graphics VM with `autostart = true` would boot through the
  upstream microvm@<vm> runner without the GPU sidecar's
  Wayland-socket bind, leaving the VM with no display. The
  assertion's remediation message points at `d2b up <vm>`
  from a Plasma terminal.

### Added (tests)

- `tests/unit/smoke/smoke-eval-extraspecialargs.nix` - regression for v0.1.1
  `extraSpecialArgs` propagation through `nixos-modules/host.nix:165`.
- `tests/net-vm-network-eval.sh` extended - regression for v0.1.2
  `ConfigureWithoutCarrier` + route entry on the host's uplink bridge.
- `tests/autostart-wiring-eval.sh` - covers `d2b@<vm>` as
  template-only, multi-user.target.wants wiring, and
  `microvms.target.wants == []`.
- `tests/unit/smoke/smoke-eval-graphics.nix` extended - regression for v0.1.4
  `/dev/net/tun rw` in the GPU sidecar's DeviceAllow.
- `tests/unit/smoke/smoke-eval-tpm.nix` - regression for v0.1.4 swtpm parent-dir
  ACL traversal grant.
- `tests/restart-policy-eval.sh` - regression for v0.1.5
  `restartIfChanged = false` across all six services.
- Negative-assertion regression in `tests/assertions-eval.sh`
  (`test_graphics_with_autostart`).

### Added (docs)

- **`docs/reference/cli-contract.md`** documents:
  - `d2b restart <vm> [--force]` (v0.1.5)
  - `pending-restart` indicator semantics in `d2b list` /
    `d2b status` (v0.1.5)
  - `d2b.site.extraSpecialArgs` consumer-side escape hatch
    (v0.1.1)

- **`docs/explanation/design.md`**:
  - New "VM lifecycle policy" section explaining
    `restartIfChanged = false` on all per-VM units, the
    `booted`/`current` symlink contract, and how
    `pending-restart` is computed (v0.1.5).
  - New "Per-env bridge bootstrap" subsection covering the
    `ConfigureWithoutCarrier = true` requirement on the uplink
    bridge and how it breaks the route-preflight deadlock at
    boot (v0.1.2).
  - New "GPU sidecar substitutes microvm-run" subsection
    explaining why the GPU sidecar carries `DeviceAllow=/dev/net/tun`
    (v0.1.4), the `microvm-set-booted`-equivalent ExecStartPre
    (v0.1.5), and the swtpm-user ACL grant (v0.1.4).
  - "Why not X" - new FAQ entry: "Why doesn't `nixos-rebuild
    switch` restart VMs?", cross-linking to the cli-contract's
    pending-restart predicate.
  - Removed `tests/static.sh doesn't iterate examples` and
    `ROOT defaults to /etc/nixos` from "Limitations / known
    gaps" (resolved).

- **`docs/how-to/migrating-from-microvm.md`**:
  - Required minimum `d2b = github:vicondoa/d2b/v0.1.6`
    (or later) - earlier versions exposed framework bugs that
    blocked real-world graphics + TPM bring-up. (Aligned with
    the CHANGELOG; v0.1.6 is the first release where the docs
    match the shipping code.)
  - New "After every rebuild" step in the procedure: check
    `d2b list` for `[pending restart]`, apply with
    `d2b restart <vm>`. Cross-links to the cli-contract's
    pending-restart section.
  - New troubleshooting note: `d2b status <vm>` shows
    `booted` vs `current` mismatch and the exact remediation
    command.

- **`docs/reference/components-graphics.md`**:
  - Added `/dev/net/tun rw` to the documented DeviceAllow list,
    with the rationale (cloud-hypervisor attaches to the tap
    upstream microvm.nix's `microvm-tap-interfaces@<vm>.service`
    helper created).
  - New "Lifecycle" subsection: GPU sidecar IS the
    cloud-hypervisor process; `restartIfChanged = false` keeps
    rebuilds from killing the VM.

- **`docs/reference/components-tpm.md`**:
  - Added the ACL traversal grant on the parent state dir to
    the documented host-side resources. No manual `chown`
    required for v0.1.4+ consumers - the framework's
    `d2bVmStatePerms` activation script handles it.
  - Updated the "DO NOT WIPE" warning to also point at the
    `pending-restart` indicator as the right signal for
    "TPM-bound creds may be re-read after restart".
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `d2b-<vm>-swtpm.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`docs/reference/components-audio.md`**:
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `d2b-<vm>-snd.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`AGENTS.md`**:
  - New "VM lifecycle policy" subsection documenting
    `restartIfChanged = false` as a framework invariant for
    contributors.
  - New convention: per-VM `wantedBy` ALWAYS via
    `systemd.targets.multi-user.wants` symlinks, never via
    per-instance `systemd.services."d2b@${name}"`
    declarations (which NixOS materializes as separate unit
    files lacking the template's lifecycle hooks).

- Example READMEs (`minimal`, `graphics-workstation`, `multi-env`,
  `with-entra-id`) gain a short "After subsequent rebuilds"
  cross-link block pointing at the template README's post-rebuild
  section.

- Plan/spec corrections (#30-#38) tracking the v0.1.x patches
  plus the v0.1.6 follow-up sweep.

### Fixed

- **`nixos-modules/cli.nix`** (`audit --strict`): the
  `bridge_isolated_workload.<vm>` skip-when-down predicate (added
  in v0.1.4) only checked `microvm@<vm>.service`. Graphics VMs
  run cloud-hypervisor via the `d2b-<vm>-gpu.service` sidecar
  (the GPU sidecar replaces the upstream runner), so the audit
  was blanket-skipping all graphics VMs even when they were
  running. Now: a VM is "running" if any of `d2b@<vm>`,
  `microvm@<vm>`, or `d2b-<vm>-gpu` is active.

- **`nixos-modules/cli.nix`** (`d2b list` / `d2b status`):
  pending-drift messages used to recommend `d2b switch <vm>`,
  which is the heavier per-VM-closure-rebuild path. The correct
  remediation for unit-file drift after a host `nixos-rebuild
  switch` is `d2b restart <vm>` (clean down+up cycles the
  running closure over the staged unit files). Messages updated;
  status label `[pending switch]` renamed to `[pending restart]`
  to match.

## [0.1.5] - 2026-05-19

Patch release. Three consumer-impacting items from the first
`/etc/nixos`-side migration: the framework's nixos-rebuild
hot-restart of per-VM sidecars was killing running VMs; the
load-host-keys group assumption broke for the standard NixOS user
shape; and once we stopped restarting, consumers had no signal that
config drift had built up.

### Added

- **`d2b restart <vm> [--force]`** - convenience wrapper around
  `down <vm>` + `up <vm>`. Idempotent (a stopped VM is just brought
  up). Graphics VMs still require a Wayland session for the up
  step. The `--force` flag is forwarded to the down step (lets you
  cycle a net VM without first stopping the env's workloads). Used
  in tandem with the new `pending-restart` indicator below: when
  `d2b list` flags a VM, `d2b restart <vm>` applies the
  pending config.

- **`pending-restart` signal in `d2b list` / `d2b status`.**
  Compares each VM's `current` symlink (latest declared closure)
  against `booted` (the closure the running VM actually exec'd).
  If they differ AND the VM is up, both UIs flag the VM:

  ```
  NAME             ENV    GRAPHICS TPM   USBIP   STATIC_IP       STATUS
  work-aad         work   true     true  true    10.20.0.10      systemd [pending restart]
  ```

  And `d2b status work-aad` adds:

  ```
  pending-restart: YES - unit files changed; run `d2b restart work-aad` to apply
    booted : /nix/store/...-microvm-cloud-hypervisor-work-aad
    current: /nix/store/...-microvm-cloud-hypervisor-work-aad
  ```

  Note: v0.1.5 originally shipped the label as `[pending switch]`
  with a `run d2b switch <vm>` recommendation; v0.1.6 renamed
  the label to `[pending restart]` and the message to recommend
  `d2b restart <vm>` (the correct action for unit-file drift
  is the lighter `restart`, not the per-VM-closure-rebuild
  `switch`). Pre-v0.1.6 docs may show the legacy strings.

  Required because of the `restartIfChanged = false` changes below -
  without that signal, consumers had no way to know their
  `nixos-rebuild switch` only landed unit-file changes and not VM
  behaviour.

### Fixed

- **`restartIfChanged = false` on every per-VM lifecycle service.**
  Pre-v0.1.5, every `nixos-rebuild switch` that touched any of the
  per-VM units killed the running VM mid-flight - for graphics
  VMs the GPU sidecar IS the cloud-hypervisor process, so its
  restart terminated CH, the guest's in-RAM Entra device-bound
  tokens evaporated, and the user lost their login session. Even
  for headless VMs, every framework-touched config (host-keys
  refresh wiring, virtiofsd hardening stanza) caused NixOS to
  override upstream microvm.nix's `X-RestartIfChanged=false` back
  to `true`. The new flag updates the unit files at rebuild time
  but does NOT cycle the running VM; consumers apply per-VM
  changes via `d2b restart <vm>` (or `d2b switch <vm>`
  for a per-VM closure rebuild + live activation).

  Services covered:
  - `d2b@<vm>.service` (user-facing wrapper)
  - `microvm@<vm>.service` (upstream runner; framework was
    overriding upstream's existing flag back to true via the
    host-known-hosts.nix drop-in)
  - `microvm-virtiofsd@<vm>.service` (per-VM virtiofs daemon;
    framework adds hardening stanza)
  - `d2b-<vm>-swtpm.service`
  - `d2b-<vm>-snd.service`
  - `d2b-<vm>-gpu.service`

- **`d2b-<vm>-gpu.service` updates the per-VM `booted`
  symlink.** Upstream microvm.nix's
  `microvm-set-booted@<vm>.service` only runs as part of
  `microvm@<vm>.service`'s lifecycle - but graphics VMs bypass
  that template (the GPU sidecar runs microvm-run directly).
  Pre-v0.1.5, `/var/lib/d2b/vms/<vm>/booted` simply didn't
  exist for graphics VMs, so the new pending-restart check
  couldn't compute anything. Added `ExecStartPre`
  (`+`-prefixed → root) that mirrors
  `microvm-set-booted_-start`:
  `rm -f booted && ln -s $(readlink current) booted`. Cleared
  by `ExecStopPost`.

- **`d2b-load-host-keys.service` primary-group resolution.**
  Pre-v0.1.5 the script assumed the guest user's primary group
  matched the username (`install -d ... -g "$SSH_USER"`). This
  only holds when the consumer's VM config sets
  `users.users.<u>.group = "<u>"` or uses DynamicUser. NixOS's
  `isNormalUser = true` default puts the user in the `users`
  group, breaking the install with
  `install: invalid group '<u>'`. Result: no d2b-managed
  pubkey ever reached the guest's `authorized_keys`, and SSH
  only worked for keys baked statically into
  `users.users.<u>.openssh.authorizedKeys.keys`.

  Now: resolve GID via `getent passwd | cut -d: -f4`, then GID →
  name via `getent group`. Works for both
  `users.users.<u>.group = "<u>"` and the NixOS default.

## [0.1.4] - 2026-05-19

Patch release. Four framework bugs surfaced during the first real
consumer migration's VM bring-up (paydro's /etc/nixos, after v0.1.3
got `d2b@<vm>` units working but the actual graphics+TPM VM
refused to boot).

### Fixed

- **`nixos-modules/host-sidecars.nix`**: per-VM GPU sidecar
  (`d2b-<vm>-gpu.service`) had `DevicePolicy = "closed"` without
  `/dev/net/tun` in `DeviceAllow`. Cloud-hypervisor needs to
  `open("/dev/net/tun")` + `ioctl(TUNSETIFF, …)` to attach to the
  VM's tap (created earlier by upstream microvm.nix's
  `microvm-tap-interfaces@<vm>.service` helper); without it
  graphics VMs crash in early boot with "Cannot create virtio-net
  device / Couldn't open /dev/net/tun / Operation not permitted".
  Added `/dev/net/tun rw` to DeviceAllow.

- **`nixos-modules/host-activation.nix`**: `d2bVmStatePerms`
  granted ACL rwx on `/var/lib/d2b/vms/<vm>/` to
  `d2b-<vm>-gpu` but not to `d2b-<vm>-swtpm`. The swtpm
  service starts as the swtpm user, opens its `StateDirectory=`
  (which systemd creates at the correct path), then tries to read
  `tpm2-00.permall` - and EACCESes because traversing the parent
  dir requires +x for the swtpm user. libtpms enters failure mode
  and the VM boots with a freshly-initialised TPM, triggering
  Entra/Intune device-tampering alerts for tenant-enrolled VMs.
  Added `setfacl -m "u:d2b-<vm>-swtpm:--x" <stateDir>` (gated
  on `vm.tpm.enable`).

- **`nixos-modules/base.nix`**: `d2b-load-host-keys.service`
  inside the guest referenced `${"$"}{pkgs.coreutils}/bin/getent` -
  but `getent` is in glibc, not coreutils. The lookup silently
  failed with "No such file or directory" and the script printed
  `user '<u>' not found in /etc/passwd - skipping` even though the
  user existed. Result: d2b-managed pubkeys + the consumer's
  `userAuthorizedKeys` never reached the guest's
  `authorized_keys` - SSH worked only via any pubkey statically
  baked into the VM's `users.users.<u>.openssh.authorizedKeys.keys`.
  Fixed path to `${"$"}{pkgs.glibc.getent}/bin/getent`.

- **`nixos-modules/cli.nix`** (audit `--strict`): the
  `bridge_isolated_workload.<vm>` check ran unconditionally and
  STRICT-FAILed when the VM wasn't running (the workload tap
  doesn't exist on the bridge, so jq returned null). With the
  framework's default `d2b.vms.<vm>.autostart = false`, this
  blocked every post-activation `d2b-audit-check.service`
  hook → `nixos-rebuild switch` returned non-zero exit code 4.
  Added a `systemctl is-active microvm@<vm>` precondition that
  emits `AUDIT SKIP [bridge_isolated_workload.<vm>]: VM not
  running` (mirrors the existing virtiofsd skip-when-down
  semantic).

## [0.1.3] - 2026-05-19

Patch release. Two more framework bugs surfaced during the first
real consumer migration, both around the d2b@<vm> wrapper +
microvm.nix interaction.

### Fixed

- **`nixos-modules/host-wrapper.nix`**: per-VM `d2b@<vm>.service`
  units for `autostart=true` VMs were emitted as separate unit files
  (via `systemd.services."d2b@${name}"`) that NixOS materialised
  WITHOUT the template's `ExecStart`/`ExecStop`/`PropagatesStopTo`/
  `Type=oneshot` settings - so systemd refused them at boot with
  "Service has no ExecStart=, ExecStop=, or SuccessAction=. Refusing."

  Fix: drop the per-instance `systemd.services` declarations and
  use `systemd.targets.multi-user.wants` symlinks instead. systemd
  then resolves each `d2b@<vm>.service` against the template
  with all its lifecycle wiring intact.

- **`nixos-modules/host-wrapper.nix`**: upstream microvm.nix emits
  `systemd.targets.microvms.wants = ["microvm@<vm>.service" …]`
  for every `microvm.vms.<vm>` declaration. `microvms.target` is
  itself `wantedBy = ["multi-user.target"]`, so workload VMs got
  pulled into boot regardless of `microvm.autostart = []`. Setting
  `microvm.autostart` only controls upstream's `multi-user.target.wants`
  on the microvm@ unit, not the `microvms.target` Wants= relation.

  Fix: `lib.mkForce` `systemd.targets.microvms.wants` to enumerate
  ONLY `autostart=true` VMs. Workload VMs are now exclusively
  on-demand via `d2b up <vm>`.

## [0.1.2] - 2026-05-19

Patch release. Surfaced during the first real consumer migration to
v0.1.x - a runtime bootstrap deadlock between
`d2b-net-route-preflight.service` and the per-env uplink bridge.

### Fixed

- **`nixos-modules/network.nix`**: per-env uplink bridge
  (`br-<env>-up`) now has `networkConfig.ConfigureWithoutCarrier =
  true`. Without it, networkd refuses to apply the Address + static
  Route to the env's LAN subnet until the bridge has carrier. But
  carrier only appears when the per-env net VM attaches its uplink
  tap to the bridge, and the net VM start is gated on
  `d2b-net-route-preflight.service`, which checks the static
  route exists. Deadlock.

  The LAN bridge already had `ConfigureWithoutCarrier = true`; the
  uplink-bridge case was missing. The fix is one option per env;
  no consumer config changes required.

  Existing v0.1.0 / v0.1.1 consumers can work around by running
  `sudo ip route add <env-lan>/<mask> via <env-uplink-gw> dev
  br-<env>-up` once per env before any
  `nixos-rebuild switch` - but the proper fix is to upgrade to
  v0.1.2 and re-rebuild.

## [0.1.1] - 2026-05-19

Patch release. Two consumer-impacting items surfaced during the
first real `/etc/nixos`-side migration to v0.1.0.

### Added

- **`d2b.site.extraSpecialArgs`** (`attrsOf unspecified`,
  default `{}`). Merged into every per-VM
  `microvm.vms.<vm>.specialArgs` after the framework's own
  baseline. Consumer keys take precedence on collision, so a
  consumer that wants its full flake `inputs` (rather than just
  d2b's narrower input set) visible inside per-VM modules
  can set:
  ```nix
  d2b.site.extraSpecialArgs = { inherit inputs; };
  ```
  Mirrors `home-manager.extraSpecialArgs` from the Home-Manager
  NixOS module - same semantics, same intent.

### Fixed

- **`scripts/migrate-d2b-v0.1.0.sh`**: `[[ -d "$dir" ]] && info ...`
  under `set -euo pipefail` aborted the script silently when the
  optional private-TPM-state directory didn't exist (return-value
  of the compound `&&` chain propagated up as the function's exit
  status). Replaced with explicit `if [[ -d ]]; then info; fi` for
  set-e safety. The bug aborted the snapshot phase before the
  `tpm2_getcap` step could run, leaving the migration in an
  in-progress state that required a manual cleanup.

## [0.1.0] - 2026-05-19

First public alpha release.

**Audience:** single-user NixOS desktop wanting isolated workspaces
(work / personal / risky-dev) on one host. Wayland-native.

**Stable in v0.1.0:**

- `nixosModules.default` (host integration)
- `templates.default` (`nix flake init -t github:vicondoa/d2b`)
- `flake.checks.<sys>.eval-{minimal,multi-env,template,graphics}`
- `d2b@<vm>.service` lifecycle wrapper + the eight `d2b` CLI
  verbs (`up`, `down`, `status`, `list`, `switch`, `build`, `boot`,
  `test`, `rollback`, `generations`, `gc`, `audio`, `usb`, `console`,
  `keys`)
- `manifestVersion = 1` JSON contract (`/run/current-system/sw/share/d2b/vms.json`)
- VM-name regex `^[a-z][a-z0-9-]*$`, reserved prefixes `sys-` and
  exact name `launcher`
- Per-env isolated network (auto-declared `sys-<env>-net` net VM,
  point-to-point uplink, isolated LAN bridge, dnsmasq, nftables NAT)
- Per-VM `/nix/store` hardlink farm
- D2b-managed SSH keys
- Components: `graphics`, `tpm`, `usbip`, `audio`, `home-manager`

**Composition:** Sibling flake [`vicondoa/entrablau.nix`][entrablau] (also
v0.1.0) provides Entra ID device-join via the per-VM
`d2b.vms.<vm>.config.imports = [ inputs.entrablau.nixosModules.default ]`
seam.

[entrablau]: https://github.com/vicondoa/entrablau.nix

> Maintainer GitHub metadata reminder (apply on the GitHub UI, not in git):
>
> - **Description:** "NixOS microVM framework with isolated per-env
>   networking, Wayland/audio/USBIP/TPM components, and a
>   `nix flake init` template scaffold."
> - **Topics:** `nixos`, `nix-flake`, `microvm`, `wayland`,
>   `microvm-nix`, `nixos-template`, `entra-id`.



### Added

- `flake.checks.<system>.eval-{minimal,multi-env,template,graphics}` -
  the root flake now gates the example flakes + the template
  scaffold. The `graphics` check is x86_64-only.
- `tests/static.sh` now iterates `examples/*/flake.nix` running
  `nix flake check --no-build --all-systems` on each.
- `SECURITY.md` - disclosure path (GitHub Security Advisory primary;
  email fallback) plus the v0.1.0 alpha support matrix.
- `docs/explanation/design.md` - full threat model + defenses-in-depth
  list + a *Why not X* rationale FAQ (~823 LOC).
- `docs/how-to/migrating-from-microvm.md` - option mapping +
  step-by-step migration procedure + troubleshooting. Ordering is
  now build-before-state-move.
- Five per-component reference docs under
  `docs/reference/components-*.md` (graphics, tpm, usbip, audio,
  home-manager).
- `docs/reference/manifest-schema.{md,json}` polished with a rendered
  example payload generated from `tests/unit/smoke/smoke-eval.nix`.
- **`examples/minimal/`** - headless starter example: one env, one
  workload VM, ~25-line flake. Provides a quick sanity test.
- **`examples/graphics-workstation/`** - desktop VM with
  `graphics.enable`, `audio.enable`, and `usbip.yubikey` all on.
  Exercises every host-side sidecar component.
- **`examples/multi-env/`** - two parallel `d2b.envs.<env>`
  instances (work + personal) demonstrating per-env LAN
  isolation, per-env net VMs, per-env USBIP backends, and the
  route-preflight fail-closed gate.
- **`examples/with-entra-id/`** - composition with the sibling
  [`vicondoa/entrablau.nix`][entrablau] flake; shows how
  the two trees meet at `d2b.vms.<vm>.config.imports`
  without either flake depending on the other.
- **`templates/default/`** - `nix flake init` scaffold with
  seven numbered placeholder markers and a matching
  `assertions = [ … ]` block. `nix flake check` on an un-edited
  scaffold fails with actionable messages until each sentinel is
  replaced.
- **`flake.templates.default`** - wires the template above so
  consumers can `nix flake init -t github:vicondoa/d2b`.
- **Manifest contract is now a documented, versioned interface.**
  - `nixos-modules/manifest.nix` - typed `config.d2b.manifest`
    `attrsOf submodule` option. Replaces the inline manifest
    construction previously folded into `cli.nix`. The Nix module
    system catches schema regressions at eval time.
  - `docs/reference/manifest-schema.md` + `docs/reference/manifest-schema.json`
    (JSON Schema Draft 2020-12) - the v1 public manifest contract
    for downstream consumers such as the Rust CLI. The
    JSON Schema is the canonical type spec; the prose doc is a
    field-by-field walkthrough + compatibility policy.
  - `docs/reference/cli-contract.md` - behavioural contract for any
    `d2b` CLI implementation (lifecycle FSM, signal semantics,
    exit codes, JSON vs human output, what is/is-not in scope).
  - `d2b.site.flakePath` is now derived as the CLI's default
    flake reference when unset (cli.nix lifecycle subcommands).
- **`docs/README.md`** - Diataxis IA index (tutorials, how-to,
  reference, explanation). The reference quadrant landed first;
  the others landed before v0.1.0.
- **Multi-arch eval coverage.** `tests/unit/smoke/smoke-eval-aarch64.nix` -
  cross-evaluates a headless workload VM on `aarch64-linux`,
  verifying the eval graph stays multi-arch clean. Runtime is still
  `x86_64-linux`-only (cloud-hypervisor + crosvm); aarch64 is
  eval-coverage only.
- **Manifest validation gate.** `tests/static.sh` now renders the
  smoke manifest and runs a 6-check sequence against
  `docs/reference/manifest-schema.json`: render → parse schema →
  JSON-Schema validate → schema-side field cross-check →
  `manifestVersion >= 1` → prose-schema table diff against the JSON
  Schema's `properties` keys to catch md ↔ json drift.
- **`d2b.site.*` public option surface.** Site-specific knobs
  extracted from previously-hardcoded references to the
  maintainer's host setup. Every option is opt-in; defaults give a
  fully headless framework with no Wayland integration. Public
  options:
  - `d2b.site.stateDir` - root of every d2b-managed state
    file (default `/var/lib/d2b`). **Advisory only in v0.1.0**
    (see option description); full threading lands in v0.2.0.
  - `d2b.site.keysDir` - directory for framework-managed
    per-VM SSH keys (default `${stateDir}/keys`). Same advisory
    caveat for v0.1.0.
  - `d2b.site.waylandUser` - primary Wayland user; required
    for any VM with `graphics.enable = true` or `audio.enable =
    true`.
  - `d2b.site.launcherUsers` - users added to the
    `d2b-launcher` group (polkit grant for VM start/stop).
  - `d2b.site.userAuthorizedKeys` - global authorized SSH
    keys merged into every VM at boot. Validated at eval time
    against an allowlist of supported key types; private-key
    markers rejected.
  - `d2b.site.yubikey.enable` - host-side Yubico udev rules +
    `usbip-host` kernel module. Default true.
  - `d2b.site.flakePath` - default flake reference for the
    `d2b` CLI's lifecycle subcommands (`build`, `switch`,
    `boot`, `test`). Nullable.
- **`d2b.vms.<vm>.userAuthorizedKeys`** - per-VM
  authorized SSH keys, merged with `site.userAuthorizedKeys`.
- **`d2b.audio.users`** - host-side option propagating an
  audio-group membership list into the guest. Default falls back
  to `[ vm.ssh.user ]` when unset.
- **Framework-managed per-VM SSH keys.** Activation
  (`nixos-modules/host-keys.nix`) generates an Ed25519 keypair
  per enabled VM under `<keysDir>/<vm>_ed25519`. Atomic via
  staging + `mv -T`; protected by `flock` on `<keysDir>/.lock`.
  The pubkey is staged under
  `<stateDir>/vms/<vm>/host-keys/host.pub` and injected into the
  guest at boot via virtiofs.
- **`d2b keys` CLI subcommands.**
  - `d2b keys list [--json]` - fingerprint + path + mtime
    per VM.
  - `d2b keys show <vm>` - print the pubkey.
  - `d2b keys rotate <vm>` - atomic rotate-and-verify with
    SHA256-fingerprint-based old-key scrub + 3-generation
    retention (see Changed entry above).
- **`d2b-load-host-keys.service`** (guest-side) - fail-closed
  service that reads `/run/d2b-host-keys/` and writes the
  union of `host.pub` + user-authorized-keys into the SSH user's
  `~/.ssh/authorized_keys`.
- **`scripts/migrate-d2b-v0.1.0.sh`** - one-shot host migration
  script for consumers upgrading from a pre-public in-tree d2b
  layout. Preserves TPM state byte-for-byte. Has `--dry-run` and
  `--rollback`. Committed under `scripts/` so CI can shellcheck it.
- **`tests/unit/smoke/smoke-eval.nix`** - minimal consumer-style nixosSystem
  that imports `d2b.nixosModules.default` and exercises the
  eval graph end-to-end. Wired into `tests/static.sh` Layer-1.
- **`tests/assertions-eval.sh`** - 8 regression tests exercising every
  eval-time invariant in the schema (CIDR shape, CIDR overlap, key
  validation, `waylandUser` presence, …).
- **`nixos-modules/lib.nix#cidrOverlaps`** - pure-Nix IPv4 prefix
  overlap helper used by network.nix assertions. Same file gains
  `parseCidr` as a public helper.
- Initial flake skeleton with Apache-2.0 license, `x86_64-linux` +
  `aarch64-linux` eval, `microvm.nix` input, and reserved-but-inert
  `nixosModules.default`.
- Mechanical lift of d2b modules from `/etc/nixos/modules/d2b/`
  into the public flake:
  - 9 core modules under `nixos-modules/` (`default`, `options`,
    `lib`, `host`, `network`, `base`, `store`, `cli`;
    `router.nix` renamed to `net.nix`);
  - 6 component modules under `nixos-modules/components/`
    (`graphics`, `tpm`, `usbip`, `home-manager`; `audio` split into
    `audio/{guest,host}.nix`);
  - Extracted pkgs: `spectrum-ch`, `vhost-device-sound`,
    `crosvm-patched`, `crosvm-seccomp`, `patches`;
  - 6 generic test scripts under `tests/`.
- `systemd.services."d2b@"` wrapper template with explicit
  `ExecStart` / `ExecStop` / `PropagatesStopTo`; `BindsTo` alone
  does not propagate stops.
- Eval-time assertions for VM names (`^[a-z0-9][a-z0-9-]*$`, no
  `sys-` prefix, not `launcher`) and env names (≤ 8 chars).
- `nixos-modules/assertions.nix` as a dedicated assertions module.
- Top-level `manifestVersion = 0` stub field in the per-VM JSON
  manifest. It was added as a stub; a later release bumps it. Stashed
  under the reserved `_manifest` sentinel key; user-declared VM names
  cannot start with `_` under the stricter regex.

### Changed

- `docs/README.md` IA now reflects the shipping how-to and
  explanation docs (was previously reference-only).
- **README:** restructured to lead with a Where-to-start table
  pointing at the four examples and the template, and rewrote
  the Quick start to walk through the template path; the prior
  manual paste-in walkthrough is preserved under Manual integration
  without the template.
- **`docs/README.md`:** added a Tutorials/Examples section linking the
  examples and the template; previously the docs index only mentioned
  the reference quadrant.
- **BREAKING for manifest consumers (pre-v0.1.0):** `manifestVersion`
  bumped `0 → 1`. The schema is now the documented contract. Future
  schema changes follow SemVer: minor field additions are
  backward-compatible; breaking changes bump the major (`2`, `3`,
  …). Consumers MUST refuse manifests with a newer major version
  than they were built against.
- **`d2b.vms.<vm>.graphics.enable` and
  `d2b.vms.<vm>.audio.enable` now refuse to evaluate on
  `aarch64-linux`** at the `microvm.vms` translation point. The
  eval-time error explains the constraint. Headless workload VMs
  (`graphics.enable = false; audio.enable = false;`) DO evaluate on
  aarch64-linux for cross-eval testing. Actual runtime is still
  x86_64-linux-only - the aarch64 path is eval-coverage only.
- `pkgs/{crosvm-patched,crosvm-seccomp,vhost-device-sound}/default.nix`
  now carry `meta.platforms = [ "x86_64-linux" ]`.
  `pkgs/spectrum-ch/default.nix` deliberately omits this (see
  in-file comment).
- `nixos-modules/options.nix` (internal refactor, no consumer-
  visible change): split into `options.nix` (aggregator) +
  `options-site.nix` + `options-envs.nix` + `options-vms.nix` for
  reviewability. The smoke-eval drvPath is bit-identical pre/post
  the split.
- **BREAKING for manifest consumers, security fix:** `sshKeyPath`
  removed from the per-VM JSON manifest. Security review flagged
  the field as a private-key path leak - the manifest at
  `/run/current-system/sw/share/d2b/vms.json` is world-readable,
  so exposing a per-VM private-key path leaks the location of
  secret material to every local user. The CLI now resolves the
  private-key path locally at Nix-eval time from
  `d2b.site.keysDir` (or per-VM `ssh.keyPath` override) and
  bakes a static per-VM mapping into the shell wrapper. Consumers
  reimplementing the CLI should mirror that: read
  `d2b.site.keysDir` from their own privileged config access,
  not from this world-readable file. The PUBLIC key path is not
  currently exposed; if a use case warrants it, a future
  `sshPubKeyPath` field is the recommended addition. `manifestVersion`
  stays at `1` - the schema was published moments before release and
  no external consumers exist yet, so this is a free pre-v0.1.0 break.
- `docs/reference/manifest-schema.json`: `manifestVersion.minimum`
  raised from `0` to `1`. The schema is the contract for v1+;
  pre-v1 manifest stubs are no longer valid under this schema.
- `docs/reference/cli-contract.md`: subcommand inventory reconciled
  with `d2b --help`. `audit` now correctly documents the
  `--strict` + `--human` flags (`--human` auto-enables on TTY);
  `rotate-known-host <vm>` (the companion to `trust`) added to the
  subcommand table and to the human/JSON output section.
- `docs/reference/cli-contract.md`: the What-is-not-in-this-contract
  section expanded. Spells out that microvm.nix internal lifecycle,
  swtpm internals, virtiofsd implementation, and polkit grant
  specifics are framework-internal; and draws the line between
  contract-bound unit names (`d2b@<vm>.service`,
  `microvm@<vm>.service`) and framework-internal unit names
  (sidecars, USBIP proxies - these MUST be read from the manifest's
  `audioService` etc. fields, not hardcoded).
- `tests/static.sh`: `nix flake check` now uses `--all-systems` so
  Layer-1 exercises both x86_64-linux and aarch64-linux flake
  outputs, not just the builder's system.
- `tests/static.sh`: 6th manifest-contract check added - diffs the
  field-name column of the prose Per-VM-entry table in
  `docs/reference/manifest-schema.md` against the JSON Schema's
  `$defs.vmEntry.properties` keys, failing the gate if either side
  has a field the other doesn't.
- README: project status now states runtime is tested on
  `x86_64-linux` desktop and eval-tested for headless
  `aarch64-linux`, reflecting cross-eval coverage.
- README: documentation section replaces a placeholder docs directory
  note with direct bullets pointing at the manifest schema and CLI
  contract under `docs/reference/`.
- `tests/README.md`: refreshed for `manifestVersion = 1`, 10/10
  assertions-eval cases, the 6-step manifest-contract gate (including
  the new md/json drift detection), and the multi-arch eval coverage.
- Diataxis reorg. `docs/manifest-schema.{md,json}` →
  `docs/reference/manifest-schema.{md,json}`; `docs/cli-contract.md`
  → `docs/reference/cli-contract.md`. Added `docs/README.md` as the
  IA index. All path references in `nixos-modules/manifest.nix`,
  `tests/static.sh`, and the moved docs' cross-links updated.
- **`d2b.vms.<vm>.ssh.keyPath` is NOT removed.** Earlier commit
  messages claimed otherwise; that was a mis-description of the
  change. The option still exists. What changed is its effective
  default: when left unset (`null`), the CLI now derives the SSH-key
  path from `d2b.site.keysDir` as `<keysDir>/<vm>_ed25519`,
  matching the framework-managed Ed25519 key generated by
  `host-keys.nix` on every activation. Consumers who explicitly set
  a path still win; the option's `null` default lets the framework
  pick. This makes the framework-managed key the zero-config happy
  path while keeping the option-shape stable for consumers supplying
  their own keys (e.g. a hardware-backed Yubikey-resident key).
- Net VM `users.allowNoPasswordLogin` is set to `lib.mkDefault true`.
  Net VMs receive SSH keys via runtime injection
  (`d2b-load-host-keys.service` reads
  `<stateDir>/vms/<vm>/host-keys/` over virtiofs); they have no
  eval-time authorized_keys. Without the flag, NixOS module-eval
  fires the `users.allowNoPasswordLogin` assertion before runtime
  injection runs. Sealed-appliance consumers can override with
  `mkForce`.
- GPU sidecar (`d2b-<vm>-gpu.service`) hardening tightened:
  `NoNewPrivileges`, `ProtectSystem=strict`, `PrivateTmp`,
  `ProtectHome`, `DevicePolicy=closed` with a `/dev/kvm` +
  render-node allowlist, `RestrictAddressFamilies =
  [ AF_UNIX AF_NETLINK AF_VSOCK ]`, `SystemCallArchitectures=native`,
  narrow `ReadWritePaths`. Two omissions documented in source
  comments: `MemoryDenyWriteExecute` (crosvm GPU JIT triggers SIGSYS)
  and `AF_VSOCK` retained (cloud-hypervisor sd_notify path).
- IPv6 disabled on workload + net VM guest networkd
  (`LinkLocalAddressing=no`, `IPv6AcceptRA=false`); net VM nft rules
  DROP `ip6` forward. Net stack is IPv4-only by construction.
- Route preflight oneshot (`d2b-net-route-preflight.service`) now
  FAILS CLOSED on conflict - exit 1 on any env-vs-route mismatch
  instead of WARN+exit 0. `RemainAfterExit=true`, `Before=` each
  enabled d2b-managed VM unit, `RequiredBy=` each wrapper, so a
  stale host route blocks VM start until the operator clears it.
- **BREAKING.** Option namespace renamed:
  - `d2b.networks.<env>` → `d2b.envs.<env>`;
  - `d2b.networks.<env>.routerName` →
    `d2b.envs.<env>.netName`;
  - `d2b.networks.<env>.extraRouterConfig` →
    `d2b.envs.<env>.extraNetConfig`.
- **BREAKING.** Per-env auto-declared VM renamed:
  `<env>-router` → `sys-<env>-net`.
- **BREAKING.** Systemd unit naming convention:
  - `swtpm@<vm>` → `d2b-<vm>-swtpm`;
  - `d2b-snd@<vm>` → `d2b-<vm>-snd`;
  - `d2b-gpu-<vm>` → `d2b-<vm>-gpu`;
  - `d2b-store-sync@<vm>` → `d2b-<vm>-store-sync`;
  - `usbipd-d2b` → `d2b-sys-usbipd`;
  - `usbipd-d2b-<env>` → `d2b-sys-<env>-usbipd-proxy`.
- **BREAKING.** System users/groups renamed: `d2b-gpu-<vm>` →
  `d2b-<vm>-gpu`, `d2b-snd-<vm>` → `d2b-<vm>-snd`,
  `swtpm-<vm>` → `d2b-<vm>-swtpm`.
- **BREAKING.** State-dir layout:
  - `<stateDir>/<vm>/` → `<stateDir>/vms/<vm>/`;
  - `<stateDir>/<env>-router/` → `<stateDir>/vms/sys-<env>-net/`;
  - `<stateDir>/swtpm/<vm>/` → `<stateDir>/vms/<vm>/swtpm/`;
  - `/run/d2b-snd/<vm>/snd.sock` →
    `/run/d2b/vms/<vm>/snd.sock`.
- **BREAKING.** Manifest JSON contract: `isRouter` → `isNetVm`,
  `routerVm` → `netVm`. Top-level `manifestVersion = 0` was added as
  a stub; a later release bumps it.
- **BREAKING.** VM/env name regex tightened from
  `^[a-z0-9][a-z0-9-]*$` to `^[a-z][a-z0-9-]*$` (require leading
  letter). Matches systemd-escape best practices; avoids ambiguity
  with tooling that treats a leading digit as a numeric index
  (`ip link show 42web-l10`). No existing in-tree names trip the
  stricter rule; consumers with numeric-prefixed VM/env names must
  rename.
- CLI: `d2b up/down/status` now target `d2b@<vm>.service`
  (the user-facing wrapper) instead of `microvm@<vm>.service`
  directly. Lifecycle propagates via the wrapper's BindsTo /
  ExecStop. Diagnostic flows (`status --verbose`, `journalctl`
  examples) keep their `microvm@<vm>` references but label them
  `backend` / `implementation detail`.
- CLI: `d2b list` / `d2b status` output tag for system VMs
  changed from `(router)` to `(net-vm)`. Helper renames:
  `ensure_router_up` → `ensure_net_vm_up`, `router_active` →
  `net_vm_active`, `IS_ROUTER` → `IS_NET_VM`. User-facing prose
  `router` / `router VM` → `net` / `net VM` (kept `routing/routes`
  only where describing the network function).
- `d2b-launcher` polkit grant tightened to an exact-unit allowlist
  generated at NixOS eval time from `cfg.vms` + `cfg.envs`, restricted
  to `start` / `stop` / `restart` verbs only. Drops the bare
  `microvm@*` prefix wildcard; default-deny invariant restored.
  Recovery / debugging paths can still authenticate via sudo or
  polkit-prompt.
- Pre-v0.1.0 breaking changes do not get a deprecation period. There
  is no compat shim for the old `d2b.networks` namespace or for
  any of the renamed unit / user / state-dir identifiers.
- The first tagged release is `v1.0.0`; the v0.x line never tagged a
  public release. These v0.x entries were the in-flight roadmap during
  the development branch and are preserved as historical record of how
  the architecture got to v1.0.
- v1.0.0 ships in lockstep with
  [`vicondoa/entrablau.nix`][entrablau] v1.0.0; consumers
  using both should pin matching tags.

### Fixed

- `tests/{static,d2b-store,audio,lib}.sh` no longer assume
  `ROOT=/etc/nixos`; the value is derived from the script's own path
  so the suite runs from any clone.
- `tests/integration/live/d2b-store.sh:33` SC2157 (preexisting).
- Host-specific `D2B_FILES` entries (`vms/personal-dev.nix`,
  `vms/work-aad.nix`) dropped or guarded so the static gate stays
  useful for the public flake.
- `tests/integration/live/audio.sh` `D2B_WAYLAND_USER` resolution chain genericized
  (no longer hardcoded to the maintainer's host user).
- README polish: `microVM` is defined inline on first use; a
  maintainer-anecdote phrasing was replaced with neutral wording;
  an encrypted-backup callout was added for `/var/lib/d2b/`.
- Manifest schema `manifestVersion` tightened from `minimum: 1` to
  `const: 1` so the JSON Schema matches the documented prose.
- **`nixos-modules/net.nix`:** neutralize base.nix's catch-all
  `10-eth-dhcp` systemd-networkd network on per-env net VMs. The
  catch-all (`matchConfig.Type = "ether"`) sorted lex-first against
  the per-MAC `10-uplink`/`10-lan` definitions and DHCP'd both NICs,
  preempting the static config. Now overridden via `lib.mkForce` with
  a sentinel MAC that never matches. Workload VMs are unaffected -
  they still inherit the base.nix DHCP fallback.
- **`nixos-modules/manifest.nix`:** dropped the redundant
  `default = { }` on the readOnly `d2b.manifest` option. The
  nixpkgs module system treats `default` as an extra definition;
  combined with `readOnly = true` and the matching
  `config.d2b.manifest = …` assignment, it produced
  `set multiple times` only when a graphics VM was synthesized. See
  `tests/unit/smoke/smoke-eval-graphics.nix` for the regression test.
- Inter-env CIDR overlap check now performs real IPv4 prefix
  arithmetic (`lib.cidrOverlaps` in `nixos-modules/lib.nix`) instead
  of exact-string equality. Containment (e.g. `10.0.0.0/16` ⊃
  `10.0.1.0/24`) is rejected. Env-vs-`hostLanCidrs` is checked under
  the same helper.
- `d2b.site.yubikey.enable = false` actually gates the host-side
  udev rules + `usbip-host` kernel module. Previous code declared the
  option but never read it.
- `d2b keys rotate <vm>` now scrubs the OLD pubkey from the
  guest's `~/.ssh/authorized_keys` (matched by SHA256 fingerprint)
  AFTER the new key is verified - rotation used to leave the old key
  authorized forever. Retention bounded: 3 most recent generations
  under `<keysDir>/old/<ts>/`; older are pruned post-rotation. Help
  text updated.

### Removed

- **`d2b.vms.<vm>.entra-id.*` option removed.** Himmelblau /
  Microsoft Entra ID support has moved out of the d2b framework
  and into the sibling `vicondoa/entrablau.nix` flake. To migrate,
  add the flake as an input and import its module into the VM's guest
  config:

  ```nix
  inputs.entrablau.url = "github:vicondoa/entrablau.nix";

  d2b.vms.<vm>.config.imports = [
    inputs.entrablau.nixosModules.default
  ];

  # Move each `d2b.vms.<vm>.entra-id.<key>` into the guest
  # config; see the entrablau README for the new schema.
  ```

  The `d2b.vms.<vm>.entra-id` attribute is kept as a hidden
  stub option so leftover assignments produce a readable assertion
  error (with migration instructions) instead of a cryptic
  `option does not exist` message from the module system. Final
  removal of the stub is tracked for v0.2.0.

- Three host-side activation scripts removed from
  `nixos-modules/host-activation.nix`:
  - **`d2bSbctlBackup`** - moved maintainer-specific
    `*-backup.tar.gz` files from `$HOME` into `/var/lib/sbctl/backup/`.
    Not a framework concern. Consumers who relied on this should
    handle their own backup-file relocation outside d2b.
  - **`d2bStoreChownRepair`** - one-shot repair for a past chown
    bug (an earlier `modules/d2b/store.nix` revision leaked
    `group=kvm` into `/nix/store` inodes via the per-VM hardlink
    farm). New installs are unaffected. Consumers upgrading from a
    pre-public d2b that ran with the buggy revision should run the
    historical repair script from `/etc/nixos` once and then drop the
    activation script there; the bug cannot recur in public code.
  - **`d2bMigrateState`** - one-shot renamer
    (`/var/lib/microvms/<vm>` → `/var/lib/d2b/vms/<vm>`, plus
    `/var/lib/swtpm/<vm>` → `vms/<vm>/swtpm/`). New installs land
    directly on the current layout. Pre-public consumers should use
    the migration script (or perform the moves manually following the
    same logic) before switching to the public flake.

  These deletions remove all host-specific bias from the public
  framework's activation logic. The remaining two activation scripts
  (`d2bVmStatePerms`, `d2bNetVmVarImgPerms`, formerly
  `d2bRouterVarImgPerms`) only adjust file ownership on per-VM
  disk images and contain no host-specific assumptions.

### Known gaps

- **USBIP per-env units materialise even when no VM opts in.** Each
  `d2b.envs.<env>` declares `d2b-sys-<env>-usbipd-backend.service`
  and the corresponding proxy socket regardless of whether any
  workload VM in the env has `usbip.yubikey = true`. The units are
  idle when nothing opts in, but they are still installed. The
  unconditional materialisation is the gap. Tracked for v0.2.0; the
  relevant conditional belongs around `nixos-modules/network.nix:484-650`.
- **No static lint for `mkOption { default = …; readOnly = true; }`
  + matching `config.<…>` assignment.** The issue was caught by
  review, not by tooling. A follow-up will add a grep-level lint to
  prevent the `default + readOnly + config-assignment` trio from
  re-appearing. Trio detection is necessary because `store.nix`
  legitimately carries `readOnly + default` on options that have NO
  matching `config.<…>` assignment, so a two-of-three match is fine;
  only the full three is a bug.
- **Per-example flake-check loop is not fully hermetic for
  `examples/with-entra-id`.** `tests/static.sh` iterates
  `examples/*/flake.nix` and runs `nix flake check --no-build
  --all-systems` per example, but `with-entra-id` depends on the
  sibling `vicondoa/entrablau.nix` flake which the core flake
  cannot pull in as an input. The example's own flake.lock pins
  the sibling and the iteration step exercises eval through it,
  but a clean-tree CI run cannot fully isolate the eval graph
  from the sibling. Tracked for v0.2.0.
- **VM-to-VM east-west traffic within the same env is not
  supported.** Workload taps on the per-env LAN bridge are
  configured with `Isolated = true`, so two workload VMs sharing
  `d2b.envs.<env>` can each reach the net VM (and via NAT,
  the upstream LAN) but cannot directly reach each other.
  Documented in `docs/explanation/design.md` and the
  `d2b.hostLanCidrs` option text. A future opt-in
  (e.g. `d2b.envs.<env>.intraLanIsolation = false`) is on the
  v0.2.0 wishlist.

[entrablau]: https://github.com/vicondoa/entrablau.nix
