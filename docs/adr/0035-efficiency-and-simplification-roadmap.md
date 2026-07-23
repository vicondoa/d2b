# ADR 0035: Efficiency and simplification roadmap

- Status: Accepted
- Date: 2026-06-19
- Related: ADR 0006 (manifest bundle versioning), ADR 0009 (Rust
  toolchain, MSRV, and supply-chain policy), ADR 0015 (daemon-only clean
  break), ADR 0022 (stabilization-mode releases), ADR 0032 (d2b v2
  constellation control plane), ADR 0034 (storage lifecycle, restart
  adoption, and synchronization), ADR 0036 (qemu-media runtime), ADR 0037
  (local hypervisor runtime seam), ADR 0038 (persistent named guest shell
  sessions), ADR 0039 (constellation persistent shell routing)

## Context

D2b has intentionally paid complexity to remove legacy surfaces and
to make host mutation explicit: daemon-only lifecycle, typed broker ops,
generated bundle artifacts, pidfd handoff, minijail profiles, guest-control
RPCs, and v2 constellation/provider seams. Those decisions are still correct.

Since this roadmap was accepted, three major components have made the
cleanup target wider and sharper:

- `qemu-media` is now a documented local runtime with QMP fd-passing,
  broker-owned media mutation, a manual-only host-window/physical-media
  posture, and deliberately unsupported NixOS workload capabilities.
- The local hypervisor runtime seam makes QEMU, Cloud Hypervisor/crosvm, and
  per-VM sidecars implementations of one runtime/service abstraction with
  shared capability, lifecycle, status, denial, reap, and adapter boundaries.
- Persistent named guest shell sessions add a default-off top-level operator
  UX, a shared terminal streaming substrate with interactive exec, a static
  guest helper around shpool, and an explicitly excluded guest helper
  workspace that needs separate supply-chain and static-link gate wiring.
- Constellation persistent shell routing extends that local shell surface to
  target-address semantics, provider guestd-compatible agents, and
  relay-safe `Shell*` operations without adding provider-specific shell
  channels.

The current codebase now has a different risk: every security hardening,
feature wave, generated contract, and test migration has left scaffolding
behind. The result is high cognitive load and slower iteration:

- large Rust hub files concentrate unrelated concerns (`packages/d2bd`,
  `packages/d2b`, and `packages/d2b-priv-broker`);
- some CLI/daemon JSON view models still live beside presentation or
  dispatch logic instead of a shared contract boundary;
- Nix modules repeatedly construct the same bundle-artifact shapes and
  recompute similar VM/env indexes;
- activation, tmpfiles, broker repair, and generated storage contracts
  overlap in ways that are hard to reason about;
- test drivers preserve fail-closed coverage but still carry transitional
  shell wrappers, repeated linting, repeated generated-artifact checks, and
  multiple entrypoint vocabularies;
- v2 constellation work is already adding provider crates and transport
  abstractions, so any cleanup that ignores ADR 0032 would become a forked
  architecture instead of simplification.

Efficiency work must therefore be architectural, not cosmetic. The goal is
to make the fastest path the normal path: fewer code paths, fewer generated
artifact patterns, fewer subprocess gates, fewer roots of authority, and
fewer places where an operator or contributor must remember historical
context.

This stage does **not** preserve backward compatibility for retired
framework surfaces. Consumers that still need an old behavior can stay pinned
to an older d2b revision. New efficiency work should delete obsolete
compatibility layers outright. If current code still calls a compatibility
wrapper, the cleanup wave updates those callers and removes the wrapper in
the same patch series.

That does not mean d2b loses the ability to handle future incompatible
changes. Versioned contracts, migration commands, schema evolution, operator
cutover tooling, and explicit release notes remain first-class mechanisms.
The distinction is simple: delete stale compatibility logic for surfaces that
are already retired; keep the architecture capable of introducing a new,
intentional migration path when a future breaking change actually needs one.

## Decision

D2b will run an efficiency and simplification program as a set of
reviewed waves. Each wave removes one class of duplication or transitional
surface while preserving the load-bearing contracts from earlier ADRs:

- `d2bd` remains the sole lifecycle supervisor.
- `d2b-priv-broker` remains the sole privileged host-mutation authority.
- The Rust CLI remains the only operator CLI surface.
- Generated bundle artifacts remain versioned contracts, not ad hoc JSON.
- Storage, lock, ACL, cleanup, and restart behavior follow ADR 0034.
- QEMU media, Cloud Hypervisor/crosvm workloads, and their sidecars follow
  ADR 0037's runtime/service seam. Runtime kind, provider, capability,
  unsupported-operation, stopped-state, and service-health contracts are
  shared; QMP, QEMU argv, Cloud Hypervisor API, crosvm argv, USBIP, and
  sidecar details stay behind adapters.
- ADR 0036's qemu-media runtime is a current local hypervisor implementation,
  not a second lifecycle architecture. Cleanup may remove the public enroll UX
  as ADR 0037 requires, but it must preserve broker-owned media fd opening,
  paused-before-boot QMP sequencing, manual-only/autostart denial reasons, and
  redacted output/audit behavior.
- ADR 0038's persistent shell UX reuses the exec terminal substrate and keeps
  shpool private behind a guest helper. Cleanup consolidates terminal
  streaming, admission, resize, cursor, detach/force/kill, and redaction
  semantics rather than creating parallel exec and shell protocols.
- Task, thread, and I/O ownership become explicit. Request/task threads do
  not perform unbounded synchronous I/O; blocking filesystem, process,
  network, or broker work runs behind bounded blocking pools, workers, or
  actor-owned queues with cancellation and backpressure.
- v2 provider/transport work follows ADR 0032; this ADR narrows and cleans
  that path rather than introducing another abstraction hierarchy.
- Tests remain fail-closed and follow the Layer-1-first test model; this ADR
  does not authorize new ad hoc shell gates.
- Excluded Rust workspaces, including the broker and the persistent-shell
  guest helper when introduced, are intentional only when they enforce a real
  unsafe-code, dependency, static-link, or supply-chain boundary. Each excluded
  workspace must be wired explicitly into fmt, clippy, test, coverage where
  applicable, deny, audit, static-ELF, and dependency-policy validation as
  applicable.
- Backward compatibility with retired CLI, systemd, option, test, or module
  surfaces is not a constraint for these cleanup waves. The default action is
  removal, not shimming.
- Future incompatible changes still use explicit versioned contracts and
  migration/cutover tooling. This ADR deletes stale shims; it does not remove
  the framework's ability to ship deliberate migrations later.

The waves below are ordered to reduce future work first. Wave 0 creates
measurement and starts compatibility deletion; Waves 1-4 remove duplicated
infrastructure; Waves 5-8 reshape Rust/Nix boundaries and the explicitly
separate helper workspaces; Wave 9 aligns v2 providers with the local
hypervisor seam; Waves 10-11 fix task/runtime hot paths including QMP,
terminal streaming, and shell helper supervision; Wave 12 trims docs and
examples; Wave 13 removes avoidable unsafe code, including helper-prelude and
fd-passing edges; Wave 14 is the recurring ratchet that keeps the codebase
from growing back.

## Efficiency principles

### One contract, many consumers

If the CLI, daemon, broker, docs, and tests all need the same shape, that
shape belongs in a contract crate or generated artifact. Presentation code
may adapt it, but it must not redefine a parallel schema.

This includes provider-neutral local runtime status and capabilities from ADR
0037 and terminal-v1/session contracts shared by interactive exec and
persistent shell from ADR 0038. QMP object ids, shpool protocol details, and
backend argv internals are adapter-private implementation data unless a later
versioned contract explicitly makes a redacted field public.

### One normalized model per evaluation

Nix module evaluation should normalize `cfg.vms`, `cfg.envs`, bundle
artifacts, runner roles, and provider capability records once, then pass
indexes to consumers. Repeated `filterAttrs` / `mapAttrsToList` scans are
acceptable only in leaf modules whose input is already narrowed.

New runtime-facing option shapes stay declarative and feed that same
normalization pass. qemu-media inputs describe runtime kind, media refs,
display posture, and manual-only policy as VM/runtime configuration; persistent
shell inputs stay under the guest shell option family (`guest.shell.*`) and
describe enablement, default name, and session limits. Neither surface may grow
a parallel evaluator, an imperative host hook, or a second capability tree.

### Concern-first naming

Names should tell a reviewer where code runs, what authority it has, and
which layer owns the contract. Rust crate names stay kebab-case, Rust module
and file names stay snake_case, and Rust types/traits stay UpperCamelCase.
The domain prefix comes first so files and crates sort by concern.

Canonical concern prefixes:

| Prefix | Meaning |
| --- | --- |
| `core` | Pure shared model, validation, IDs, and DTOs with no runtime side effects. |
| `ipc` | Local daemon, broker, and guest wire contracts. |
| `daemon` | Unprivileged `d2bd` request handling, task ownership, and supervision. Public binary crate may stay `d2bd`; internal libraries/modules use `daemon`. |
| `broker` | Privileged broker dispatch, audited ops, fd passing, and FFI quarantine. Public binary crate may stay `d2b-priv-broker`; internal libraries/modules use `broker`. |
| `host` | Local or remote full-host substrate work: host OS state, NixOS/generic Linux integration, host networking, and broker-prepared host resources. |
| `guest` | Code running inside workload or gateway guests, including guestd, user/session helpers, and in-guest exec runners. |
| `gateway` | Per-realm gateway guest control-plane services and realm entrypoint glue. |
| `provider` | External provider adapters such as ACA or Relay. Provider crates use `d2b-provider-<name>`. |
| `constellation` | Transport-neutral cross-node/realm protocol, routing, streams, IDs, and capability negotiation from ADR 0032. |
| `contract` | Schema, generated-artifact, policy, and fixture contract tests. |
| `cli` | Operator presentation and command dispatch. Public binary crate may stay `d2b`; internal modules use `cli` or command-family names. |

Avoid catch-all names such as `remote`, `manager`, `helper`, `util`, and
`common` unless the item is genuinely generic and has no clearer authority
or lifecycle owner. "Remote" is relative to the caller; code should instead
name the actual concern (`constellation`, `provider`, `gateway`, or `host`).
ADR 0032 target names remain DNS-shaped (`<workload>.<node>.<realm>.d2b`
or `d2b://...`) and do not encode whether a realm is host-resident or
gateway-backed; crate/module/type names follow the implementation concern,
not the address string.

For v2 provider axes, name the axis explicitly:

- local hypervisor providers sort under `d2b-provider-hypervisor-<name>`
  or an equivalent `provider::hypervisor::<name>` module family;
- host substrate providers sort under `d2b-host-substrate-<name>` or
  `host::substrate::<name>`;
- display/Wayland providers sort under `d2b-provider-display-<name>` or
  `provider::display::<name>`;
- transport providers sort under `d2b-constellation-transport-<name>` or
  `constellation::transport::<name>`.

Crate names should follow `d2b-<concern>-<specific-capability>` when the
crate is not a public binary. Examples:

- `d2b-constellation-core`, `d2b-constellation-router`,
  `d2b-constellation-transport`;
- `d2b-provider-aca`, `d2b-provider-relay`;
- `d2b-host-activation-helper`, `d2b-host-substrate-nixos` or
  `d2b-host-substrate-linux` for future host-substrate adapters;
- `d2b-guest-exec-runner` rather than a locus-free exec crate if the
  code runs in the guest.

Rust type names should use standard Rust acronym casing (`VmId`, `IpcFrame`,
`JsonSchema`, `HttpClient`) and carry the concern only when the unqualified
name is ambiguous. Use consistent suffixes:

- `Id`, `Name`, `Ref` for validated identifiers and references;
- `Request`, `Response`, `Event`, `Frame` for wire shapes;
- `Spec`, `Intent`, `Plan` for generated desired state;
- `State`, `Snapshot`, `Report` for observed or persisted state;
- `Policy`, `Capability`, `Permission` for authorization/feature gates;
- `Actor`, `Worker`, `Task`, `Handle` for concurrency-owned runtime pieces;
- `Error` for typed error enums.

File and module paths should mirror the concern tree before the operation
name. For example, daemon guest-control code belongs under
`daemon/guest_control/*` or a `guest_control/` module family, broker ops under
`broker/ops/<op>.rs`, provider adapters under `provider/<name>/*`, and
constellation protocol code under `constellation/{frame,stream,capability,...}`.
Do not encode historical wave names, temporary implementation phases, or
review finding IDs in source file, type, crate, or package names.

ADR 0038's shell helper follows the same rule: the binary may be named
`d2b-guest-shell-runner` because it runs in the guest and owns shell
helper behavior; host terminal adapters, daemon shell admission, guestd shell
RPC handlers, and shpool translation do not share a locus-free `shell` or
`terminal` bucket.

### One side-effect owner

Every mutable host surface has exactly one owner. NixOS tmpfiles creates
static base roots, activation performs migrations and static repairs, the
broker mutates privileged runtime state, and `d2bd` owns daemon ledgers.
No wave may reintroduce broad recursive `chmod`, `chown`, `setfacl`, or
raw-path repair logic to make a test pass.

Runtime and service operations must also be idempotent. Start, stop, reap,
adopt, detach, and cleanup paths for sidecars such as Wayland proxy, QEMU,
QMP media attachments, virtiofsd, swtpm, USBIP, audio/video, guest shell
helpers, and shpool daemons must tolerate already-done and partially-cleaned
state without leaking resources or weakening the ownership boundary.

### Delete obsolete compatibility scaffolding by default

A placeholder, no-op option, legacy comment, bootstrap feature, or
transition wrapper must name a current invariant that justifies its
existence. If it cannot, it is deleted. A current caller is not enough to keep
the wrapper; the cleanup wave updates the caller and removes the wrapper.
Compatibility aliases and re-exports are not preserved for convenience and
are not public promises.

Keep migration machinery separate from compatibility shims. A migration tool
has an explicit source version, target version, validation path, and removal
or support policy. A compatibility shim silently keeps old behavior alive in
the current code path. The cleanup waves delete the latter while preserving
the former capability for future breakage.

NixOS option tombstones are migration machinery, not compatibility shims.
When a public option is removed or renamed, use `mkRemovedOptionModule` or
`mkRenamedOptionModule` so evaluation fails closed with an actionable message
that names the ADR/release path. Deleting the old behavior is required; making
the failure understandable is also required.

### Non-blocking task model by default

D2b's daemon, guest-control, provider, gateway, relay, and metrics paths
must have a declared task model. Async tasks may perform CPU-light state
transitions and non-blocking socket I/O; they must not perform unbounded
blocking filesystem walks, process spawning/waiting, synchronous network
calls, JSON reads on hot paths, or broker round trips while holding global
locks. Blocking work is isolated behind one of:

- a bounded `spawn_blocking` pool with per-operation admission limits;
- a dedicated worker thread or actor that owns the resource and exposes a
  bounded queue;
- a broker operation whose caller awaits a typed response without holding
  unrelated daemon locks;
- a startup/reconcile phase that is outside request-response hot paths.

Every long-running task needs a cancellation path, a bounded queue or
concurrency limit, and an observability surface for saturation. The minimum
runtime surface is queue depth, admission rejections/dropped requests,
backpressure triggers, blocking-pool/worker exhaustion, task age, and
blocking-duration histograms. Add runtime stall detection where the executor
supports it; static linting is necessary but not sufficient. This applies to
local-only paths and ADR 0032 constellation/provider paths: adding remote
transports must not move blocking I/O onto the daemon's request handlers.

The concurrency model has six hard rules:

1. **One runtime, structured supervision.** A daemon or broker that owns a
   Tokio runtime uses structured tasks, cancellation tokens, bounded queues,
   and semaphores for independent connection/background work. Raw
   `std::thread::spawn` / `std::thread::Builder` is not used for request,
   socket, relay, or retry loops.
2. **No nested runtimes.** Code must not hide async work inside
   `spawn_blocking` by constructing a fresh `tokio::runtime::Runtime`.
   Async guest-control, provider, and relay clients join the parent runtime
   and inherit its cancellation/backpressure model.
3. **Subprocess waits are owned work.** `std::process::Command` is not used
   in daemon/provider async hot paths. Subprocess execution either uses
   `tokio::process::Command`, a dedicated worker/actor, or a broker op whose
   blocking wait is explicitly outside request/task runtime workers.
4. **State ownership beats mutex contention.** Broad ledgers and supervisor
   maps move toward actor ownership with message-passing mutation. A mutex may
   protect a small in-memory invariant; it must not be held across filesystem,
   network, broker, process, or metrics I/O.
5. **Thread-local kernel state never runs on runtime workers.** Operations
   such as `setns`, `capset`, `prctl(PR_SET_SECCOMP)`, seccomp install,
   namespace setup, credential mutation, and `clone3` child setup run only on
   dedicated isolated OS-thread workers or inside tightly-scoped `pre_exec`
   / fork-child paths that follow async-signal-safety rules. The ban on raw
   thread-per-request handling does not ban isolated kernel workers whose
   only job is to avoid poisoning the async runtime.
6. **File descriptor receive is cancellation-safe.** Any `SCM_RIGHTS`
   receive path must guarantee that delivered fds are immediately wrapped in
   `OwnedFd` or closed before the next cancellation point. Use a
   cancellation-safe readiness pattern plus synchronous `recvmsg`, or an
   owned blocking worker that performs receive-and-wrap atomically.

Persistent-shell helper supervision inherits these rules. guestd may spawn
short-lived helper processes and manage shpool daemon adoption only through an
explicit owner with bounded stdout/stderr/framed-JSON reads, pidfd-backed
teardown, liveness handling, and no process-global `libshpool` calls inside
guestd.

The shpool socket trust model remains same-UID. Guest storage, runtime
directory, and socket cleanup may tighten workload-user permissions, but must
not broaden them to host users, daemon users, auxiliary sidecar users, or a
shared group as a convenience for management commands.

### Future compatibility bridge keys

Any future incompatible change that genuinely needs a temporary bridge must
carry a grep-friendly compatibility key. The key is part of the design
contract, not an optional comment. It appears in the ADR that authorizes the
bridge, in any code/docs implementing it, and in the validation or migration
record that proves it can be removed.

Key format:

```text
compat-ADR<NNNN>-added-<YYYYMMDD>-<surface>-<slug>
```

Required metadata beside the key:

| Field | Meaning |
| --- | --- |
| `adr` | Four-digit ADR number that authorizes the incompatible change and bridge. |
| `added` | Date the bridge first landed, in UTC `YYYYMMDD`. |
| `surface` | Closed surface family such as `cli`, `wire`, `bundle`, `option`, `test`, `schema`, `daemon`, `broker`, or `provider`. |
| `slug` | Short kebab-case identifier for the specific bridge. |
| `from` / `to` | Source and target contract versions or behavior names. |
| `owner` | Owning crate/module or docs/test surface. |
| `removeWhen` | Concrete removal condition, preferably a version floor or migration proof. |
| `validation` | Test, policy lint, migration record, or release gate that detects the bridge and proves its behavior. |

Example:

```text
compat-ADR0042-added-20260815-wire-v6-handshake
from=wire-v5
to=wire-v6
owner=packages/d2b-contracts
removeWhen=minSupportedWireVersion >= 6
validation=packages/d2b-contract-tests/tests/policy_compat.rs
```

Compatibility keys are reserved for deliberate future bridges, not for stale
legacy code found during cleanup. Cleanup waves delete unkeyed compatibility
logic. If an implementation wave believes unkeyed compatibility code still
protects a current invariant, that wave either converts it into an explicit
keyed bridge under the scheme above or deletes it while updating callers.

For JSON, bundle, and schema surfaces, the key metadata lives in the
generating source, not in the final artifact. Security-sensitive JSON keeps
`deny_unknown_fields`; do not add compatibility metadata fields to emitted
bundle/manifest/schema JSON unless that field is part of a deliberate schema
version bump. Put the `compat-ADR...` key in the Rust DTO docs/source, Nix
emitter, schema-generation code, or migration record that produces the
artifact.

### Prefer typed builders over string assembly

Repeated argv, readiness, process, path, lock, and audit shapes should move
toward typed builders that validate once and render once. Builders must
encode security invariants; they are not string-concatenation helpers.

### Remove unsafe code or quarantine it behind safe crates

The workspace already forbids unsafe code in most crates and quarantines
kernel FFI in narrow areas. Efficiency cleanup should make that stronger:
remove local `unsafe` when a maintained safe wrapper exists (`rustix`, `nix`,
or another focused crate), and keep unavoidable Linux FFI in a small audited
module with documented safety preconditions. New unsafe code is an
architecture finding, not a local implementation detail. If a syscall still
requires project-local unsafe, the ADR or module contract must explain why no
safe crate is sufficient and what would allow deleting the unsafe wrapper
later.

### Make generated output families explicit

Generated schemas, docs, completions, proto bindings, and fixtures are
separate artifact families. Gates should fail closed per family so a stale
completion does not require rerunning protobuf generation, and a protobuf
change does not obscure CLI docs drift.

## Multi-wave plan

### Wave 0 — Baseline, budgets, and compatibility deletion

Goal: make efficiency measurable and begin removing obsolete compatibility
logic immediately.

Tasks:

1. Establish repository budgets for the surfaces that predict iteration
   cost:
   - Rust hub size by file and module;
   - public DTO count by crate;
   - crate dependency fan-in/fan-out;
   - Nix module count and repeated full-`cfg.vms` / full-`cfg.envs` scans;
   - generated-artifact family runtime;
   - Layer-1 local wall time and peak Nix store / Cargo target growth.
2. Build a compatibility-removal inventory from tracked code and remove the
   low-risk entries in the same wave. The first pass targets bootstrap broker
   feature paths, no-op systemd scaffolding, retired option placeholders,
   old comments that describe deleted CLI/systemd modes, and shell wrappers
   superseded by Make targets. "Backward compatibility" alone is not a valid
   reason to keep code; if a current caller still exists, the wave updates the
   caller and deletes the compatibility path.
3. Identify migration/versioning machinery that must remain available for
   future incompatible changes, such as manifest/bundle schema versioning,
   migration commands, generated schemas, release notes, and cutover
   validation. This machinery is kept because it enables explicit future
   migrations, not because it preserves retired behavior today.
4. Add a policy lint or inventory report for `compat-ADR` keys. It must list
   each key, parse its ADR/date/surface/slug, and fail on malformed keys or
   missing required metadata once keyed bridges exist.
5. Categorize all code as one of:
   - contract;
   - pure model/policy;
   - adapter;
   - side-effect execution;
   - presentation;
   - test-only support.
6. Define an "efficiency proof" requirement for later waves: each wave must
   delete or consolidate a named surface, not merely move code.

Validation:

- repository inventory lists any remaining compatibility code and the
  current invariant, not future compatibility, that keeps it alive;
- repository inventory separately lists future migration/versioning machinery
  and confirms it is not mixed into normal hot paths as silent compatibility;
- compatibility-key inventory is ready to detect future keyed bridges and
  rejects malformed `compat-ADR...` markers;
- baseline report is generated from tracked files only and does not become
  release documentation.

Exit criteria:

- every later wave has an owned list of surfaces to remove or consolidate;
- low-risk retired compatibility paths are deleted before the broader
  refactors begin;
- future schema/version/cutover machinery remains available as explicit
  migration infrastructure;
- future compatibility bridges have a standard key shape with ADR/date/surface
  metadata for later cleanup;
- no cleanup implementation wave can add a compatibility stub; it must update
  the caller and delete the old path instead. Future breaking-change bridges
  must use the keyed `compat-ADR...` scheme above.

### Wave 1 — Generated artifact family consolidation

Goal: reduce Nix and Rust drift by making bundle-artifact emission a single
pattern.

Tasks:

1. Add a Nix helper for generated bundle artifacts that owns:
   - `schemaVersion`;
   - `data`;
   - compact `jsonText`;
   - `pkgs.writeText` / `pkgs.runCommand` output;
   - `options.d2b._bundle.<artifact>`;
   - private `/etc/d2b/<artifact>.json` installation.
2. Apply it to the repeated host/process/privilege/closure/storage/sync
   emitters while preserving each artifact's schema, path, owner, and mode.
3. Keep `bundle.json` hashing special, but factor the common install and
   `_bundle` option wiring.
4. Create named generated-artifact families:
   - bundle schemas and DTO schemas;
   - CLI shell artifacts and CLI JSON schemas;
   - daemon API and error-code docs;
   - guest-control protobuf / ttRPC bindings;
   - rendered fixture artifacts.
5. Split drift reporting by family while keeping one `make test-drift`
   entrypoint.

Primary targets:

- `nixos-modules/bundle.nix`;
- `nixos-modules/host-json.nix`;
- `nixos-modules/processes-json.nix`;
- `nixos-modules/privileges-json.nix`;
- `nixos-modules/closures-json.nix`;
- `nixos-modules/storage-json.nix`;
- `nixos-modules/sync-json.nix`;
- `packages/xtask/src/main.rs`;
- `tests/unit/gates/drift-check.sh`.

Validation:

- existing bundle/manifest/schema drift gates remain green;
- generated file bytes are unchanged unless a schema bump is explicitly
  part of the wave;
- no new shell gate is introduced.

Exit criteria:

- adding a new bundle artifact requires one helper invocation plus Rust DTO
  and schema registration, not a copy of 40-80 lines of emitter boilerplate.

### Wave 2 — Normalized Nix VM/env indexes

Goal: evaluate host, network, observability, USBIP, and process modules from
one normalized model instead of repeated full-tree scans.

Tasks:

1. Introduce an internal normalized index under the Nix module layer, with
   at least:
   - enabled VMs;
   - enabled normal workload VMs;
   - framework-declared system VMs;
   - enabled envs;
   - workloads by env;
   - graphics/audio/video/TPM/USBIP/observability subsets;
   - qemu-media/manual-only/runtime-media capability subsets;
   - persistent-shell enablement and configured shell limits;
   - declared net VM per env;
   - stable per-env port/name/IP metadata;
   - provider/runtime capability summary.
2. Replace local recomputation in host and network modules with reads from
   the index.
3. Delete retired interface-name fallback paths that no current host can
   consume; move any still-required derivation into a single helper with
   typed inputs.
4. Keep index generation pure and read-only; it may not perform activation,
   tmpfiles, broker, or host mutation work.
5. Build the index only from base-level declared inputs that cannot depend on
   the index itself, such as option values, enable flags, env names, VM names,
   component toggles, and explicit IDs. Do not compute the index from fully
   evaluated config subtrees whose definitions may read the index, or the
   module system can recurse.

Primary targets:

- `nixos-modules/lib.nix`;
- `nixos-modules/host.nix`;
- `nixos-modules/network.nix`;
- `nixos-modules/net.nix`;
- `nixos-modules/options-envs.nix`;
- `nixos-modules/options-vms.nix`;
- `nixos-modules/processes-json.nix`.

Validation:

- representative examples render byte-identical manifest/process/network
  artifacts unless intentional changes are documented;
- eval cases cover multi-env, net VM, USBIP, observability, and graphics
  selection from the index;
- recursion guards or focused eval cases prove index consumers do not create
  cycles;
- `nix flake check --no-build` does not regress in eval time.

Exit criteria:

- modules no longer open-code "enabled VMs in env X" or "USBIP envs" scans;
- adding a per-VM component adds one index classification, not repeated
  scans across modules.

### Wave 3 — Contract crate and DTO boundary cleanup

Goal: move shared JSON/wire/output models out of CLI and daemon hub files.

Tasks:

1. Classify all serde/json schema structs in `d2b`, `d2bd`,
   `d2b-contracts`, and `d2b-core` as one of:
   - wire contract;
   - bundle contract;
   - CLI output contract;
   - daemon internal state;
   - presentation-only view.
2. Move stable CLI output DTOs and daemon API DTOs to a contract crate
   boundary. The exact home may be an expanded `d2b-contracts` or a narrower
   `d2b-contracts` crate, but dependency direction must stay acyclic:
   presentation crates depend on contracts; contracts do not depend on CLI
   or daemon execution crates.
3. Leave formatting, terminal behavior, and command dispatch in `d2b`.
4. Leave listener loops, process supervision, and mutation orchestration in
   `d2bd`.
5. Regenerate CLI schemas and daemon API docs from the new DTO homes.

Primary targets:

- `packages/d2b/src/lib.rs`;
- `packages/d2bd/src/lib.rs`;
- `packages/d2b-contracts/src/public_wire.rs`;
- `packages/d2b-core/src/*`;
- `packages/xtask/src/main.rs`;
- `docs/reference/cli-output/`;
- `docs/reference/daemon-api.md`.

Validation:

- generated docs/schemas are byte-for-byte equivalent except source-location
  churn and intentional module-path changes;
- CLI JSON contract tests still deserialize with `deny_unknown_fields`;
- dependency-direction policy continues to forbid CLI/daemon backedges.

Exit criteria:

- `d2b/src/lib.rs` is presentation and dispatch, not a schema warehouse;
- `d2bd/src/lib.rs` is orchestration and module wiring, not the canonical
  source for public output schemas.

### Wave 4 — Rust hub-file decomposition

Goal: make the largest Rust crates understandable without changing behavior.

Tasks:

1. Split crate-root hub files by architectural role:
   - accept loop and auth admission;
   - lifecycle command dispatch;
   - VM status/read models;
   - guest-control client/server bridge;
   - gateway-mode routing;
   - QEMU media lifecycle;
   - provider-neutral local runtime/service status and capability adapters;
   - terminal-v1 streaming plus exec and persistent-shell adapters;
   - host doctor/read-only checks;
   - CLI command groups.
2. Replace broad imports in hub files with narrow module APIs.
3. Move test-only helpers behind `cfg(test)` modules or dedicated
   test-support features/crates.
4. Keep public crate APIs stable unless an earlier contract wave explicitly
   moved the public type.
5. Remove comments that only narrate deleted migration phases; keep comments
   that explain current invariants.

Primary targets:

- `packages/d2bd/src/lib.rs`;
- `packages/d2b/src/lib.rs`;
- `packages/d2b-priv-broker/src/runtime.rs`;
- `packages/d2b-priv-broker/src/live_handlers.rs`;
- `packages/d2b-core/src/bundle_resolver.rs`;
- `packages/d2b-guestd/src/service.rs`;
- `packages/d2b-guest-shell-runner/*` once ADR 0038 implementation
  introduces the excluded helper workspace.

Validation:

- no public JSON/wire behavior changes;
- `cargo test` and contract tests pass with warnings denied;
- generated daemon API docs remain accurate after source-line churn.

Exit criteria:

- no crate root remains the place where unrelated subsystems accumulate by
  default;
- new command or daemon feature work has an obvious module home.

### Wave 5 — Runner/process builder DSL

Goal: encode process launch invariants once and remove repeated argv,
profile, readiness, and audit assembly.

Tasks:

1. Define a typed runner/process builder that composes:
   - role id and principal;
   - minijail profile id;
   - argv renderer;
   - environment;
   - fd requirements;
   - cgroup leaf;
   - readiness predicate;
   - writable path references;
   - restart/adoption class;
   - audit operation id.
2. Convert existing pure argv generators to builder-backed renderers without
   weakening their current tests.
3. Make `processes.json`, minijail profiles, storage references, and broker
   `SpawnRunner` requests consume the same builder model.
4. Model qemu-media QEMU runner startup, paused-before-boot readiness, QMP
   socket readiness, and media boot transaction dependencies without leaking
   QMP internals into the generic builder.
5. Model persistent-shell helper execution and shpool daemon adoption through
   the same process/isolation vocabulary where it fits, while keeping the
   shpool protocol private to the guest helper and guestd adapter.
6. Keep provider-specific launch details behind provider adapters, not in
   the generic builder.

Primary targets:

- `packages/d2b-host/src/*_argv.rs`;
- `packages/d2b-host/src/runner_argv_regenerator.rs`;
- `packages/d2b-host/src/qemu_media_argv.rs` if present, or the current
  qemu-media argv renderer home;
- `packages/d2b-core/src/processes.rs`;
- `nixos-modules/processes-json.nix`;
- `nixos-modules/minijail-profiles.nix`;
- `packages/d2b-priv-broker/src/ops/*`;
- `packages/d2bd/src/supervisor/*`.

Validation:

- existing argv-shape and minijail-validator tests remain green;
- qemu-media tests cover paused-before-boot readiness, QMP fd/block/device
  transaction ordering, and failure cleanup for boot media and runtime hotplug
  dependencies;
- rendered `processes.json` and minijail profile fixtures are unchanged
  unless an intentional schema bump is made;
- adding a new runner role requires a lifecycle matrix plus builder
  implementation, not scattered JSON/string logic.

Exit criteria:

- the runner role lifecycle matrix becomes executable structure rather than
  prose that must be manually mirrored in Nix and Rust.

### Wave 6 — Side-effect ownership cleanup

Goal: make host filesystem, ACL, lock, cleanup, and migration work follow a
single ADR 0034 contract.

Tasks:

1. Audit every tmpfiles, activation, broker storage op, daemon ledger write,
   and runner-created path against `storage.json` / `sync.json`.
2. Move broad static path creation to tmpfiles only when the path is a base
   root and has no dynamic ACL/ownership state.
3. Move privileged dynamic repair to broker ops addressed by opaque storage
   ids.
4. Limit activation to migrations and static repairs that cannot be moved to
   tmpfiles or broker execution.
5. Delete no-op systemd and activation scaffolding once no consumers remain.
6. Replace inherited or incidental file locks with sync-contract-owned OFD
   locks.
7. Treat qemu-media boot images, physical media registry state, QMP sockets,
   and stale proxy cleanup as runtime-owned storage/cleanup surfaces with one
   broker or daemon owner per mutation.
8. Treat persistent-shell runtime directories, shpool sockets, helper logs, and
   boot-scoped shell metadata as generated storage/sync rows or guest-owned
   ledgers; do not add host activation or daemon raw-path repair as a shortcut.
   Helper logs must have an explicit rotation or maximum-size policy before
   long-lived shells ship.

Primary targets:

- `nixos-modules/host-activation.nix`;
- `nixos-modules/host-daemon.nix`;
- `nixos-modules/bundle.nix`;
- `nixos-modules/storage-json.nix`;
- `nixos-modules/sync-json.nix`;
- `packages/d2b-priv-broker/src/ops/*`;
- `packages/d2bd/src/storage_lifecycle.rs`.

Validation:

- storage lifecycle contract tests cover every mutable host path touched by
  activation, broker, or daemon code;
- guestd and guest-owned lifecycle tests cover persistent-shell runtime
  directories, shpool sockets, helper logs, and boot-scoped shell metadata
  cleanup/adoption where those paths are not host-mutable;
- no new raw-path privileged mutation enters daemon code;
- restart/adoption behavior remains continuation-safe.

Exit criteria:

- when a path is wrong, there is one generated storage row and one repair
  owner; contributors do not patch around failures with one-off ACL code.

### Wave 7 — Test driver thinning and native policy migration

Goal: keep coverage fail-closed while reducing shell orchestration and
duplicate work.

Tasks:

1. Make `make test-*` the single stable test vocabulary; update any current
   CI or maintainer invocation that still names a legacy alias, then delete
   the alias in the same wave.
2. Collapse duplicate shell linting: `test-lint` owns syntax, shellcheck,
   and Nix parse; `static-fast-tier0.sh` is retired after any remaining
   callers move to `test-lint`.
3. Move source-tree policy checks that need parsing or cross-file reasoning
   into Rust policy tests under `packages/d2b-contract-tests/tests`.
4. Keep shell only for orchestration that genuinely needs ecosystem tools,
   Nix evaluation, or platform setup.
5. Split generated-artifact drift failures by artifact family as defined in
   Wave 1.
6. Avoid running the same fixture build from both `test-rust` and
   `test-flake` unless the second run proves a different boundary.

Primary targets:

- `Makefile`;
- `tests/test-lint.sh`;
- `tests/static-fast-tier0.sh`;
- `tests/test-rust.sh`;
- `tests/test-drift.sh`;
- `tests/unit/gates/drift-check.sh`;
- `tests/unit/meta/*`;
- `packages/d2b-contract-tests/tests/*`;
- `.github/workflows/*`.

Validation:

- migration records are updated for any retired shell gate and successor
  test when the test model requires it; do not replace a retired bash wrapper
  with another per-test cargo wrapper;
- pinned test inventory confirms no coverage silently disappears;
- CI still runs every Layer-1 family, but with fewer repeated setup paths.

Exit criteria:

- contributors can answer "what should I run?" from the Makefile alone;
- shell scripts orchestrate tools, while policy and contract logic lives in
  typed tests.

### Wave 8 — Workspace, dependency graph, and naming taxonomy simplification

Goal: reduce compile cost and architectural ambiguity in the Rust workspace,
and make crate/file/type names sort by concern.

Tasks:

1. Audit crates with one source file or no independent release boundary:
   keep them only if they enforce dependency direction, feature isolation,
   static-link boundaries, or provider plug-in boundaries.
2. Decide whether `d2b-priv-broker` remains a separate workspace. If it
   must remain separate for security or build reasons, document the reason
   and remove duplicated dependency/lint declarations through shared
   metadata where possible.
3. Move provider traits and capability descriptors to the narrowest crates
   that ADR 0032 needs; avoid generic provider crates that only forward
   types.
4. Gate heavyweight provider dependencies behind feature flags or separate
   binaries so the local-only path does not compile cloud providers.
5. Ensure test-support features do not enter production builds.
6. Classify every crate by concern prefix (`core`, `ipc`, `daemon`, `broker`,
   `host`, `guest`, `gateway`, `provider`, `constellation`, `contract`, or
   `cli`). Public binary crate names are allowed historical exceptions; library
   crates and modules are not.
7. Rename or delete ambiguous crates and modules as part of cleanup waves
   instead of preserving compatibility aliases. Initial audit targets include
   `d2b-host-providers` (ambiguous host/provider boundary),
   locus-free runner names, and daemon modules that mix request handling,
   background task ownership, and presentation DTOs in one root.
8. Extend `docs/reference/naming-conventions.md` from host-visible resource
   names to include crate/module/type naming so future code review has one
   canonical taxonomy.
9. Add policy coverage that rejects new crate names, module names, and public
   type names containing process markers, ambiguous catch-all words, or the
   `remote` prefix where a precise concern exists.
10. Keep wire compatibility separate from Rust naming. Wire field names can
   remain stable for schema/version reasons while Rust types/modules move to
   concern-first names behind generated schemas.
11. Keep the persistent-shell helper as an excluded workspace only while it
   needs the unsafe/libshpool/static-link boundary described in ADR 0038.
   Its separate lockfile, deny policy, and static packaging gates are part of
   the justification, not boilerplate to be silently dropped.

Primary targets:

- `packages/Cargo.toml`;
- `packages/d2b-priv-broker/Cargo.toml`;
- `packages/d2b-constellation-*`;
- `packages/d2b-provider-*`;
- `packages/d2b-gateway*`;
- `packages/d2b-host-providers`;
- `packages/d2b-guest-shell-runner/*` once introduced;
- `flake.nix` Rust package source construction;
- `docs/reference/naming-conventions.md`.

Validation:

- dependency-direction policy remains green;
- local-only CLI/daemon build does not pull provider-only dependencies unless
  explicitly enabled;
- supply-chain gates still cover every lockfile that can ship;
- naming policy accepts the intentional public binary exceptions and rejects
  new ambiguous crate/module/type names;
- generated docs/schemas preserve wire compatibility where Rust names move.

Exit criteria:

- each crate has a sentence-long reason to exist and an explicit concern
  prefix or documented public-binary exception;
- each excluded workspace has a sentence-long reason to stay excluded, the
  validation gates that cover it, and the condition under which it can rejoin
  or be deleted;
- compile graphs match operator paths: local-only users do not pay for
  provider experiments;
- reviewers can identify host/guest/gateway/provider/constellation ownership
  from crate and module paths without opening the implementation first.

### Wave 9 — v2 provider integration simplification

Goal: make ADR 0032 extensibility reduce code, not multiply adapters.

Tasks:

1. Fold Wave 0 provider abstraction work from ADR 0032 and the local
   hypervisor seam from ADR 0037 into the efficiency taxonomy from this ADR:
   provider code is an adapter, not a new place to define core lifecycle
   semantics.
2. Define a single provider capability descriptor that covers local
   hypervisors, host substrates, display transports, cloud sandboxes, remote
   full hosts, and runtime/service capability denial.
3. Keep local fast path as one provider instance with no relay/gateway
   overhead when no remote realm is configured.
4. Make unsupported operations fail through typed capability denial, not
   by probing for optional code paths.
5. Require each provider to declare which generic builders/contracts it
   consumes: runner/process, storage/sync, display, transport, audit, and
   observability.
6. Convert qemu-media's current enrollment compatibility into the ADR 0037
   start-time boot media and runtime QMP hotplug model. The removal is not a
   generic USBIP cleanup; it is a provider-adapter simplification that keeps
   physical media redaction and broker-owned fd opening intact.
7. Make Wayland/window policy consume runtime display capabilities rather than
   provider names or process-node names.
8. Ensure capability-denial errors preserve their typed reason across daemon,
   guest-control, and CLI JSON boundaries. Disabled persistent shell, shell
   capacity exhaustion, qemu-media unsupported guest-control, and manual-only
   autostart denials must produce actionable operator messages before side
   effects.

Primary targets:

- `packages/d2b-constellation-core`;
- `packages/d2b-constellation-provider`;
- `packages/d2b-constellation-router`;
- `packages/d2b-provider-aca`;
- `packages/d2b-provider-relay`;
- qemu-media provider/adapter modules in `packages/d2bd` and
  `packages/d2b-priv-broker`;
- `packages/d2b-gateway-runtime`;
- `nixos-modules/gateway-vm.nix`;
- ADR 0032 implementation wave notes.

Validation:

- local single-host lifecycle path is unchanged and does not instantiate
  gateway/relay/provider credentials;
- qemu-media unsupported guest features fail before side effects through the
  shared capability-denial protocol;
- capability-denial DTOs round-trip through generated schemas without
  collapsing distinct denial reasons into opaque provider errors;
- qemu-media autostart/manual-only reasons are visible through status/doctor
  without starting the runtime;
- provider-managed sandbox paths use provider exec/log/display subsets and
  do not pretend to be local guestd VMs;
- capability denial is covered by unit/contract tests.

Exit criteria:

- adding a provider is mostly adapter code plus capability records, not a new
  copy of lifecycle, display, transport, audit, and storage logic.

### Wave 10 — Threading, task, and non-blocking I/O model

Goal: make d2b's concurrency model explicit and keep request/task threads
from doing unbounded blocking I/O.

Tasks:

1. Inventory every daemon, broker, guest-control, gateway, relay, provider,
   metrics, CLI, qemu-media/QMP, terminal streaming, and persistent-shell path
   that performs filesystem, socket, process, DNS/HTTP, JSON parse/read,
   guest-control, shpool helper, or broker IPC work.
2. Classify each operation by owner and execution class:
   - async non-blocking socket I/O;
   - bounded blocking filesystem/process work;
   - broker-owned privileged work;
   - startup/reconcile-only work;
   - CPU-bound serialization or policy computation;
   - long-running stream/relay task.
3. Define a task-supervision contract with:
   - task owner;
   - cancellation trigger;
   - maximum concurrency;
   - queue bound/backpressure behavior;
   - shutdown ordering;
   - saturation metric/log posture;
   - whether blocking work may run on a generic blocking pool or needs a
     dedicated worker/actor.
4. Remove ad hoc thread spawning from daemon/provider hot paths in favor of
   structured task groups, owned workers, or bounded blocking pools.
5. Forbid holding global daemon locks while awaiting broker IPC, reading
   files, spawning processes, scraping metrics, or performing provider
   network calls.
6. Move synchronous JSON reads and bundle parsing off list/status/doctor hot
   paths through explicit snapshot/cache ownership and bundle-hash
   invalidation.
7. Require provider/relay transports from ADR 0032 to expose async traits or
   actor-owned blocking adapters so remote I/O cannot stall local lifecycle
   request handling.
8. Replace OS-thread-per-connection handling in daemon request paths with
   runtime-owned tasks and bounded admission. The initial audit includes
   `d2bd` connection handling for public socket requests, exec-owner
   sessions, gateway display sessions, and exec writer plumbing.
9. Delete runtime-in-runtime bridge patterns. Guest-control probes and ttRPC
   clients become natively async or actor-owned adapters; they do not consume
   the blocking pool just to create a private single-thread Tokio runtime.
10. Move broker and daemon background retry loops under structured
   supervision. ACL refresh retries, vsock/observability retries, and similar
   polling loops use cancellation-aware tasks with bounded retry policy, not
   detached sleep loops.
11. Classify subprocess execution sites. Host mutation commands such as
   `systemctl`, `mkfs`, `nft`, NetworkManager tools, detached exec
   reconciliation, persistent-shell helper processes, qemu-media QEMU/QMP
   readiness helpers, and activation helpers either become async subprocesses,
   broker-owned blocking work, actor/worker-owned process work, or pure Rust
   operations with explicit backpressure.
12. Move broad supervisor state and pidfd/task ledgers toward actor ownership
   so mutation is serialized by the owner task instead of guarded by global
   synchronous mutexes that can be touched by blocking worker code.
13. Extract terminal-v1 as a shared owner model for interactive exec and shell:
   stdin chunk offsets, output cursors, resize sequencing, raw-mode guards,
   owner teardown, attach slot accounting, force/detach/kill semantics, and
   slow-reader/output-gap handling live in one substrate with adapters for the
   distinct public contracts.
14. Keep guestd shell helper management off async hot paths: bounded management
   frames, bounded diagnostic streams, pidfd-backed helper lifecycle, and
   shpool daemon readiness/adoption use explicit owners and cancellation
   paths.

Primary targets:

- `packages/d2bd/src/lib.rs`;
- `packages/d2bd/src/autostart.rs`;
- `packages/d2bd/src/concurrency.rs`;
- `packages/d2bd/src/exec_session*.rs`;
- `packages/d2bd/src/shell*`;
- `packages/d2bd/src/guest_control_bridge.rs`;
- `packages/d2bd/src/guest_control_*`;
- `packages/d2bd/src/supervisor/*`;
- `packages/d2bd/src/metrics.rs`;
- `packages/d2bd/src/ch_stats.rs`;
- `packages/d2b-priv-broker/src/live_handlers.rs`;
- `packages/d2b-priv-broker/src/runtime.rs`;
- `packages/d2b-priv-broker/src/ops/*`;
- `packages/d2b-gateway-runtime/*`;
- `packages/d2b-provider-*`;
- `packages/d2b-constellation-*`;
- `packages/d2b-core/src/bundle_resolver.rs`;
- `packages/d2b-guestd/src/service.rs`;
- `packages/d2b-guest-shell-runner/*` once introduced.

Validation:

- policy lint or Rust tests identify blocking APIs in async/request-handler
  modules and require an explicit allowlist with owner and execution class;
- request handlers do not hold global locks across I/O awaits or blocking
  work;
- every long-running stream/relay/task path has cancellation and bounded
  queue/backpressure coverage;
- list/status/doctor hot paths use snapshots or cached reads with freshness
  metadata where appropriate;
- no nested Tokio runtime construction is used to service request-path async
  work;
- subprocess sites have explicit execution-class coverage: async process,
  actor/worker, broker-owned blocking op, or startup/reconcile only;
- terminal-v1 tests prove exec and shell share streaming semantics without
  merging their public lifecycle contracts;
- terminal-v1 tests prove stdout/stderr/output rings, resize events, cursor
  state, detach/force/kill notifications, and slow-reader/output-gap handling
  use bounded queues and close cleanly on disconnect;
- shell helper stream caps, management framing, pidfd teardown, and shpool
  readiness/adoption are covered by unit, binary, or VM tests appropriate to
  the Linux boundary being exercised.

Exit criteria:

- contributors can name where blocking work is allowed and why;
- local lifecycle requests cannot be starved by provider HTTP calls, metric
  scraping, filesystem walks, or subprocess waits;
- the v2 transport-neutral API has one task model for CLI, future web UI,
  local daemon peers, gateway-backed realms, and remote providers.
- daemon/broker background tasks are owned, cancellable, and visible to
  shutdown/doctor/metrics surfaces.

### Wave 11 — Runtime hot-path efficiency

Goal: improve runtime behavior where simplicity and performance align.

Tasks:

1. Keep daemon state in immutable snapshots with narrow mutation points so
   list/status/doctor reads do not lock large global structures.
2. Replace repeated JSON parse/read cycles for static bundle artifacts with
   an explicitly versioned in-memory cache invalidated by bundle hash.
3. Batch broker requests where the ordering is contractually one operation,
   such as host prepare phases, storage repair sets, or network reconcile,
   while keeping per-op audit records.
   Batching must include audit burst controls: bounded batch sizes, stable
   low-cardinality labels, rate-limited diagnostic summaries, and metrics
   that report dropped/throttled audit emission if the fail-closed policy ever
   refuses a batch due to audit pressure.
4. Use pidfd and cgroup discovery helpers consistently so restart/adoption
   code does not duplicate kernel traversal.
5. Keep expensive observability scraping off command-response paths; surface
   cached health with timestamps and explicit freshness.
6. Cache static runtime capability records and service specs by bundle hash so
   list/status/doctor do not re-read provider-specific artifacts to explain
   qemu-media unsupported operations or persistent-shell availability.
7. Keep terminal streaming hot paths allocation- and lock-conscious: cursor
   state, attach slots, and resize events are owned by session actors rather
   than repeatedly walking global daemon maps.
8. Keep instrumentation and exporters isolated from core lifecycle. Scrape
   failures, exporter timeouts, stale-cache reads, helper diagnostic failures,
   and provider telemetry errors must surface degraded/freshness state without
   crashing the daemon, blocking lifecycle requests, or poisoning cached health.
9. Bound telemetry cardinality and retention for new runtimes. Persistent shell
   session names, attach ids, shell instance ids, QMP object ids, physical media
   selectors, host media paths, image paths, registry paths, and helper log
   paths are not metric labels; helper diagnostics and logs are byte-capped and
   rotated or size-limited before they can persist indefinitely.

Primary targets:

- `packages/d2bd/src/concurrency.rs`;
- `packages/d2bd/src/supervisor/*`;
- `packages/d2bd/src/storage_lifecycle.rs`;
- `packages/d2bd/src/ch_stats.rs`;
- `packages/d2bd/src/metrics.rs`;
- `packages/d2b-priv-broker/src/runtime.rs`;
- `packages/d2b-core/src/bundle_resolver.rs`.

Validation:

- no stale-bundle dispatch after host switch;
- broker audit remains per typed operation;
- batched operations cannot create unbounded audit/log cardinality or burst
  volume;
- list/status/doctor output includes freshness where cached health is used;
- cached runtime/shell capability reads cannot dispatch stale bundle data after
  host switch.
- exporter and scrape failures degrade only the relevant health surface and do
  not block VM lifecycle operations;
- metric and log tests reject shell session names, QMP object ids, media paths,
  and helper paths as labels or unbounded fields.

Exit criteria:

- runtime speedups come from fewer repeated reads/parses/locks, not from
  skipping validation or swallowing errors.

### Wave 12 — Example, template, and documentation diet

Goal: make shipped docs/examples teach the current model without preserving
historical scaffolding.

Tasks:

1. Remove comments in examples/templates that explain retired modes instead
   of current usage.
2. Keep one minimal example, one multi-env example, one graphics/workstation
   example, one observability example, and one identity-composition example
   only when each teaches a distinct public surface.
3. Move deep architecture history out of how-to docs and into ADRs.
4. Ensure README and reference docs describe the same CLI, daemon, and bundle
   surfaces as the code.
5. Keep process markers out of shipped consumer docs and released changelog
   sections.
6. Describe qemu-media as a narrow local runtime with explicit unsupported
   capabilities, not as a general NixOS VM mode or USBIP replacement.
7. Describe persistent shell as a first-class `d2b shell` UX over
   guest-control and shpool-backed guest state, not as SSH, tmux/screen, or a
   public shpool protocol.

Primary targets:

- `README.md`;
- `docs/how-to/*`;
- `docs/reference/*`;
- `examples/*`;
- `templates/default/*`;
- `CHANGELOG.md`;
- ADR 0036, ADR 0037, and ADR 0038 cross-references when shipped docs describe
  their surfaces.

Validation:

- docs links and ADR index checks pass;
- examples still evaluate;
- no user-facing docs describe retired bash/systemd surfaces as live.

Exit criteria:

- new users see the current architecture first; historical context remains
  available in ADRs but does not dominate day-to-day docs.

### Wave 13 — Unsafe-code removal and FFI quarantine

Goal: remove avoidable project-local `unsafe` and make unavoidable kernel FFI
small, audited, and replaceable by maintained safe wrappers when available.

Tasks:

1. Inventory every `unsafe` block, `unsafe fn`, `unsafe impl`, and
   `allow(unsafe_code)` in tracked Rust code. The initial audit starts with
   the broker FFI quarantine and tests plus the host activation helper; when
   the persistent-shell helper lands, include its prelude/libshpool boundary in
   the same inventory:
   - `packages/d2b-priv-broker/src/sys.rs`;
   - `packages/d2b-priv-broker/src/seccomp_compile_tests.rs`;
   - `packages/d2b-priv-broker/tests/socket_activation.rs`;
   - `packages/d2b-host-activation-helper/src/main.rs`;
   - `packages/d2b-guest-shell-runner/*`.
2. For each site, choose one disposition:
   - replace with a maintained safe API (`rustix`, `nix`, `capctl`, or another
     focused crate);
   - move behind the existing broker FFI quarantine with a safe wrapper and
     documented safety preconditions;
   - delete because the compatibility or legacy path is removed;
   - keep temporarily only when no safe crate exposes the required Linux API.
3. Prefer `rustix`/`nix` safe wrappers for fd-relative filesystem operations,
   fd flags, owned-fd conversion helpers, directory iteration, pidfd helpers,
   waits, and mount/capability primitives where they preserve the same kernel
   semantics.
4. Keep raw `libc` only for syscalls or structs not safely exposed by a
   maintained crate, such as specific `clone3`, TUN/TAP ioctl, seccomp, or
   capability operations that lack a suitable wrapper.
5. Require `openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_MAGICLINKS`, and
   symlink refusal for path resolution that crosses into untrusted,
   guest-controlled, or externally writable filesystem boundaries. Plain
   `openat` is acceptable only for already-trusted anchored paths whose parent
   ownership/mode invariants are proven by the storage contract.
6. Add policy coverage that fails on new unsafe outside the approved
   quarantine and prints the owning ADR/module rationale for each remaining
   site.
7. Add policy coverage that fails on new blocking I/O in async/request-handler
   modules unless the call is routed through the approved task model
   (`spawn_blocking`, actor/worker, broker op, or startup/reconcile phase)
   with a bounded queue/concurrency limit.
8. Treat `libshpool::run`, helper privilege-drop preludes, pidfd acquisition,
   fd cleanup, terminal controlling-tty setup, and process-global environment
   mutation as unsafe-boundary work even when the Rust `unsafe` keyword is
   hidden inside an upstream crate. The ADR/module contract must name the
   process isolation and test evidence that keep guestd and daemon runtime
   workers safe.
9. QEMU media fd paths use close-on-exec defaults, explicit ownership transfer,
   and deterministic cleanup. Fds opened by the broker are `O_CLOEXEC` until the
   deliberate QMP handoff, and any failed handoff closes the fd before the next
   cancellation point.
10. Persistent-shell daemon adoption is PID-reuse-safe. guestd may use pidfds,
    systemd unit identity, or another kernel-backed identity proof, but it must
    never treat a numeric pid alone as authority after a helper exits or guestd
    restarts.
11. Helper preludes reset inherited signal masks and dispositions where needed,
    establish the intended process/session/controlling-terminal relationship,
    and verify the final non-root credential/capability state before touching
    the shpool socket or spawning descendants.

Primary targets:

- `packages/d2b-priv-broker/src/sys.rs`;
- `packages/d2b-priv-broker/src/seccomp_compile_tests.rs`;
- `packages/d2b-priv-broker/tests/socket_activation.rs`;
- `packages/d2b-host-activation-helper/src/main.rs`;
- `packages/d2b-guest-shell-runner/*` once introduced;
- `packages/*/Cargo.toml` lint settings;
- `packages/d2b-contract-tests/tests/policy_*.rs`.

Validation:

- workspace lint policy continues to forbid unsafe by default;
- each remaining unsafe site has a documented safety contract and no safe
  wrapper replacement available at the pinned dependency versions;
- thread-local kernel mutations are isolated from async runtime workers;
- `SCM_RIGHTS` fd receive paths use a cancellation-safe receive-and-wrap
  pattern;
- untrusted path resolution uses `openat2`/equivalent anchored resolution;
- tests cover any safe-wrapper conversion with behavior-equivalent fd,
  syscall, or process semantics;
- no production crate broadens `allow(unsafe_code)` beyond the quarantine;
- blocking-I/O policy coverage rejects new unbounded sync filesystem,
  process, network, or JSON reads in request/async hot paths;
- the persistent-shell helper remains the only place that may call shpool's
  process-global entrypoints, and guestd never calls them in-process;
- tests prove helper process-global side effects, including signal handling,
  environment mutation, tracing setup, daemonization, and stdio/fd changes, do
  not leak back into guestd or daemon runtime workers.

Exit criteria:

- avoidable unsafe is gone;
- unavoidable unsafe is centralized, audited, and tied to a removal condition
  such as "replace when rustix/nix exposes this safe API";
- runtime worker threads are never poisoned by namespace/capability/seccomp
  mutations;
- fd-passing code cannot leak fds on task cancellation;
- new blocking I/O and new unsafe code are prevented by policy tests or
  review gates, not just by convention.

### Wave 14 — Recurring efficiency ratchet

Goal: prevent re-growth.

Tasks:

1. Add review checklist items for:
   - new crate justification;
   - new shell gate justification;
   - new generated artifact family registration;
   - prohibition on new compatibility surfaces;
   - required `compat-ADR<NNNN>-added-<YYYYMMDD>-<surface>-<slug>` key for any
     explicitly authorized future bridge;
   - new full-tree Nix scan justification;
   - new public DTO location;
   - task/concurrency model for any new daemon/provider/relay path;
   - unsafe-code disposition for any new kernel/FFI work;
   - concern-prefix naming for new crates, modules, and public Rust types.
2. Add policy lints that prevent backsliding:
   - no unapproved `std::fs`, `std::net`, `std::process::Command`, blocking
     HTTP/DNS clients, or synchronous JSON file reads in daemon/provider
     request-handler and async task modules;
   - no new `unsafe`, `unsafe fn`, `unsafe impl`, `allow(unsafe_code)`, or
     generated unsafe bindings outside the approved quarantine;
   - every allowlisted blocking/unsafe site carries an owner, execution class
     or safety contract, and removal condition.
3. Add runtime telemetry that catches what static lints miss:
   - executor/task stall counters or histograms where supported;
   - blocking-section duration histograms;
   - blocking-pool/worker queue depth and saturation;
   - dropped/admission-refused counts for bounded queues;
   - audit batch size, throttling, and cardinality posture.
4. Add lightweight budgets to policy tests where they are stable enough to
   avoid churn. Budgets should warn or fail only on trends that predict real
   maintenance cost, not on arbitrary line counts.
5. Require every future ADR with implementation waves to include a
   simplification/deletion row.
6. Periodically retire stale ADR process markers from unreleased changelog
   prose before release.

Validation:

- policy tests enforce the checklist only where mechanically reliable;
- blocking-I/O and unsafe-code policy lints fail closed on new unapproved
  sites;
- runtime telemetry exposes executor stalls, blocking-section duration,
  queue saturation, dropped/admission-refused requests, and audit burst
  behavior;
- panel review treats budget increases as design questions, not automatic
  blockers.

Exit criteria:

- every growth wave also pays down or deletes something;
- future blocking I/O and unsafe code require explicit architectural review
  and machine-checkable allowlisting;
- efficiency remains part of architecture review, not a one-time cleanup.

### Wave 15 — Fast async status, exec, and USB operations

Goal: make the visible control-plane operations fast enough for interactive UI
clients while preserving broker-only host mutation and daemon-only supervision.

Tasks:

1. Add a daemon-maintained public status read model for unfiltered list/status
   requests. The model carries generation, source-fingerprint, freshness, and
   deep-diagnostic availability metadata so `d2b-wlcontrol` and other UI
   clients can render from one fast model instead of recomputing expensive
   probes on every refresh.
2. Keep status/read-model paths free of deep USB/guest-control diagnostics and
   sysfs driver mutation. Deep diagnostics remain explicit and bounded.
3. Invalidate the model on artifact generation changes, host switch evidence,
   mutating public operations, and runner pidfd registration/deregistration so
   cached lifecycle state cannot survive VM start/stop or switch boundaries.
4. Bound USBIP driver bind and unbind behind the broker's isolated driver helper.
   Research found no Rust crate with true async USB sysfs bind/unbind semantics,
   and Tokio file APIs are blocking-pool wrappers, so the fallback is explicitly
   broker-only and excluded from status/read-model paths.
5. Keep exec and USB latency work tied to concrete caps, deadlines, cancellation
   behavior, and operator-visible remediation rather than hiding slow operations
   behind generic blocking pools.

Validation:

- status/list tests prove read-model metadata is published, repeated unfiltered
  status uses the model, and pidfd-table generation changes invalidate it;
- USBIP broker tests prove both bind and unbind use the bounded driver helper
  and preserve bounded stderr/timeout behavior;
- local Rust checks cover `d2bd` and the privileged broker helper paths;
- implementation review verifies the real `/etc/nixos` host-switch path and
  latency budgets before PR merge.

Exit criteria:

- `d2b status --json` and `d2b list --json` have a fast daemon-side model
  path for UI refreshes;
- USBIP driver mutation cannot indefinitely pin the broker control path;
- status/read-model clients never trigger sysfs bind/unbind or deep guest USB
  probes by default;
- the service remains working after the wave, with a full panel implementation
  signoff and PR merge.

## Highest-leverage deletion and consolidation targets

These targets are good first compatibility-removal and consolidation inputs:

| Target | Desired outcome |
| --- | --- |
| Duplicate JSON emitter boilerplate | One helper-backed artifact emitter pattern. |
| Repeated full `cfg.vms` / `cfg.envs` scans | One normalized Nix index consumed by host/network/process modules. |
| CLI/daemon public DTOs in hub files | Shared contract boundary plus presentation adapters. |
| `d2bd`, `d2b`, and broker hub files | Focused modules with narrow APIs. |
| One-off argv/readiness/audit assembly | Typed runner/process builder shared by Nix and Rust contracts. |
| Activation/tmpfiles/broker ownership overlap | ADR 0034 storage/sync ownership rows with one repair owner. |
| Blocking I/O in daemon/provider hot paths | Declared task model with bounded blocking pools, workers, actors, cancellation, and backpressure. |
| Avoidable project-local unsafe code | Safe crate wrappers where available; remaining FFI quarantined with safety contracts and removal conditions. |
| Duplicate shell lint entrypoints | One lint owner; update callers and delete retired aliases. |
| Monolithic drift gate reporting | Named generated-artifact families. |
| Shell policy checks that parse source | Rust contract/policy tests. |
| Provider dependencies on local-only path | Feature-gated/provider-isolated compile graph. |
| Ambiguous crate/module/type names | Concern-first naming taxonomy that separates host, guest, gateway, provider, constellation, daemon, broker, and CLI ownership. |
| Duplicated local runtime status paths | One provider-neutral runtime/service status, capability, denial, and stopped-state contract. |
| qemu-media enrollment compatibility | Start-time boot media resolution plus runtime QMP hotplug behind a qemu-media adapter. |
| Separate exec and shell terminal machinery | One terminal-v1 substrate with distinct exec and shell public adapters. |
| Excluded helper workspace drift | Explicit per-workspace justification and gate wiring for unsafe/static/supply-chain boundaries. |
| Example/template historical comments | Current-model examples with history moved to ADRs. |

## Anti-goals

- Do not weaken security boundaries to reduce code. The broker/daemon split,
  typed ops, pidfd handoff, minijail profiles, and no-bash invariant are
  not optimization targets.
- Do not collapse generated contracts into untyped runtime maps.
- Do not replace many small explicit broker ops with one unstructured
  "run shell" or "apply path" escape hatch.
- Do not introduce a second v2 architecture beside ADR 0032.
- Do not introduce a second local runtime architecture beside ADR 0037. QEMU
  media and Cloud Hypervisor/crosvm can have different adapters and
  capabilities, but lifecycle/status/denial/reap vocabulary is shared.
- Do not turn qemu-media into a general NixOS workload runtime by silently
  adding guest-control, store sync, SSH, or observability assumptions it does
  not support.
- Do not expose shpool's CLI, config language, templates, or protocol as
  d2b's public shell contract.
- Do not reintroduce SSH, tmux/screen management, or broker shell execution to
  implement persistent shells.
- Do not add new shell gates for efficiency measurement; use existing
  Layer-1 test types or repository-local generated reports.
- Do not use line count alone as a success metric. Deleting useful
  invariants is failure even when the diff is smaller.
- Do not optimize CI by skipping families of validation. Efficiency comes
  from better factoring and narrower invalidation, not fail-open coverage.
- Do not delete explicit migration/versioning infrastructure just because
  stale compatibility shims are being removed. Future incompatible changes
  still need deliberate migration paths.
- Do not move blocking I/O from one hot path to another. Blocking work must
  become explicit, bounded, cancellable, and observable.
- Do not hide unsafe code inside helper crates or generated bindings without
  an audited safe API and documented safety contract.

## Consequences

Positive:

- New contributors can find the owner of a behavior by type: contract,
  model, adapter, side-effect executor, or presentation.
- The v2 provider architecture gets simpler because providers plug into
  existing lifecycle/storage/display/transport contracts instead of
  inventing parallel paths.
- The local runtime architecture gets simpler because qemu-media, Cloud
  Hypervisor/crosvm, and sidecars share status/capability/denial semantics
  while keeping backend mechanisms private.
- Interactive exec and persistent shell share terminal streaming mechanics
  without weakening exec's connection-owned command contract or shell's
  persistent-session contract.
- Local-only users pay less compile and runtime cost for cloud/provider
  experiments.
- Generated-artifact failures become narrower and cheaper to fix.
- The daemon and CLI crates become easier to review because public DTOs and
  presentation/dispatch code no longer live in the same hub files.
- Deletion becomes a normal implementation task with explicit owners.
- Request/task threads stop being the default place for filesystem walks,
  subprocess waits, provider HTTP calls, metrics scraping, and other blocking
  I/O.
- Unsafe-code review becomes a bounded inventory rather than a search through
  unrelated crates.

Negative:

- Several waves initially move code without changing behavior, which can
  create source-line churn in generated docs.
- Contract extraction must update downstream code synchronously, which can
  make those patches larger than a compatibility re-export approach.
- A normalized Nix index is itself a new abstraction; if it becomes a dumping
  ground, it will centralize complexity instead of reducing it.
- Splitting drift families can make the gate graph more explicit but also
  requires careful CI wiring to avoid accidental coverage gaps.
- Compile-graph feature gating can make local development more sensitive to
  missing feature flags if not documented well.
- Excluded helper workspaces prevent unsafe/static-link risk from leaking into
  the main workspace, but they also add validation surfaces that must be kept
  wired explicitly.
- Provider-neutral runtime status can hide necessary backend details if the
  adapter-private/public-contract boundary is not reviewed carefully.
- Moving blocking work behind actors or bounded pools can reveal backpressure
  and timeout choices that were previously implicit.
- Removing unsafe wrappers may require dependency updates or waiting for safe
  crate APIs for kernel features that are still only exposed through raw
  syscalls.

## Review and validation policy

Each implementation wave must include:

1. a deletion/consolidation list;
2. validation evidence for the surfaces it touched;
3. generated artifact regeneration when source locations or DTO homes move;
4. a panel review before the next wave begins when the wave changes
   architecture or behavior;
5. no new compatibility surface; update callers and delete old paths in the
   same wave instead.
6. explicit handling for any ADR accepted since this roadmap whose components
   add runtime kinds, helpers, workspaces, public commands, generated
   contracts, or unsafe/task-model boundaries.

Panel reviewers should treat this ADR as a ratchet: a wave that only adds a
new abstraction without deleting duplication has not satisfied the roadmap,
even if tests pass.
