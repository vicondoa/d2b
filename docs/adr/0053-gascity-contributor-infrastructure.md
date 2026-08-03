# ADR 0053: Gas City as contributor infrastructure, not a d2b capability

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0015](0015-daemon-only-clean-break.md) (daemon-only clean
  break, and its prohibition on host-singleton framework services),
  [ADR 0035](0035-efficiency-and-simplification-roadmap.md) (efficiency and
  simplification roadmap), [ADR 0046](0046-d2b-3-provider-control-plane.md)
  (d2b 3.0 provider control plane), whose validation and delivery contract is
  specified in
  [`docs/specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md);
  sections 12.2, 12.3, 12.5 and 12.6 cited throughout this record are sections
  of that specification, not of ADR 0046, which carries no numbered section
  12.3. Also [ADR 0048](0048-copilot-native-agent-surface.md) (Copilot-native
  agent surface). This record changes none of them.
- Scope: contributor workflow ownership and external configuration only.
  Documentation only; it authorizes no code and no change to any d2b product
  surface.
- Unblocks: the acceptance program below and the follow-on specification that
  consumes it.

## Context

This repository already runs a heavyweight engineering process: Spec Kit for
requirements and planning, thirteen committed Copilot agents, five d2b skills,
a ten-seat panel, and an attest/seal/merge-eligibility gate in
`packages/xtask/src/delivery/`. What it does not have is **durable
orchestration**. A run lives inside one agent session. A context handoff, a
crashed harness, or a closed terminal loses the position, and
`/d2b-autopilot --resume` recovers from a checkpoint it wrote itself rather
than from a workflow engine that owns the state.

The decision here is to put a durable orchestrator underneath the process that
already exists: Gas City drives Spec Kit planning, Superpowers-style per-task
discipline, simplification, this repository's own ten-seat panel, and a final
publication gate, with Discord as the human surface and a dashboard for
observation.

### What category this belongs to

**Gas City is contributor infrastructure. It is not a d2b feature.**

d2b is an opinionated NixOS desktop microVM framework. Its product surface is
the flake outputs, the NixOS option schema, the versioned manifest and bundle
contract, the daemon and broker wire protocols, the CLI, and the Diataxis
documentation tree that describes all of it to a consumer. Gas City is none of
those things. It is tooling that some contributors use to work **on** this
repository, in the same category as `.github/agents/`, `scripts/copilot/`, the
`Makefile`, and `tests/tools/`: development infrastructure whose entire
audience is people changing the framework.

A d2b consumer configuring VMs in their NixOS host config should never
encounter Gas City, never need to know it exists, and never find it in an
option, a schema, a manifest field, a CLI verb, or a released changelog entry.
That is not a soft preference; D1 makes it a set of checkable exclusions,
because the failure mode for a piece of development tooling is that it
gradually acquires a product surface nobody decided to ship.

The same reasoning applies in the other direction. Contributor tooling should
not consume the framework it is used to develop, which D2 makes explicit.
Earlier drafts of this record designed a d2b-backed microVM sandbox for agent
execution; that design is withdrawn in full and is out of scope here.

The operator's design brief for this system is `d2b_gascity_design.txt`,
supplied as desired architecture rather than as verified syntax. Everything
below was measured instead.

### Naming

The upstream project's own preferred name is **Gas City**, two words, title
case: `README.md` renders `<h1>Gas City</h1>` and the CHANGELOG uses the same
form. The GitHub organisation, repository, and Go module path all spell it
closed and lowercase (`gastownhall/gascity`, `github.com/gastownhall/gascity`),
and the CLI binary is `gc`. Its predecessor was **Gas Town**, and `README.md`
still routes former users through
`docs/getting-started/coming-from-gastown.md`. This record uses "Gas City" in
prose and the closed lowercase form only where it names a repository, module,
binary, or config key. The operator's spelling "gascity" is the repository
alias, not the project name, and the intended configuration repository
`vicondoa/d2b-gascity-configs` keeps that alias deliberately.

### What was measured, and where

| Repository | Branch | Commit | Date |
| --- | --- | --- | --- |
| `gastownhall/gascity` | `main` | `38ed358fae0e8238834eb778a23a664fe4cb8954` | 2026-08-02 |
| `gastownhall/gascity-packs` | `main` | `0b9574272814ba175950731b73c2cc201804ee61` | 2026-08-01 |
| `gastownhall/gascity-dashboard` | `main` | `fdd2d636751963ca786d06c9e43369fc6a71f7e2` | 2026-07-17 |
| `github/spec-kit` | `main` | `d1e86f638277a99b82715c22c90558cd58d3cffd` | 2026-07-31 |
| `numtide/llm-agents.nix` | `main` | `7b99fc4bbb8a7c2fff82c2708d8636c1cbc65661` | 2026-08-03 |
| `vicondoa/d2b` | `v3` | `5ccb12a2edaac85b4735ee95d697067bd3d339af` | 2026-08-02 |

Gas City's latest tagged release at that commit is `1.4.0` (2026-07-24,
`CHANGELOG.md`); the tree carries an `[Unreleased]` section on top of it.

### The `gc` binary comes from a packaging flake, not from us

`numtide/llm-agents.nix` describes itself as "Nix packages for AI coding agents
and development tools. Automatically updated daily." Its
`packages/gascity/package.nix` builds `pname = "gascity"`, `version = "1.4.0"`,
from `fetchFromGitHub` of `gastownhall/gascity` at tag `v1.4.0`, building
`subPackages = [ "cmd/gc" ]`. The package update commit is
`5cd614eafd8cfcdbba8cafa6a37ef51633dd0f86`.

Two details matter for the deployment. It `wrapProgram`s `gc` with a PATH
prefix carrying `beads`, `dolt`, `flock`, `gitMinimal`, `jq`, `lsof`, `procps`
and `tmux`, which is exactly the prerequisite set Gas City's own README lists as
`Required: Always`, plus the beads provider tools; it also asserts
`lib.versionAtLeast dolt.version "2.1.0"`, matching the minimum that README
states. So the runtime dependency closure is the packaging flake's problem and
is already solved there. And `doCheck = false` with `versionCheckHook` enabled,
so the build does not run upstream's test suite but does verify the installed
binary reports its version.

What it does **not** provide is deployment wiring. Its flake outputs are
per-system `packages` and `overlays.shared-nixpkgs`; there is no `nixosModules`
output and no service, user, credential, or network configuration of any kind.
That split is the reason D4 assigns the package to one repository and the module
to another rather than looking for a single upstream that does both.

### Where d2b's product surface actually is

The exclusions in D1 are checkable because this repository already draws the
line clearly. `flake.nix` exposes `nixosModules.default`, `templates.default`,
`checks`, `packages`, and `devShells`. `AGENTS.md` describes `docs/` as the
"Diataxis tree (explanation / how-to / reference)" and `README.md` as the
"consumer-facing entry point", while `docs/contributing/` carries process. The
manifest and bundle contracts are version-pinned by `manifestVersion` and
`bundleVersion` and indexed in `docs/contributing/critical-subsystems.md`. And
`docs/contributing/changelog-and-commits.md` requires that "each released
section reads as a coherent, consumer-facing summary of what changed", barring
internal process detail from every CHANGELOG section including `[Unreleased]`.

So "not a product surface" has a precise meaning here: absent from the flake
outputs, absent from the option schema, absent from the versioned contracts,
absent from the Diataxis tree, absent from the critical-subsystems index, and
absent from released changelog prose.

### The supported execution model is local, and it is what upstream tests

`cmd/gc/runtime_registry.go:118-119` registers `tmux` and then calls
`r.SetFallback(tmuxFactory)`, and the README prerequisites table lists tmux as
`Required: Always`, adding that "tmux is the default session backend **and**
the fallback, so it stays required even if you run agents on another backend".
Builtin runtimes alongside it are `subprocess`, `acp`, `t3bridge`, `k8s`,
`herdr`, `hybrid`, and the `exec:` and `ssh:` prefixes.

Isolation in the shipped packs is a **git worktree on the host**.
`gascity/assets/workflows/do-work/prepare-worktree.md:22` runs
`git worktree add "$WORKTREE" --detach HEAD`, and `gastown`'s
`mol-polecat-work` and `pr-review`'s `mol-adopt-pr` do the same per bead and
per pull request. No shipped pack uses container or VM isolation.

That is the whole execution story: a supported runtime and a supported
isolation model, both exercised by upstream's own tests, and neither needing
anything from d2b.

### Gas City runs from configuration, and reads nothing from the target repo

Gas City reads no configuration from inside a rig's own repository. The rig is
bound by path from the city side: `internal/config/site_binding.go` resolves
bindings from `.gc/site.toml` and errors with "rig %q is declared in city.toml
but has no path binding in .gc/site.toml; run `gc rig add <dir> --name %s` to
bind it". `.gc/` is **city** state throughout (`.gc/worktrees/`,
`.gc/events.jsonl`, `.gc/site.toml`, and `$HOME/.gc/events.jsonl` for the
dashboard audit log).

Two consequences follow. The city's configuration can live entirely outside
this repository, which is what D4 does. And the brief's proposal to put
`.gc/approvals/`, `.gc/reviews/` and `.gc/verification/` inside the d2b
checkout collides with a name upstream owns, and puts evidence in Git that spec
section 12.5 forbids.

Packs are pinned by `packs.lock`, whose schema records `version`, `commit` and
`fetched` per pack, and imports pin a commit with
`Import.Version = "sha:<hex>"`.

### The panel cannot be delegated, and this is enforced in code

`packages/xtask/src/delivery/model.rs` pins `PANEL_PROVIDER_POLICY =
"github-copilot"`, `PANEL_MODEL_POLICY = "gemini-3.1-pro-preview"`,
`PANEL_REASONING_EFFORT_POLICY = "high"`, and a ten-element `PANEL_ROLES`.
`panel.rs` calls `ensure_panel_binding` on both the request and every record,
requires exactly one record per role, requires `signoff` true iff
`recommendations` is empty, and binds every record to the same `candidate_id`,
`content_id` and `snapshot_sha256`. There is no override and no partial pass.
`packages/xtask/src/delivery/storage.rs` refuses a delivery state root that
lives inside any enclosing Git working tree, because spec section 12.5 says
validation output, panel transcripts and attestation payloads never enter Git.

Upstream's nearest equivalent, `compound-code-review` in the
`compound-engineering` pack, has 17 review lanes of which 7 are always on and
10 sit behind selector gates; it synthesises them into one verdict checked by
`implementation-review-approved.sh` and loops up to `max_attempts = 8`. It
records no per-lane model or effort, has no unanimity concept, and has no
independent seat identity to attest. It therefore cannot emit a record
`panel-attest` would accept. That is a structural fact, not a configuration
gap.

### Approvals upstream are not artifact-bound

Gas City models a durable wait as a bead of type `gate`
(`internal/session/waits.go`) whose states are `pending`, `ready`, `closed`,
`canceled`, `expired`, `failed`. Those are lifecycle states, not decisions. The
formula-level `Gate` struct (`internal/formula/types.go`) carries `Type`
(`gh:run`, `gh:pr`, `timer`, `human`, `mail`), `ID` and `Timeout`. Nothing in
either binds a decision to an artifact path or content hash, and there is no
`on_reject` concept anywhere in the engine. Upstream human gates in packs are
ordinary steps a person closes with `gc bd close`; there is no first-class
ask-human node.

The dashboard is weaker still. Its Approve and Deny buttons POST
`/v0/city/{cityName}/session/{id}/respond` with `SessionRespondInputBody {
action: string, metadata }` against a `PendingInteraction { kind, metadata,
options, prompt, request_id }`. There is no path, no hash and no revision in
either type: this is a real-time "allow this tool call" prompt, not an artifact
review. Its own README calls the repository a temporary workspace for the next
dashboard, it has no authentication or authorization model beyond binding to
`127.0.0.1` and a `DASHBOARD_READONLY=1` switch, and the respond path is not
written to its audit log.

Spec Kit, by contrast, does now have a gate engine, and its recent history is
instructive: commit `d1e86f6` ("fix(workflows): fail a gate whose on_reject is
not abort/skip/retry") closed a bug where an `on_reject` value outside
`{"abort", "skip", "retry"}`, including `"Abort"` with a capital A, made a
**rejected** gate report completed and walk past the review. That is the exact
failure class this repository's fail-closed rule exists to prevent, observed in
the upstream we would depend on.

### Spec Kit artifacts are a prose convention, not a schema

Commands are namespaced `speckit.*`: `specify`, `clarify`, `plan`, `tasks`,
`implement`, `analyze`, `checklist`, `constitution`, `converge`,
`taskstoissues`. A `tasks.md` line looks like:

```
- [ ] T014 [US1] Implement [Service] in src/services/[service].py (depends on T012, T013)
```

Task IDs are stable `T` plus three digits and `## Phase N` headers are
reliable, but dependencies are parenthetical prose, affected files are unmarked
prose inside the description, there is no structured dependency field, there is
no JSON schema or parser, and there is no format version anywhere in the
artifact. The template is a prompt to a model, so output varies per invocation.

The handoff itself is clean: `templates/commands/tasks.md` declares
`handoffs: [{label: "Implement Project", agent: speckit.implement, ...}]`, so
substituting a different executor after `tasks` is a supported shape rather
than a hack. This repository already installs spec-kit in skills mode per
ADR 0048, so the `.github/skills/speckit-*` surface is the one in use.

### Names the brief assumes that do not exist

`speckit-adapter`, `runtime-d2b`, `d2b-verified-build`, and a `simplify` pack
are all absent upstream. `superpowers`, `compound-engineering`, `pr-pipeline`,
`pr-review`, `discord`, and `github` are real. Simplification exists as stage 1
of `mol-pr-ship` in `pr-pipeline` and as a `simplicity-review` lane in
`build-basic-review`, not as a pack, and not per task. `mol-pr-ship` ends at a
readiness report and deliberately does not push or open a pull request. The
`github` pack does ship `create-pr` and `push-branch` commands.

`superpowers/formulas/superpowers-development.formula.toml` has steps
`implement`, `write-failing-test`, `verify-test-fails`, `implement-change`,
`verify-test-passes`, `task-review`, `record-item-result`,
`close-source-anchor`. The red/green core and the per-task review are therefore
upstream; a per-task simplification step is not.

## Decision

**D1. Gas City is contributor infrastructure and acquires no d2b product
surface.** It is repository development tooling for agent-assisted work on
d2b, in the same category as `.github/agents/`, `scripts/copilot/`, the
`Makefile`, and `tests/tools/`. It is not a d2b feature, capability, public
contract, Provider, Runtime, Service, or consumer-facing surface, and the
following are the checkable form of that:

- **No flake output.** Gas City appears in no `nixosModules`, `templates`,
  `checks`, `packages`, or `devShells` output of `flake.nix`.
- **No option.** No `d2b.*` NixOS option is added, renamed, or documented for
  it, and `nixos-modules/` gains nothing.
- **No versioned contract.** No manifest field, bundle artifact, schema, wire
  message, broker op, or CLI verb, and therefore no `manifestVersion` or
  `bundleVersion` implication.
- **No consumer documentation.** Nothing under `docs/reference/`,
  `docs/how-to/`, or `docs/explanation/`, and no mention in `README.md`.
  Contributor documentation belongs in `docs/contributing/`, which is where
  the follow-on specification points.
- **Not a critical subsystem.** No row in
  `docs/contributing/critical-subsystems.md`, because it constrains no
  framework invariant.
- **No capability framing anywhere, including the changelog.** Gas City is
  never presented as something d2b gained, in any artifact. Mentions are
  permitted in contributor surfaces (`docs/adr/`, `docs/contributing/`,
  `specs/`, `.d2b-orchestration.toml`, `changelog.d/`, and contributor-process
  prose in `CHANGELOG.md`, including this record's own fragment); what is
  forbidden is framing, not spelling. An entry saying the repository adopted a
  contributor workflow is correct; an entry under a released `Added` or
  `Changed` heading describing Gas City as a d2b feature is not. M1 scans the
  product surfaces mechanically and states plainly which half of this rule a
  reviewer has to judge.

A d2b consumer must be able to adopt, configure, and run the framework without
learning that Gas City exists.

**D2. No d2b component runs, hosts, or isolates any part of this system, and
none is in scope.** Contributor tooling does not consume the framework it is
used to develop.

Gas City, its agents, its sessions, and its packs must not use `d2bd`, the
privileged broker, d2b microVMs, any d2b Provider, Runtime, Zone, Resource or
Service component, the guest-control transport, the Credential service, or any
other d2b-supplied mechanism. The environment is host software operating on a
git checkout of this repository, and that is the entirety of its relationship
to d2b.

A d2b-backed sandbox, a d2b runtime provider, a Gas City runtime pack targeting
d2b, guest artifact return, and any credential path through d2b are **out of
scope for this record**. They are not deferred sub-parts of this decision and
they have no acceptance criteria here. If they are ever wanted, they require
their own ADR, which will have to reckon with facts this record deliberately
does not build on: `VmCommand` in `packages/d2b/src/lib.rs` has no `create` or
`destroy` verb and operates only on VMs declared in `d2b.vms.<name>`, and
`GuestCapability` in `packages/d2b-contracts/src/guest_wire.rs` is a closed
enum whose only file operation is `ReadGuestFile` over a closed set containing
exactly `GuestConfig`. No architecture for crossing those gaps is proposed,
sketched, or reserved here.

D1 and D2 together also keep ADR 0015 unengaged. Whatever host service
supervises `gc` is operator host configuration, not a d2b framework unit; d2b
ships nothing for it, declares nothing for it, and gains no fourth root-visible
unit.

**D3. Gas City is opt-in, and the standalone contributor surface stays.** The
committed Copilot surface remains authoritative and unchanged: thirteen agents
under `.github/agents/`, the d2b skills `/d2b-adr`, `/d2b-panel-round`,
`/d2b-wave-delivery`, `/d2b-memory`, `/d2b-autopilot`, and the `speckit-*`
skills. They remain fully usable, and fully supported, for contributors who
never touch Gas City. No Gas City work may delete, rename, gate, or condition
any of them, and none of them may acquire a Gas City dependency. Using Gas City
is a contributor's choice about their own workflow, not a repository
requirement.

**D4. Four layers of ownership, and d2b owns almost none of it.** Package
source, generic deployment machinery, d2b-specific policy, and machine-private
instantiation are four different concerns with four different change rates and
four different audiences, so they get four homes.

- **`numtide/llm-agents.nix`** supplies the `gc` package and nothing else. It
  is upstream, out of our control, and listed here because the alternative is
  somebody building Gas City by hand. Pinned by layer 2 or layer 3, never by
  d2b.
- **`vicondoa/gascity.nix`** owns the generic, reusable NixOS module and its
  helper packages: the systemd units and their ordering, the orchestrator,
  worker, approval-controller, publisher and root-owned append-helper
  principals of D14, state and lock paths, the graceful shutdown and
  restart-adoption sequence of D5, dashboard read-only and loopback
  enforcement, the worker network namespace and firewall primitives, the
  append and audit helper with its rotation and retention, health checks, and
  module tests.

  It contains **nothing d2b-specific at all**: no d2b name or identifier, no
  d2b path or default, no option that mentions d2b, no repository or rig
  binding, no workflow formula, no panel or delivery concept, no Discord
  identity value, no d2b egress allowlist entry, and no task-import logic.
  Every one of those lives one layer up in `d2b-gascity-configs`. The rule is
  not "avoid coupling where convenient"; it is that a reader of `gascity.nix`
  should not be able to tell which project motivated it, and M21 checks that
  mechanically rather than by review.

  Its public option namespace is Gas City-specific and rooted at
  `services.gascity`, with every declared option beneath it. A name like
  `services.d2b-gascity` or an option carrying a d2b default would leak the
  first consumer into the interface every later consumer has to use, which is
  the same mistake as a d2b path in a unit file and is harder to undo once
  configurations depend on it.

  Its package option is wired from, or defaults to, the locked
  `llm-agents.nix` `gascity` package rather than repackaging Gas City.
- **`vicondoa/d2b-gascity-configs`** owns the d2b-specific instance and policy
  layer. It pins and imports `gascity.nix`, following or aligning that flake's
  `llm-agents.nix` pin rather than introducing a competing one, and it owns
  `city.toml`, `packs.lock` and pack pins, the d2b workflow formulas, the
  repository and rig binding, the single Discord user, guild and channel
  values, the d2b-specific egress allowlist, the artifact approval policy and
  schema, the `tasks.md` importer, and secret references by name.
- **The host's `/etc/nixos`** owns machine-private instantiation: secret
  values, local paths, uid and gid assignment, and the import of the d2b
  configuration module. It is operator-private and outside every repository
  named here.

**`vicondoa/d2b`** remains this record, the follow-on contributor
documentation, the Spec Kit artifacts under `specs/`, and the repo-local
manifest of D7. Nothing else. No Gas City concern lands in `packages/` or
`nixos-modules/`, and neither `llm-agents.nix` nor `gascity.nix` is ever a d2b
flake input.

**This record authorizes creating `vicondoa/gascity.nix` and
`vicondoa/d2b-gascity-configs` later; it does not create them now.** Neither
exists at the time of writing, and nothing here is blocked on their existing.
Research at the measured commits found no reusable Gas City NixOS module to
adopt instead: the upstream organisation ships Homebrew taps and no Nix
deployment repository, and `llm-agents.nix` exposes per-system `packages` and
`overlays.shared-nixpkgs` with no `nixosModules` output at all.

Secrets are referenced, never inlined, in either shared repository, and their
values exist only where `/etc/nixos` places them.

**D5. Lifecycle is generic, ordered, and owned by the module layer.** The
machinery this record specifies is mostly not about d2b. Five principals, a
network namespace, an append-only store with an exclusive lock, a read-only
dashboard, rotation and retention, and the sequence below are all things any
Gas City deployment that takes agent isolation seriously would need. That is
the argument for layer 2 existing at all: this is a body of generic host
behaviour large enough to be worth testing once, in a module with its own NixOS
VM tests, rather than re-deriving per consumer inside a policy repository.

**Five units, one ordering chain, and shutdown is its exact reverse.** The
generic module ships five ordered units, and the reverse property is obtained
from systemd rather than hand-written: units related by `After=` are stopped in
the reverse of the order they were started, so specifying startup order once
specifies shutdown order too. Writing two independent sequences and hoping they
mirror is how they stop mirroring.

| Order | Unit | `After=` | `Requires=` / `BindsTo=` | Stop-time coupling |
| --- | --- | --- | --- | --- |
| 1 | `gascity-append-helper.service` | nothing | nothing | last to stop |
| 2 | `gascity-worker-netns.service` | append helper | `Requires=` append helper | stops fourth |
| 3 | `gascity-dashboard.service` | worker netns | nothing | stops third |
| 4 | `gascity-orchestrator.service` | dashboard **and** worker netns | `Requires=` worker netns only | stops second |
| 5 | `gascity-discord-ingress.service` | orchestrator | `BindsTo=` orchestrator | first to stop |

The dependency edges carry meaning beyond ordering, and the kinds are not
interchangeable:

- `gascity-worker-netns.service` is `After=` and `Requires=` the append helper,
  because a namespace brought up while the audit store has no owner would let
  agent activity begin unrecorded.
- `gascity-orchestrator.service` is
  `After=gascity-dashboard.service gascity-worker-netns.service` and
  `Requires=gascity-worker-netns.service`. The two edges do different jobs and
  both are needed. `Requires=` on the netns is the safety edge: the orchestrator
  spawns agents and must not be able to spawn one outside its confinement.
  `After=` on the dashboard is the **ordering** edge, and it is what actually
  makes position 3 and position 4 a chain rather than two siblings.

  Without it, the dashboard and the orchestrator would both merely be `After=`
  the netns, which constrains each against the netns and neither against the
  other; systemd would be free to start them in either order, and because stop
  order is the reverse of start order, it would be equally free to stop the
  dashboard **before** the orchestrator. The dashboard would then go dark
  exactly during the drain it exists to let an operator watch. The edge is
  deliberately ordering-only, with no `Requires=`, so a dashboard that fails to
  start delays the orchestrator's ordering but does not prevent it.
- `gascity-dashboard.service` is `After=` the worker netns and carries **no**
  `Requires=` or `BindsTo=` at all. It is observation only, so it must be able
  to run while the orchestrator is absent, still starting, or failed. That is
  the point of starting it third: an operator watching a troubled startup sees
  it.
- `gascity-discord-ingress.service` is `After=` and **`BindsTo=`** the
  orchestrator. `BindsTo` rather than `Requires` because if the orchestrator
  dies unexpectedly the ingress must stop with it; an ingress accepting
  approvals with no orchestrator behind it is the confused state D12's
  authority rules exist to prevent.

**Startup, in order.**

1. **The append helper acquires and adopts the store.** It takes the
   single-owner lock, reconciles per D12, and becomes ready. Nothing else
   starts until it has, so no component can act before its actions can be
   recorded.
2. **The worker namespace, interface, DNS proxy and firewall become ready.**
   This unit is `Type=oneshot` with `RemainAfterExit=yes` and exits zero only
   after verifying the namespace exists, the interface is up, the resolver
   answers, and the deny-by-default rules are loaded. Ordering alone would only
   prove the unit ran.
3. **The read-only dashboard may start**, in a backend-not-ready observation
   mode. It is expected to come up before the orchestrator and to display that
   the backend is not yet ready rather than failing, because the startup it is
   most useful for observing is the one that is not going well.
4. **The orchestrator acquires and adopts its state and becomes ready.**
   Spawning is impossible before namespace readiness is **proven**, not merely
   ordered: beyond `Requires=`, the orchestrator re-checks namespace readiness
   immediately before each spawn and refuses if it is absent. Unit ordering
   establishes the initial state and says nothing about a namespace unit
   restarted underneath a running orchestrator, which is exactly when an
   unconfined agent would otherwise appear.
5. **Discord ingress starts last.** Human authority enters only once everything
   that would act on it has adopted its state and is ready.

**Shutdown, which is the same chain reversed.**

1. **Ingress refuses and stops first.** While the rest proceeds it **actively
   refuses** arriving interactions, so an approval submitted during shutdown is
   rejected rather than dropped, queued, or applied against a draining
   workflow.
2. **The orchestrator parks or drains and terminates every agent session**, the
   namespace still standing throughout. In-flight nodes reach a durable
   boundary: a task at a safe point finishes, anything else parks on its gate
   so it resumes rather than restarts. Sessions are asked to stop, given a
   bounded grace period from configuration, then terminated, because an agent
   that will not exit must not prevent the host from shutting down.
3. **The dashboard stops** once orchestrator state has settled, having stayed
   up through the drain so an operator can watch it.
4. **The namespace, firewall and DNS proxy tear down**, only after every agent
   and the orchestrator itself have exited. Per D15 the confinement's lifetime
   strictly contains every process it confines; an agent outliving the teardown
   of its own confinement is an unconfined agent, and the race is removed by
   ordering rather than narrowed by timing.
5. **The append helper flushes, closes and releases its locks last.** Every
   in-flight append completes or is abandoned with its temporary unlinked per
   D12, nothing is left acknowledged but unsynced, and the single-owner lock is
   released only after every writer above has stopped, so the lock's lifetime
   strictly contains every operation it guards.

**Adoption always precedes cleanup.** Each stateful unit acquires ownership,
reads existing state, reconciles, and only then resumes work. It never cleans
before adopting, because a sweep by an instance that does not own the store can
delete the in-flight state of one that does. D12 states that rule for the
append helper's temporaries; D5 makes it the general shape for every stateful
principal here.

D2 still holds throughout: none of this uses `d2bd`, the broker, a microVM, or
any other d2b component. It is systemd, Unix identities, and namespaces.

**D6. One local, single-user deployment, and what that scope does not
simplify.** The deployment is one operator on their own machine, and
`/etc/nixos` instantiates exactly one of it. That scope is used deliberately to
remove work, and just as deliberately not used to remove the boundary that
matters.

What it removes:

- **Discord ingress is allowlisted to one identity.** Exactly one configured
  Discord user id is accepted, and where the deployment uses a guild rather
  than a direct message, exactly one configured guild and channel. Every other
  author, guild, or channel is **rejected at ingress**, before the message can
  produce an approval, a response, a clarification answer, a gate transition,
  or any other change to workflow state. The allowlist is fail-closed: an
  unset or empty allowlist rejects everything, including the operator, rather
  than defaulting open. Signature verification authenticates the platform, not
  the person, so it is a precondition of the allowlist rather than a substitute
  for it.

  **A rejection is diagnosed locally, never silent.** Silent unresponsiveness
  is indistinguishable from a broken token, a dead service, or a bot that was
  never invited, and the operator would have no way to tell which. Every
  rejection emits one local, bounded, redacted diagnostic naming the rejection
  class from a closed set (`unconfigured-allowlist`, `identity-mismatch`,
  `guild-mismatch`, `channel-mismatch`, `signature-invalid`) together with the
  received and configured identity digests of D12, never the raw ids. Digests
  are what makes it actionable: the operator can compare the received digest
  against the configured one and see whether they are dealing with the wrong
  account or an empty configuration, without the diagnostic itself becoming a
  place where platform identifiers accumulate in plaintext.
- **The approval reviewer identity is that one operator.** D12 records it for
  provenance and for binding a decision to a human, not to choose among
  reviewers or to evaluate a permission.
- **The dashboard binds loopback only** and gains no authentication layer,
  because there is no second *human* on the machine to authenticate. That is
  the limit of what this scope buys: loopback excludes remote humans and
  admits every local process, so D13 additionally requires read-only mode.
- **Nothing multi-party is designed.** No role model, no permissions matrix, no
  tenancy, no shared or hosted instance, no remote access path, no high
  availability, no failover, and no federation across cities.

What it does not remove: **local agent processes remain untrusted principals.**
"Single-user" is a statement about humans. The deployment still runs autonomous
agents, each of which is a local principal that can open loopback sockets, read
what its identity can read, and write what its identity can write. Every
control in D13 and D14 exists for those principals and none of them is relaxed
by the fact that one person is at the keyboard.

Concretely, the **worker and publisher separation, and the worker's inability
to write approvals, both stand unchanged** under D14. Those boundaries are
between the operator and an agent process, not between one human and several.
Collapsing them because "it is only me" would delete the design's only
mechanical controls while leaving every reason for them intact, and D14 forbids
it.

The same distinction applies to the Discord allowlist. It answers "which human
may speak to this deployment", which is a small question here because the
answer is one person. It does not answer "what may an agent do", which is the
question D14 answers, and the two must not be conflated because both happen to
be touched by the word "single-user".

**D7. One tiny repo-local manifest, with a d2b-owned schema.** Gas City reads
nothing from the rig repository, so this file is read by the configuration
repository's tooling only and must not imitate upstream syntax. It is
`.d2b-orchestration.toml` at the repository root and carries exactly `schema`,
`spec_root`, `gascity_compat` (a version range checked against what the
deployed binary reports through `gc version`, per D18), and `panel_authority`
(whose only legal value is `xtask-delivery`). It is contributor metadata, not a
configuration surface: it exists so that a rename of `specs/` or a
compatibility break is caught in the same commit that causes it. `.gc/` is
forbidden inside this repository; Gas City owns that name for city state.

**D8. Execution is upstream's supported local model.** Sessions run on the
`tmux` runtime, which is both the default and the registered fallback, and each
task works in a git worktree created from the checkout and removed on
completion. Both are what upstream ships and tests. No alternative backend is
configured in the first deployment.

**D9. Gas City owns durable execution; Spec Kit owns planning; neither owns
review.** For an opted-in run, Gas City owns workflow state, the task DAG,
dependency execution, retries, waits, routing, notification, runtime desire,
and non-convergence reporting. Spec Kit owns `constitution`, `specify`,
`clarify`, `plan`, `tasks`, `analyze` and their artifacts under `specs/`.
`speckit.implement` is not used for opted-in runs, and neither is
`/d2b-autopilot`: exactly one executor drives a given run, and for an opted-in
run that executor is Gas City.

**D10. Per-task execution is Superpowers-shaped, with an evidence rule that
does not force a fake test.** A task that changes behaviour runs the full
red/green discipline: write the failing test, prove it fails, implement, prove
it passes, local simplification, re-run the focused tests, revert the
simplification alone if they fail, spec-compliance review, code-quality review,
bounded repair. The repair loop is bounded and escalates rather than looping.
Because upstream has no per-task simplification, the revert must be scoped to
the simplifier's own change so a failed simplification never discards the
implementation; M8 proves that.

Not every task has a meaningful unit test. Documentation, comments, pure
configuration data, file moves, and reorganisation are real tasks whose
correctness is not expressible as a failing assertion, and demanding one
produces a fabricated test that proves nothing and then has to be maintained.
For those tasks the discipline is preserved in substance rather than in form:
the task record MUST name a **pre-change failure proof** and a **post-change
validation**, both non-empty and both naming a command or a specific
observation rather than a claim.

| Task shape | Pre-change proof | Post-change validation |
| --- | --- | --- |
| Behaviour change | A failing test, observed failing | The same test, observed passing |
| Documentation or prose | The command or reading that exhibits the defect, e.g. a doc lint, a broken link check, or the exact passage that contradicts committed code | The same check or passage, now correct |
| Configuration or data | The eval, schema check, or diff that shows the wrong value | The same check on the new value |
| Move or rename | The reference scan or build that fails to resolve, or a stated before-state inventory | The same scan resolving, with no orphan |

Inventing a test whose only purpose is to satisfy the red/green shape is
forbidden. A task that can state neither a pre-change proof nor a post-change
validation is not ready to run and returns to the task-DAG gate.

After integration the run performs global simplification, then
post-implementation `speckit.analyze`, then a verification matrix mapping every
requirement to tasks, changed files, commits, tests, review disposition and
final status, and fails when any chain is incomplete.

**D11. Gas City orchestrates the panel stage; the panel itself stays d2b's and
runs unchanged.** The distinction matters and the previous phrasing blurred it.
Gas City may schedule the stage, block the run on it, and consume its outcome.
Gas City may not perform, synthesize, summarise, re-run, short-circuit, or
substitute for the review, and may not produce or edit an attestation record.

The binding review stays this repository's existing one, executed by the
existing d2b panel skill and `packages/xtask/src/delivery/` tooling, with
nothing about it modified: ten independent seats, read-only by construction,
`gemini-3.1-pro-preview` at `high` under provider `github-copilot`, `signoff`
true iff `recommendations` is empty, unanimous ten of ten, delta-scoped after
round one, prompts carrying the integrator's validation evidence, and any
content change invalidating every prior sign-off. Records go to the external
delivery state root, never into Git.

**The bridge between the two is unproven and is prototype work.** ADR 0048
measured that `--agent` is ignored over ACP and that the skills execute as
in-session Task lanes, so a Gas City formula step cannot be assumed to be able
to trigger an in-session slash command, and this record does not assert that it
can. What mechanism causes a panel round to run when Gas City reaches that
stage, and how the outcome returns to the run, is left to the follow-on
specification and gated by M10. Until that is proven, the honest fallback is a
durable gate that parks the run and a contributor who runs the panel by hand,
which is strictly better than a bridge that silently reports a review nobody
performed.

**A parked run must say how to unpark it.** A fallback that leaves the
operator with a permanently blocked workflow and no named next step is not a
fallback. Upstream closes a human gate by closing its bead, measured in the
shipped packs as `gc bd close <gate-bead-id>`, so the parked gate's own message
carries that command with the concrete bead id substituted, and the follow-on
specification re-measures the invocation before it is written down as
operator-facing text.

The resume is itself gated: the close is permitted only when a `panel-attest`
record exists for the wave, bound to the same `candidate_id`, `content_id` and
`snapshot_sha256` the run is at. Closing a parked panel gate with no attested
record is refused, because a manual fallback that can be waved through by
typing a command is a worse gate than no gate, and the whole point of D10 is
that Gas City cannot report a review that did not happen.

Upstream `compound-engineering` may run as an **additional, non-binding**
pre-panel filter whose only output is findings for the integrator to fix before
the panel round. A clean compound review never sets `signoff`, never
substitutes for a seat, and never shortens the roster. `pr-pipeline`'s
`mol-pr-ship` may supply the readiness report; it does not gate the merge. The
standalone panel skill stays usable outside Gas City.

**D12. Approvals are artifact-bound, digest-only, append-only, and fail
closed.** The approval record is owned by the deployment's approval store, not
by a Gas City gate, a Discord message, or the dashboard. Each record carries:
the gate node identifier, the artifact identity, a decision from the closed set
`{approve, revise, rescope, abort}`, a reviewer digest, a run digest, and the
timestamp. For the publication gate the artifact identity is D14's immutable
integration commit hash; for a document gate it is the artifact path plus the
`sha256` of its exact bytes. A Gas City gate is the **transport** that blocks
the run; Discord is the **surface** that collects the decision; neither is the
**authority**.

**Identifiers are stored as digests, never in plaintext.** The Discord user id
and the run handle are recorded as fixed digests, not raw values, because a
durable audit surface that accumulates raw platform identifiers and opaque
correlation handles leaks both to anyone who later reads it. The digest is
**keyed with a deployment-scoped secret** rather than a bare hash: a Discord
snowflake is low-entropy and enumerable, so an unkeyed digest of it is
reversible by anyone who can guess a few million candidates. The key is stable
for the life of the deployment, so the digest remains a usable correlation
handle across records while the raw value stays unrecoverable. Per D6 the
reviewer is always the one configured operator, so the digest identifies rather
than authorises: a decision whose reviewer digest does not equal the configured
operator's is rejected, not recorded.

An approval is honoured only when the recorded artifact identity equals what is
in front of the consumer. On mismatch the run is **denied**, not warned, and
the error names the remediation: re-request approval on the new revision. This
mirrors section 12.6's content-invalidation rule that the panel already
enforces, and it is the direct answer to the upstream gate bug commit `d1e86f6`
fixed.

Human approval is required at the constitution, spec, plan, task-DAG, and
publication gates. Analysis and per-task review are autonomous with escalation.

**History is append-only and nobody may rewrite it.** A mutable approval log
cannot prove that a human authorised anything, because the proof and the thing
it constrains are then editable by the same run. The store is written **only**
by D14's root-owned `append-helper`, which exposes one append operation and no
update, replace, truncate, or per-record delete operation of any kind. Neither
`agent-worker` nor `orchestrator` nor `approval-controller` nor `publisher` may
write, rewind, or truncate it directly.

**The store layout is fixed, and the helper derives every component of it.**

```
<store root>/
  .lock                                  <- single-owner lock, held for the
                                            helper's whole lifetime
  periods/
    2026-08-02/                          <- current period, pinned
      approval-<64 hex>.rec
      publication-attempt-<64 hex>.rec
      .tmp-<random>                      <- in-flight only, never history
    2026-08-01/                          <- sealed period
      seal-<64 hex>.rec                  <- manifest of that period
  quarantine/
    00/                                  <- shard, bounded set, helper-owned
      2026-07-01.<token>/                <- removed from history, pending
                                            cleanup; date preserved
    01/
```

The helper opens the store root once, then opens or creates `periods/` and
`quarantine/` fd-relatively beneath it, then opens or creates the current
period directory fd-relatively beneath `periods/`. The period component is a
canonical UTC `YYYY-MM-DD` string the **helper** computes from its own clock; a
caller cannot name, select, hint at, or influence it. Records are written as
final basenames inside the pinned current-period descriptor, never at the store
root and never in a path a caller can address.

Rotation seals the current period and switches the pinned descriptor to the new
date. Retention operates one level up, renaming a whole child of `periods/`
into a shard of `quarantine/`. Because `periods/` and the quarantine shards are
under one root on one validated mount, that rename is a directory-to-directory
move within a single filesystem, which is what makes whole-period removal
atomic.

**A quarantined directory keeps its date, and the seal records it too.** The
physical name is `<YYYY-MM-DD>.<token>`: the canonical UTC date of the period
it was, plus an opaque token that makes the name unique and unguessable. An
opaque-only name, as an earlier draft had, meant that recovering after a crash
required reading a directory's contents to learn what period it had been, and
that a directory whose contents were partly removed might no longer be able to
say. The seal manifest independently carries the same canonical UTC date, so
the period's identity survives in two places: in the name, which recovery can
read without opening anything, and in the manifest, which survives a rename.

This changes nothing about what is observable. Surfaces still emit the period
date and the keyed correlation digest of D12 and never the raw token or a path,
so the date is available for operator reasoning while the token stays a
filesystem detail.

**Sealing writes a manifest, so the wholly-present invariant is falsifiable.**
This record claims a sealed period is byte-identical to when it was sealed.
Without something to check that against, the claim cannot be tested and a
period that quietly lost a record would look exactly like a period that never
had it. Sealing therefore writes one final `seal` record into the period,
listing the sorted basenames it contained, their count, a digest computed over
that sorted list, and the period's canonical UTC date. The manifest is itself
an immutable record, is written by the helper rather than by any caller, and is
the last thing a period receives. `gascity-audit verify` checks a period
against it, which is what makes a physically planted missing record detectable
rather than merely forbidden.

**The helper authenticates every caller and binds it to one record kind.** Its
Unix socket checks the peer credentials of each connection (`SO_PEERCRED`, or
the platform equivalent) against a closed matrix. Three record kinds have
external callers, and the mapping is one to one:

| Caller | Peer identity | May append | Any other kind |
| --- | --- | --- | --- |
| Approval controller | `approval-controller` | `approval` | reject |
| Publisher | `publisher` | `publication-attempt` | reject |
| Retention timer | uid 0 | `retention-deletion` | reject |
| Anyone else, including `agent-worker` and `orchestrator` | any | nothing | reject |

The `seal` kind has no external caller at all; only the helper originates it,
so the matrix above stays closed and a caller offering `seal` is rejected like
any other unauthorised kind.

Cross-kind requests are the point of the matrix, not an afterthought: a
compromised publisher cannot forge an approval, a compromised controller cannot
forge a publication audit trail, and neither can synthesise a retention record
to explain a period it removed. The retention timer is admitted precisely
because the retention rule below requires it to record its own deletions, and a
deletion it could not record would otherwise force either an unaudited gap or a
second write path. Being uid 0 grants it no additional kinds. No caller can
erase or alter anything already written, and the publisher re-reads and
verifies the record it acts on rather than trusting a value passed to it.

**The caller submits typed fields; the helper builds the bytes.** A caller does
not hand over canonical record bytes. Each kind has a strict schema of typed
fields with per-field bounds, the request **denies unknown fields**, and a
request is parsed against the schema of the kind the caller is authorised for
before anything else happens. The helper then constructs the canonical envelope
itself, embedding the **authenticated** kind, derived from the peer credential
rather than from anything the request asserted, hashes those helper-produced
bytes, and derives the final basename as that kind, a hyphen, 64 lowercase
hexadecimal characters, and the fixed suffix `.rec`. The canonical envelope is
bounded at **4096 bytes**; a record is a small set of digests, hashes,
identifiers and timestamps, and anything larger indicates a caller doing
something other than what this store is for.

Letting a caller supply the bytes was the defect this replaces. A publisher
could have submitted approval-shaped content and had it stored under a name
indistinguishable from a real approval, because the kind lived only in a field
the caller controlled. Now the kind is in the name, is in the hashed envelope,
and comes from the peer credential, so a publisher cannot cause any file
beginning `approval-` to exist by any request it is able to send.

Basenames are validated before use against exactly
`(approval|publication-attempt|retention-deletion|seal)-[0-9a-f]{64}\.rec`, so
even a defect in derivation cannot produce a name containing a separator, a
traversal component, a leading dot, or anything outside the permitted alphabet.

**Resolution is anchored and fd-relative.** Every operation runs relative to a
pinned descriptor, using `openat2` with
`RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS`, falling back
where the kernel lacks `openat2` to a component walk with
`O_DIRECTORY | O_NOFOLLOW` and `fstat` verification of each opened directory.
No ancestor is ever re-resolved by name. This is the shape this repository
already uses for its own root-adjacent writer,
`packages/xtask/src/delivery/storage.rs`, whose module documentation records
the same anchoring and the same reason: a pathname write is atomic only within
whichever directory each syscall happens to resolve, so an attacker who
replaces an intermediate component between validation and use can redirect the
write elsewhere.

The internal component resolver is a unit-testable surface in its own right,
because the derived-name path is pure hexadecimal and would never exercise it.
It is tested directly against `..`, an absolute component, a component
containing a separator, a symlinked component, and a magic-link component, and
must refuse each rather than resolving it.

**Close-on-exec is native or the helper does not start.** Every descriptor is
created close-on-exec by the creating call itself: `O_CLOEXEC` on every open,
`openat`, and `openat2`; `SOCK_CLOEXEC` on `socket`; `accept4` with
`SOCK_CLOEXEC` for accepted connections; `dup3` with `O_CLOEXEC` where a
descriptor must be duplicated. There is no post-creation `fcntl(F_SETFD)`
fallback, because that is not atomic: between the create and the `fcntl` there
is a window in which a concurrent `fork` and `exec` inherits the descriptor,
and a root-owned writer to the audit store is exactly the descriptor that must
not leak. If any required close-on-exec variant is unavailable, the helper
fails to start with a typed error rather than accepting the window.

**An append installs atomically and is durable before it is acknowledged.**
Writing the final name directly with `O_CREAT | O_EXCL` was wrong on two
counts: a reader can observe a short record while the write is still in
progress, and an acknowledged record can lose its directory entry to a power
loss because the parent was never synced. The sequence is instead:

1. create a temporary in the **pinned current-period directory**, with
   `O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC`, mode `0600`, verified by
   `fstat` to be a regular file owned by the helper. The temporary name carries
   a random component; an `EEXIST` collision is retried a bounded number of
   times with a fresh name and then fails;
2. write the complete bounded envelope, looping until every byte is written so
   a short write is completed rather than truncating the record, and retrying
   `EINTR` a bounded number of times; then `fsync` the file;
3. install it under the content-addressed final name with
   `renameat2(RENAME_NOREPLACE)` against the same pinned descriptor;
4. `fsync` the current-period directory;
5. only then acknowledge the append.

Every syscall in that sequence retries `EINTR` boundedly and treats exhaustion
of the bound as a failure rather than as success.

**Every abandoned temporary is unlinked on the path that abandoned it.** A
temporary is a resource the helper created and still holds, so the code that
gives up on it removes it rather than leaving it for a later sweep. That
applies to a failed write, a failed `fsync`, a failed rename, a validation
failure discovered after creation, and the `EEXIST` case below. Leaving them
behind is how a repeatedly retried caller turns a correct refusal into
unbounded disk growth.

**Temporaries are budgeted, not exempt.** In-flight temporary bytes count
against a dedicated bounded temporary budget **and** against the total store
budget of the retention section. They are not excluded from either on the
grounds of being transient: a leak that is invisible to the caps is a leak that
grows until the filesystem stops it. Exceeding the temporary budget fails the
append rather than evicting anything.

**`renameat2` is required, not preferred.** An earlier draft offered a
`linkat` plus `unlinkat` fallback for portability. That is removed. The
deployment is a single Linux workstation, `RENAME_NOREPLACE` has been available
for many years, and the fallback bought portability this deployment does not
need while adding a second install path with different failure modes,
including `EMLINK` on link-count exhaustion, that would have to be tested and
reasoned about separately. The helper asserts `renameat2` with
`RENAME_NOREPLACE` at startup against the actual store filesystem and refuses
to start if it is unavailable, with a typed configuration error. Copying is
never a fallback for either install or removal, because a copy is not atomic
and atomicity is the entire property being bought.

**A collision on the final name is idempotent success, or corruption.** The
name is the kind plus the digest of the helper-built envelope, so an existing
file under that name should be those exact bytes, which happens whenever a
caller retries after a crash between step 4 and step 5. The helper:

1. unlinks its own temporary first, so the retry cannot accumulate one per
   attempt;
2. opens the existing record anchored under the pinned descriptor and verifies
   its length equals the envelope's and its bytes hash to the same digest;
3. `fsync`s the containing period directory before returning. This is not
   redundant. The crash being retried may have happened after the rename but
   before the original directory `fsync`, so the entry the retry just observed
   is not yet durable; syncing here is what makes the retry's success mean the
   same thing as the original's would have;
4. returns **idempotent success**.

If the length or the bytes disagree, the helper does not overwrite, does not
repair, and does not succeed: it fails closed and reports corruption, because a
mismatch under a content-addressed name means something outside this design has
written to the store.

**Readers open final names only.** Temporaries carry a prefix that cannot occur
in a derived basename, and every reader enumerates the store accepting only
names matching the record shape. A temporary is never opened, never counted,
and never treated as history.

**Startup adopts the store under an exclusive lock, then reconciles
individually.** There is no unconditional sweep at startup, because a sweep by
a second instance while a first is still running would delete temporaries the
first is actively writing. The helper instead takes an open file description
lock on `<store root>/.lock` and holds it for its entire lifetime, so ownership
is tied to the open file rather than to a process id and is released by the
kernel when the owner exits.

A starting instance must acquire that lock **before it touches anything**. If
another instance holds it, the new one does not sweep, does not reconcile, does
not delete, and does not open a period for writing: it fails to start, or
parks, with a typed error naming the conflict. No cleanup of any kind happens
before ownership is exclusive.

Once ownership is exclusive, reconciliation is per temporary rather than
wholesale. For each temporary found: read it, and if it parses as a canonical
envelope, derive its final name and check for that name. If the final record
exists, verify it against the envelope and unlink the temporary, since the
append had in fact completed. If no final record exists, or the temporary does
not parse or is short, it is an uncommitted fragment that was never history,
and it is removed under a bounded policy. Nothing in this path appends a
record, because nothing in it changes history.

**Retention is bounded by stated defaults.** "Bounded" is not a default, so the
first deployment ships these, and the asymmetry between them is deliberate:
event logs are high volume and low evidentiary value, while audit records are
tiny and are the only proof that a human authorised anything.

| Store | Rotation | Retention floor | Aggregate cap | Per-file cap |
| --- | --- | --- | --- | --- |
| Gas City event logs | daily | 14 days | 512 MiB | 64 MiB, rotates early if exceeded within a day |
| Approval records | daily seal | 365 days | 256 MiB | not applicable |
| Publication audit records | daily seal | 365 days | 256 MiB | not applicable |

For event logs the first bound to bind wins: a chatty fortnight is trimmed by
the byte cap before the age floor is reached, and a quiet one is trimmed by age.
For the audit stores the floor wins and the cap is a backstop that should never
bind. Records are digests, hashes, and timestamps, so a year of them on a
single-user workstation is small; if the cap ever would bind before the floor,
deletion is **refused** and the condition is reported, because silently
discarding audit history to satisfy a disk budget is the failure this
requirement exists to prevent.

These values are configurable **only in the external deployment repository**,
alongside the rest of the deployment configuration. They are not exposed
through `.d2b-orchestration.toml`, a d2b option, or any surface inside this
repository, per D1.

**Retention and append-only are reconciled by removing whole periods and
nothing else.** The append API has no update, replace, truncate, or per-record
delete operation, and gains none. Enforcement is instead a separate root-owned
retention timer, which may do exactly one thing: remove an entire sealed period
once the retention floor has passed for it. It may never truncate a period,
never rewrite or re-encode one, never remove an individual record from within
one, and never touch the current unsealed period. A period is therefore either
wholly present and byte-identical to when it was sealed, or wholly absent, and
its seal manifest is what makes that checkable.

**Removal is a rename first, a delete second.** Deleting a populated directory
in place is not atomic: a power loss part way through leaves a visible sealed
period missing an arbitrary subset of its records, which is exactly the state
the invariant above forbids and is indistinguishable from tampering. The timer
therefore proceeds in this order:

1. **If a new shard is needed, create it and make the shard entry durable
   first.** A shard directory is created fd-relatively under `quarantine/`,
   validated for device and mount identifier like every other shard, and then
   `quarantine/` itself is `fsync`ed **before** anything is moved into the new
   shard. Skipping that sync allows a crash in which the period has been
   renamed into a shard whose own directory entry never reached disk, leaving
   the period reachable from neither `periods/` nor a surviving shard: an
   invisible period, which is worse than either state the invariant permits.
2. `renameat2(RENAME_NOREPLACE)` the whole period directory from `periods/`
   into a validated quarantine shard, under the name `<YYYY-MM-DD>.<token>`,
   both descriptors pinned and both close-on-exec. A destination `EMLINK`
   selects the next shard and retries, per the sharding rule above, repeating
   step 1 if that shard must be created.
3. `fsync` **both directories that the rename modified**: the `periods`
   descriptor, which lost an entry, and the **selected shard** descriptor,
   which gained one. Syncing a common parent is not equivalent and is not what
   this requires; a cross-directory rename changes two directories and both
   must be made durable before anything depends on the move having happened.
   When a retry moved the destination to a different shard, the descriptor
   synced is the shard that actually received the directory.
4. Append the `retention-deletion` record naming the period date, its
   quarantine correlation id, its record count, and its seal digest.
5. Only then remove the quarantined directory's contents recursively.
6. **Remove the now-empty quarantined period directory itself**, unlinked
   fd-relatively from its shard, and `fsync` that shard descriptor. An emptied
   directory that is never removed still holds a link in its shard, so a
   deployment that retains for a year would march toward the very link-count
   limit the sharding exists to survive, one directory per day, while looking
   like it had cleaned up. Reclaiming the directory is what makes retention
   steady-state rather than merely slower-growing.

Ordering the two `fsync` calls before the record is what keeps the audit trail
honest. A record appended before the rename was durable could describe a
deletion that a crash then un-did, leaving history asserting the removal of a
period that is still present.

A crash leaves exactly one of two states: the original sealed directory,
untouched and still visible, or a quarantined directory that is no longer part
of history. There is no third state in which a visible sealed period is
partially deleted.

**The quarantine token is a filesystem detail and never becomes observable.**
The directory under `quarantine/` is named with an opaque token, and that token
appears in no log line, no audit record, no error message, and no operator
output. Observable surfaces identify a quarantined period by two things: its
canonical `YYYY-MM-DD` period date, and a **quarantine correlation id** that is
a keyed digest over the token under the same deployment-scoped key D12 uses for
its other identity digests. The correlation id is stable, so an operator can
follow one quarantine across a status listing, an error, and an audit record,
without any surface publishing a path or a raw token that would invite someone
to act on it directly.

**The store must be one mount, validated two ways, and a copy fallback is
refused rather than deferred.** The whole-period guarantee rests on the rename
being atomic. A copy-then-delete substitute has a window in which the period
exists in both places or in neither, which is exactly the partially-visible
state this design forbids and which an earlier panel round required be
impossible. So there is no copy fallback for either install or removal, and its
absence is a decision rather than an omission: adding one would silently
convert the invariant into a best-effort claim, and every later reader would
assume the guarantee still held.

Validation is therefore two-layered, because `st_dev` alone is not sufficient.
At startup and at activation the helper stats `periods/` and every quarantine
shard and requires both:

- the same `st_dev`, and
- the same Linux mount identifier from `statx` with `STATX_MNT_ID`.

Two directories can share a device and still sit on different mounts, for
instance across a bind mount, and a rename between them fails `EXDEV` even
though a device comparison said they matched. Checking only the device produces
a deployment that passes its own startup check and then fails at the first
retention run, months later, on the one operation that must not fail.

The two failures are distinct errors with distinct remediation, and are never
collapsed into one message:

- **`audit-store-cross-filesystem`** when the devices differ. The remediation
  is to place the whole store on one filesystem.
- **`audit-store-cross-mount`** when the devices match but the mount
  identifiers differ. The remediation is to remove the bind mount or nested
  mount inside the store, which is a different action and would be
  undiscoverable from a device-oriented message.

An `EXDEV` observed later maps to whichever of those two conditions the helper
finds and **parks retention** with that error rather than degrading. Parking is
correct here: a retention run that cannot proceed atomically leaves history
intact and over budget, which is a reportable condition, whereas proceeding
non-atomically risks a state no invariant covers.

**Destination `EMLINK` is handled by sharding, not by copying.** A rename into
a directory can fail `EMLINK` when the destination's link count is exhausted,
which on some filesystems is a hard limit on subdirectory count. `quarantine/`
is therefore a bounded set of helper-owned shard directories rather than one
flat directory. On `EMLINK` the helper selects or creates the next shard
fd-relatively under the same containment rules as everything else, validates
that shard's device and mount identifier against `periods/` exactly as at
startup, retries the atomic rename into it, and on success syncs both the
source `periods/` descriptor and the selected shard descriptor, which are the
two directories that rename modified.

If every shard in the bounded set is exhausted, the helper emits
`audit-quarantine-link-limit` naming the period date and its correlation
digest, with a remediation that is a sequence rather than an instruction to go
and delete something:

1. run `gascity-audit status` to see the outstanding quarantines and why each
   is still there;
2. if any is blocked on a failed audit append, repair the `append-helper` unit,
   which is the usual cause;
3. run `gascity-audit converge`.

`converge` processes the outstanding quarantines that are already audited,
removing their contents and then their now-empty directories per step 6 above,
which is what frees link capacity in the shard. So the remediation resolves the
condition by **completing work the system already owed**, not by discarding
anything: an audited quarantine was always going to be removed, and converge
simply does it now. It never asks the operator to delete a store entry by hand,
because a human unlinking things inside the audit store is the failure mode
this whole design is built to make unnecessary.

The helper does not copy, and does not delete anything unaudited to make room,
because both would trade the invariant for throughput on a path that is
already exceptional.

**Recovery resumes, and never deletes unaudited.** Under the exclusive
ownership established at startup, the timer enumerates the quarantine shards
and, for **every** quarantined period it finds, applies the same two-stage rule
without exception:

1. **If the `retention-deletion` record for that period is absent, append it.**
   If that append cannot be made, for any reason including the append helper
   being unavailable, stop here: the quarantined directory is **retained and
   retried** rather than removed, because a period deleted without its record
   is a gap the history cannot explain.
2. **Once the record is present, complete the removal unconditionally.** It
   makes no difference whether the record was already there from an earlier
   attempt or was appended a moment ago; in both cases the timer resumes and
   finishes the recursive content removal, unlinks the now-empty period
   directory from its shard, and `fsync`s that shard.

The wording matters because the earlier phrasing read as though removal
followed only from the append, leaving the already-audited case undefined.
That case is the common one after a crash or a repaired helper, and leaving it
ambiguous is how audited quarantines accumulate forever while every individual
step looks correct: the record exists, so nothing appends, and removal never
runs because nothing triggered it. Recovery is idempotent by construction, and
an already-audited quarantine is work owed rather than work done.

**A retained quarantine is reported, not merely accumulated.** Letting the
256 MiB backstop be the first signal would surface a broken append helper as a
disk-capacity message weeks later, which tells the operator nothing about the
cause. The timer instead emits the closed error `retention-audit-append-failed`
on the first failed attempt, naming the period date, its quarantine correlation
id, the underlying append error, and the remediation: inspect the
`append-helper` unit, then run the status command below.

**The operator surface is read-only inspection plus an explicit converge.** The
configuration repository owns `gascity-audit`, whose relevant subcommands are:

- `gascity-audit status` lists every quarantined period with its date,
  correlation id, cause, last append error, age, size on disk, and the
  remediation for that cause. It changes nothing.
- `gascity-audit converge` runs the recovery and retention sequence
  immediately, rather than waiting for the next timer firing, so an operator
  who has just repaired the append helper gets closure now instead of
  tomorrow. It performs the **same** sequence with the same ordering: it cannot
  skip, defer, or substitute for the missing `retention-deletion` append, and a
  quarantine whose record still cannot be appended is still retained rather
  than removed. For quarantines that are already audited it completes the
  removal, including unlinking the now-empty period directory and syncing its
  shard, which is how it frees shard link capacity. The documented flow is
  `status` first to see what is outstanding and why, remediate the
  `append-helper`, then `converge`.
- `gascity-audit verify` checks integrity. With `--period <date>` it verifies a
  sealed period against its seal manifest, reporting any record present in the
  manifest and missing from the directory, or present in the directory and
  absent from the manifest. With `--record <kind>-<digest>` it verifies one
  record's bytes against its content-addressed name.

None of these can delete a record or a quarantined period. Deletion of a
quarantine happens only by fixing the append path and letting the sequence
complete, because any other route is an unaudited deletion by another name.

**A corrupt record blocks publication and is never resolved by deleting it.**
If the helper finds bytes under a record name that do not hash to it, or
`verify` reports a mismatch, the correct response is not to remove the
offending file. Deleting history to make a check pass is the failure this
entire store exists to prevent, and an operator instructed to do it once will
do it again. The diagnostic is a closed fail-closed error naming the record,
the expected and computed digests, the length seen, and exactly one next step:
run `gascity-audit verify --record <kind>-<digest>` for the full report, then
restore the protected audit store from a known-good backup, or reinitialise it
only through the explicit archived-store recovery procedure documented in the
configuration repository, which preserves the corrupt store as evidence rather
than discarding it. Publication stays blocked until the store verifies clean,
because a publisher that cannot trust the approval history has no basis to act.

Each retention deletion is therefore recorded through the same append path as
everything else, is equally immutable, and is written by the one caller
authorised for that kind.

**Only the approval controller may originate a record, and it is not the
worker.** A record reaches the append helper only from D14's
`approval-controller`, and only after that principal has verified the Discord
platform signature, applied D6's allowlist, and derived the artifact identity
itself under D14. `agent-worker` has no origination path by any route: not a
file it can write, not a socket that accepts a record from it, not a helper
that appends on its behalf. Read access for the worker is permitted, because a
run must observe whether its gate is satisfied and reading cannot forge.

This is the half of the boundary that is easy to omit. Withholding the GitHub
credential from agents accomplishes nothing on its own if an agent can write
the approval that makes the publisher act; the credential stays where it is and
the branch is published anyway, through the sanctioned path. M15 tests for it
directly.

**D13. The dashboard runs read-only, bound to loopback, and is not an
interaction surface.** Loopback is not access control here. Every agent this
deployment runs is a local process on the same host, so a loopback bind admits
exactly the principals the design is most concerned about. The dashboard's own
respond path proves the point: `POST /v0/city/{cityName}/session/{id}/respond`
answers an agent's pending interaction prompt, so a worker agent that can reach
the dashboard can approve its own permission requests.

The first deployment therefore runs the dashboard in its read-only mode,
measured upstream as `DASHBOARD_READONLY === '1'`
(`backend/src/config.ts:275`), whose proxy rejects every non-GET/HEAD forward
with a 405 (`backend/src/routes/supervisor-transport-proxy.ts:170`) and strips
the mutation-authorizing request header. Loopback binding stands as a second
layer, not the first. If a future upstream renames or restructures that knob,
the requirement is the property rather than the variable: no mutation endpoint
reachable, proven by M14 rather than by configuration intent.

Discord, gated by D6's allowlist and D12's approval store, is the only human
interaction surface. The dashboard is for run observation, stage state,
transcripts, live activity and debugging. It may not carry an artifact
approval, may not answer a pending interaction, and may not be exposed beyond
loopback or fronted by a proxy; each of those is a new record, not a
configuration change.

**D14. Five host principals: four dedicated unprivileged identities plus a
root-owned helper.** The design brief's
rule is that implementation agents do not hold publishing authority. Two
earlier forms of this decision failed to deliver it. The first withheld the
GitHub credential but let the worker write the approval store, so an agent
could approve its own revision and invoke the publisher **legitimately**. The
second offered a two-identity variant in which `LoadCredential=` was claimed to
confine the credential to one unit; that claim is wrong. Credentials placed in
`$CREDENTIALS_DIRECTORY` are readable by the unit's own uid, so any process
sharing that uid, including a network-facing one, can read them. `LoadCredential`
isolates a secret from *other* uids and from the general filesystem; it does not
isolate it from a peer process running as the same user. The variant is
withdrawn, and this record now requires a distinct Unix identity for every
boundary that must be mechanical.

`/etc/nixos` establishes five principals: four dedicated unprivileged
identities, each with its own uid, and one root-owned helper service. This is
host contributor infrastructure and involves no d2b component.

| Principal | Holds | Must not be able to |
| --- | --- | --- |
| `orchestrator` | Durable Gas City state, the protected integration object store | Run agent code |
| `agent-worker` | Model credential, its own worktrees, a restricted network namespace (D15) | Read orchestrator state or memory, write the integration store, write approvals, publish |
| `approval-controller` | Discord ingress, the verifying key, the allowlist | Hold a GitHub credential, run agent code, rewrite approval history |
| `publisher` | The one GitHub credential scoped to `vicondoa/d2b` | Run agent code, accept a mutable ref, write approvals |
| `append-helper` | Root-owned; sole writer of the approval and audit store, admitting three authorised caller-and-kind pairs per D12 | Expose any update, replace, truncate, or per-record delete operation |

Four dedicated unprivileged identities plus one root-owned helper service is
cheap on one machine: the four are `users.users` entries with units, the helper
is a root-owned service rather than a fifth login account, and the deployment
is single-user and local, so there is no directory, no role mapping, and no
lifecycle to manage. What is not cheap, and is therefore forbidden, is
asserting any of these boundaries within one uid. A separation enforced by file
mode inside a single uid, a wrapper script, a directory convention, environment
scrubbing, a proxy process, or an instruction to agents is not a boundary in
this design and may not be substituted for a distinct identity.

**Orchestrator and worker are separate because same-uid is not a boundary.**
A process can `ptrace` and read `/proc` state of another process running as the
same user, so an agent sharing the orchestrator's uid could read its memory,
its file descriptors, its environment, and any token it holds, and could drive
its sockets directly rather than through whatever wrapper it was offered.
Splitting the uid removes that class outright. The worker reaches the
orchestrator only over a Unix socket whose server checks the peer credentials
of every connection (`SO_PEERCRED`, or the platform equivalent) and serves a
closed set of request types, rejecting any peer uid outside the expected one.
Same-uid wrapper or proxy separation is forbidden here specifically.

**The protected integration object store gives the artifact an identity the
worker cannot rewrite.** The worker commits in its own worktree, which it owns
and may rewrite freely; that is not the artifact. When a task completes, the
**orchestrator** ingests the result into a Git object store it owns and the
worker cannot write, producing an **integration commit**. The hash of that
commit is the artifact identity for every downstream stage. Because Git objects
are content-addressed and the store is owned by a different uid, the bytes
behind a given hash cannot be changed by the worker after the fact, and a
second commit with different content necessarily has a different hash.

**The approval controller derives the artifact identity itself.** It resolves
the integration commit **by hash** from the protected store, reads the commit
and its tree directly, and constructs the Discord prompt from what it read: the
commit hash, the changed paths, and the diff statistics it computed. It accepts
from the workflow only the hash and the gate node identifier. It does not take
a summary, a title, a description, a rendered diff, or any hidden interaction
metadata from the worker, and it does not display worker-authored prose as
though the controller vouched for it.

That rule exists to defeat a confused-deputy attack the previous draft allowed:
a worker could present benign prose to the operator while binding the approval
to the hash of different bytes, and the operator would approve what they read
rather than what would ship. Deriving the prompt from the same immutable object
the publisher will push closes the gap, because the operator, the approval
record, and the publisher all resolve one commit hash to one set of bytes.

**The publisher accepts a commit hash and nothing else.** It takes a full
immutable commit object name, rejects every mutable reference including branch
names, tag names, `HEAD`, symbolic refs, and any expression that resolves
through one, and it pushes that object. Before acting it independently reads
the approval record from the store and requires all of: node equal to the
publication gate, decision equal to `approve`, reviewer equal to the configured
operator, and approved commit hash equal to the commit presented. Any mismatch,
including an otherwise-valid approval recorded against an earlier gate, is a
refusal.

Accepting a ref was a real defect and not a stylistic one: a worker able to
move a ref between the moment of verification and the moment of push publishes
bytes nobody approved, and the approval record afterwards describes something
that was never shipped.

**The helper's exact protocol and transport remain prototype and specification
work**, and this record deliberately does not invent them; several plausible
shapes exist on a NixOS host and choosing among them needs measurement rather
than assertion. What is not deferred are the properties the split exists to
provide, each an MVP acceptance condition independent of which protocol wins:
M11 for credential unreachability, M15 for approval unforgeability, M16 for
orchestrator-state unreachability, and M18 for append-only history.

Credential rules that apply to every principal:

- **Model access** is whatever the configured providers require, scoped to
  model inference, and reaches `agent-worker` only. Where a model provider is a
  cloud service, its credential is a model-provider credential and is
  permitted; it is named in the configuration and scoped as narrowly as that
  provider allows.
- **Excluded outright** are credentials for unrelated infrastructure: compute,
  storage, deployment, CI administration, secret managers, or any cloud service
  the workflow does not use for model inference; credentials for another
  repository or organisation; and any credential the configuration does not
  name a use for.
- Credentials are never passed between principals, and redaction of credential
  values from logs, state files, and error text is a requirement on the
  configuration repository's own tooling.

**D15. The agent-worker runs in a restricted host network namespace with
default-deny egress.** An agent that inherits the host network namespace also
inherits every route the host has, which on this machine includes the d2b
per-environment bridges the operator is developing against. That would let
contributor tooling reach d2b guest networks while this record claims no d2b
relationship, and it would weaken the environment isolation d2b exists to
provide, without any d2b component being involved in the breach.

`agent-worker` sessions therefore run in a dedicated network namespace
configured by `/etc/nixos`, with its own interface and its own firewall policy.
This is ordinary host networking: no d2b module, no d2b bridge, no broker, and
no d2b service participates in it.

Denied, by default and explicitly:

- every d2b bridge, TAP, and per-environment interface, and every address on
  them;
- RFC1918 and other LAN ranges, and any other address on the host's own local
  networks;
- link-local, `169.254.0.0/16` and `fe80::/10`, including metadata endpoints;
- the host's loopback services. The namespace has its own loopback, so the
  dashboard, the orchestrator socket, and any other host-loopback listener are
  simply not addressable from inside it. This is why D13's read-only
  requirement and this decision are complementary rather than redundant: D13
  holds if the namespace is ever misconfigured, and this holds if the
  dashboard's mode is ever wrong.

Allowed, only when named in the configuration:

- the configured model provider endpoints;
- package registries and Nix substituters the tasks need;
- DNS to a specified resolver;
- GitHub **read** access, if and only if a task genuinely requires fetching
  rather than working from the checkout. Publication is unaffected: the worker
  has no credential to publish with under D14, so read reachability grants it
  nothing.

The policy is default-deny with an explicit allowlist, not default-allow with
exceptions, so an endpoint nobody considered is refused rather than reached.
Unix-domain sockets are not affected by the network namespace, which is why
D14's peer-credential-checked socket remains the worker's channel to the
orchestrator and does not require relaxing anything here.

**The confinement outlives every process it confines.** The namespace, its
veth or other interface, its DNS proxy, and its firewall rules are created
before the first agent session starts and are torn down only after the last one
has exited or been forcibly terminated at the end of D5's grace period. There
is no window, at shutdown or at reconfiguration, in which an agent process is
still running while the confinement around it is being dismantled.

This matters because the failure is silent and favourable to the attacker. If
teardown races an agent that is ignoring its stop request, that agent is
briefly a process in the host namespace with the host's routes, which is
precisely the d2b bridge and LAN reachability D15 exists to deny, and it
happens at the moment supervision is least attentive. Ordering teardown strictly
after termination removes the race rather than narrowing it, and M17 tests it
with an agent deliberately kept alive through the whole grace period.

M17 tests the policy with planted denied targets and allowed controls, because
a firewall that denies everything, including what the workflow needs, passes a
naive "is it blocked" check while being useless.

**D16. Publication is one step, gated by a human, audited on every attempt,
with the lint as defence in depth.** Pushing a branch and creating a pull
request happen only in a publisher step, only against the immutable commit hash
approved under D12, and only through D14's publisher. If the upstream `github`
pack's `push-branch` and `create-pr` commands are used, they are reachable from
that step and from nowhere else, and the formula set carries a lint asserting
it.

**Every publication attempt emits exactly one audit record, whether it
succeeds or fails.** Publication is the only effect this system has outside the
host, so an attempt that leaves no trace is the one case where operational
evidence matters most. The record is appended through D12's append-only helper
and carries: the commit hash, the approval record's own digest, the outcome as
one of a closed set, the failure class when it failed, and the timestamp. It is
bounded in size and redacted, carrying digests rather than the reviewer's raw
identity or the run handle, and it never carries a credential, a token, a
remote URL with embedded authentication, or captured command output. Exactly
one record per attempt: a refusal before the push is still an attempt, and a
retry is a second attempt with its own record.

The lint is a third layer, not the control. The controls are D14's separations:
a formula that reached a publishing command from the wrong step would still
fail because the worker holds no publishing credential, a run that manufactured
its own approval would still fail because the worker cannot originate a record
and the publisher re-verifies the record itself, and a run that tried to swap
the bytes after approval would still fail because the publisher accepts only an
immutable commit hash. Ordering them this way is deliberate, since a lint over
a formula set is exactly the kind of guard an agent editing formulas could
regress.

Automatic publication is not configurable in the first deployment; the
capability is absent rather than disabled. The publisher is not the merger:
`v3` is protected and the merge stays the operator's, as it is today.

**D17. State the trust boundary honestly.** Agents run as `agent-worker` with
that identity's privileges, in git worktrees, inside D15's network namespace,
with the model credential reachable. There is no sandbox, no isolation of an
agent from its sibling worktrees, and no confinement of an agent to the
checkout. The blast radius of a misbehaving or compromised agent is that
identity and everything it can reach.

What that blast radius **excludes**, mechanically rather than procedurally, is
a set of five properties, each enforced by a distinct identity or namespace
rather than by a rule an agent is asked to follow:

1. It cannot publish, because the GitHub credential is not present in its
   identity and is not readable from it.
2. It cannot manufacture the approval that would let it invoke the publisher
   legitimately, because only `approval-controller` originates records and the
   publisher re-reads and verifies them.
3. It cannot move the bytes after approval, because the publisher accepts only
   an immutable commit hash resolved from a store the worker cannot write.
4. It cannot read or attach to orchestrator state, because that runs under a
   different uid and is reached only through a peer-credential-checked socket.
5. It cannot reach d2b networks, the LAN, link-local addresses, or host
   loopback services, because it is in its own network namespace under
   default-deny egress.

Properties 1 through 3 are required together: any one alone leaves a complete
path from a compromised agent to a published branch. Property 4 is what makes
the other four durable, because an agent that can read the orchestrator's
memory can extract whatever it holds. Property 5 is what keeps a contributor
tool from undercutting the product it is used to build.

The dashboard sits behind two of these. A worker agent cannot reach it at all
from inside D15's namespace, and if that namespace is ever misconfigured, D13's
read-only mode still refuses the mutation. Neither is presented as sufficient
alone. A local agent is a local principal; "only I can reach it" is a statement
about humans, and D6 says so explicitly.

Everything else is scope and gating rather than containment: model access
scoped to inference, human approval at five named gates, the publication lint,
branch protection on `v3`, per-task worktrees, and a delivery state root that
stays outside every working tree. Those reduce the chance of a bad change
reaching `v3`; they do not contain a bad agent inside the worker identity. Any
claim stronger than that is unsupported by this design, and D2 forecloses the
obvious way to strengthen it until a separate ADR takes that up.

**D18. The version pin is a three-way check, not a single lock.** Three things
must agree, and each is pinned in a different place, so an upgrade that moves
one without the others is caught rather than discovered at runtime:

1. **The configuration repository revision**, which carries the module, the
   formulas, and the city configuration.
2. **The `llm-agents.nix` revision**, pinned in that repository's own
   `flake.lock`, which determines the `gascity` package version and therefore
   the `gc` binary. At the commit measured above that package is Gas City
   `1.4.0`.
3. **The Gas City pack lock**, `packs.lock`, whose schema records `version`,
   `commit` and `fetched` per pack, with imports pinning a commit through
   `Import.Version = "sha:<hex>"`.

`gascity_compat` in `.d2b-orchestration.toml` is the assertion that ties the
target repository to the deployment: it is checked against the version the
deployed binary reports through `gc version`, not against the flake input's
attribute name and not against what the module intended to install. Checking
the running binary is the point, since the binary is what executes the
formulas.

Upgrading is a deliberate change to the `llm-agents.nix` lock in the
configuration repository, followed by re-checking `gascity_compat` and the pack
locks against the new binary. Rolling back is the same operation in reverse:
restore the previous lock revision, which restores the previous package version
by construction, because the lock is the version. The configuration repository
documents both, and an upgrade that changes a pack's formula step identifiers is
treated as a breaking change to the run.

Automatic updates upstream are the reason this is stated as a three-way check
rather than left implicit. `llm-agents.nix` describes itself as automatically
updated daily, so an unpinned or casually-followed input would move the `gc`
binary underneath a pinned pack set without anyone deciding to.

**D19. The importer is a lenient parser with a strict output.** `tasks.md` has
no schema and no format version, so the importer parses what is reliable (the
`- [ ]` checkbox, the `T\d{3,}` identifier, the `[P]` and `[US\d+]` markers,
and `## Phase N` headers) and treats the rest as advisory. Prose dependencies
and prose file paths are extracted best-effort and then **confirmed by a human
at the task-DAG approval gate**, which is already in the flow.

An unparseable line fails the import, and the failure **names the expectation
that was not met** as well as quoting the line. Quoting alone leaves the
operator guessing which of several required tokens was wrong: the `- [ ]`
checkbox, the `T` plus three-or-more digits identifier, the `[P]` or `[US<n>]`
marker shape, or the enclosing `## Phase N` header. The message states which
element the parser required, what it found in that position, and the line
verbatim. A line is never silently dropped, and re-import is keyed by task ID
and must not duplicate unchanged tasks.

## Non-goals, and what this ADR does not authorize

The first two items are the load-bearing ones.

- **No d2b product surface, of any kind.** No flake output, no `d2b.*` option,
  no `nixos-modules/` content, no crate in `packages/`, no manifest field,
  bundle artifact, schema, wire message, broker op, or CLI verb, no
  `docs/{reference,how-to,explanation}/` page, no `README.md` mention, no
  critical-subsystems row, and no released changelog entry presenting Gas City
  as a d2b capability. Gas City is not on d2b's roadmap, is not a supported
  d2b feature, and carries no compatibility promise to anyone outside this
  repository's contributors.
- **No d2b execution substrate, in any form.** No d2b runtime provider, no Gas
  City runtime pack targeting d2b, no microVM sandbox, no declared VM pool, no
  guest artifact return, no workspace transfer protocol, no guest credential
  injection, and no use of `d2bd`, the broker, the guest-control transport, or
  the Credential service. None of this is deferred, staged, or reserved by this
  record; it is absent, and reintroducing any of it requires a new ADR that
  argues for it on its own merits.
- No fourth root-visible unit, and no change that engages ADR 0015.
- No change to `.github/agents/`, `.github/skills/`, or `scripts/copilot/`, and
  no removal or conditioning of the standalone skills. Gas City is never a
  precondition for contributing.
- No change to `PANEL_PROVIDER_POLICY`, `PANEL_MODEL_POLICY`,
  `PANEL_REASONING_EFFORT_POLICY`, `PANEL_ROLES`, or any part of
  `packages/xtask/src/delivery/`.
- No claim that this design isolates agents, beyond the two separations D14
  establishes and M11 and M15 prove. See D17.
- No multi-party deployment shape. No role model, permissions matrix, tenancy,
  shared or hosted instance, remote access path, high availability, failover,
  or federation. No authentication layer for the dashboard, and no exposure of
  it beyond loopback; a proxy or bind-address change in front of it is a new
  record, not a configuration tweak.
- **No mutable dashboard surface.** The dashboard runs read-only, and its
  respond, approve, deny, sling, and bead-mutation endpoints are unreachable.
  Loopback binding alone is not accepted as the control, because every agent is
  a local principal.
- **No worker-writable approval store**, and no design in which the publisher
  trusts an approval assertion supplied by its caller rather than reading and
  verifying the record itself.
- **No mutable approval history.** No update, replace, truncate, or per-record
  delete operation is exposed by the append helper to any principal, no
  rotation scheme rewrites a sealed period, and no retention path deletes a
  visible sealed period in place or without its `retention-deletion` record.
- **No caller-supplied bytes, names, or paths in the store.** A caller submits
  typed fields for the one kind it is authorised for; the helper builds the
  canonical envelope, embeds the authenticated kind, hashes its own bytes,
  derives the kind-bound basename, and computes every period component from its
  own clock. Requests deny unknown fields. No caller can cause a record of a
  kind it does not own to exist under any name.
- **No unaudited or operator-initiated deletion of history.** No operator
  command deletes a record or a quarantined period, and no diagnostic ever
  instructs an operator to remove a conflicting record. Corruption is resolved
  by restoring from backup or by the archived-store recovery procedure, with
  publication blocked until the store verifies clean.
- **No cleanup before ownership.** No sweep, reconciliation, unlink, or period
  open for writing happens before the single-owner store lock is held
  exclusively, so a second instance can never delete a first instance's
  in-flight state.
- **No non-atomic close-on-exec.** Descriptors are close-on-exec from the
  creating call; there is no post-creation `fcntl` fallback, and a platform
  lacking a required variant fails startup instead.
- **No unbudgeted temporaries.** In-flight temporary bytes count against both a
  dedicated temporary budget and the total store budget, and every abandoned
  path unlinks its own temporary.
- **No `llm-agents.nix` or `gascity.nix` in d2b.** Neither is a flake input, an
  overlay, a dependency, or a mention in `flake.nix`, `flake.lock`, or
  `nixos-modules/`. They exist only in the shared configuration layers and the
  private host deployment, and d2b neither builds nor tests against them.
- **Nothing d2b-specific in `gascity.nix`.** The generic module carries no d2b
  name, identifier, path, option, default, workflow, or assumption; no
  repository or rig binding; no formulas, panel or delivery policy; no Discord
  identity values; no d2b egress entries; and no task-import logic. Its public
  option namespace is rooted at `services.gascity` and never names a consumer.
  All of that lives in `d2b-gascity-configs`, and M21 proves the absence
  mechanically rather than by review.
- **No hand-built or hand-wrapped `gc`.** Neither shared repository rebuilds
  Gas City, vendors it, or restates its runtime dependency closure.
- **No copy fallback anywhere in the store.** Neither record install nor
  whole-period removal may degrade to a copy when a rename is unavailable.
  `periods/` and `quarantine/` share one filesystem, asserted by `st_dev` at
  startup, and `EXDEV` is a typed fail-closed configuration error. Likewise
  `renameat2` with `RENAME_NOREPLACE` is required rather than preferred; there
  is no `linkat` install path, and an unsupported filesystem refuses startup
  instead of selecting a weaker one. A copy is not atomic, and atomicity is the
  whole property being bought.
- **No mutable ref in the publication path.** The publisher accepts a full
  immutable commit hash only, and rejects branch names, tag names, `HEAD`,
  symbolic refs, and anything resolving through one.
- **No worker-supplied approval prose or metadata.** The approval controller
  derives the artifact identity and constructs the operator-facing prompt from
  the protected immutable object, never from text or hidden interaction
  metadata originating in the worker.
- **No raw identifiers in durable records.** Discord user ids and run handles
  appear only as keyed digests in approval records, audit records, diagnostics,
  and event logs.
- **No unbounded state.** Event logs and approval or audit history carry
  shipped rotation and retention defaults rather than growing until the disk
  fills.
- **No host-network agents.** Worker sessions do not run in the host network
  namespace and do not inherit host routes, and no d2b bridge, guest address,
  LAN range, link-local address, or host loopback service is reachable from
  them.
- **No same-uid separation anywhere in the trust model.** A boundary asserted
  by file mode within one uid, by a wrapper script, by a proxy process, by
  directory convention, by environment scrubbing, or by an instruction to
  agents does not count as a boundary in this design and may not be substituted
  for a distinct identity. This explicitly includes systemd per-unit
  credentials: `LoadCredential=` isolates a secret from other uids, not from a
  peer process running as the same uid, so it is a delivery mechanism and never
  a trust boundary.
- **No absence scan without counted coverage and a planted control.** A scan
  that cannot show it examined a non-empty corpus, and cannot show it rejects a
  planted violation, does not satisfy any acceptance condition in this record.
- No Discord ingress from any identity other than the one configured operator,
  and no allowlist that defaults open when unset or empty.
- **Single-user scope is never a reason to collapse identities or relax D13.**
  D6 removes multi-human concerns; it removes nothing about what a local agent
  process may do, and D13 and D14 stand independently of it.
- No credential beyond those D14 names, no credential value committed to
  either repository, and no publishing credential reachable from the worker
  identity.
- No assertion that a Gas City step can invoke an in-session Copilot slash
  command. The panel bridge is unproven; see D11 and M10.
- No `.gc/` directory in this repository, and no delivery evidence, transcript,
  or attestation payload committed to Git.
- No freezing of upstream syntax beyond what the measured commits above prove.
  The `city.toml` layout, the formula step vocabulary, the Discord interaction
  primitives, and the approval store's wire shape are deferred to the follow-on
  specification, which must re-measure them at its own pinned commits.

## Acceptance

Every condition below is evaluable by a machine except the one half of M1 that
is explicitly marked as a reviewer judgement, which is called out rather than
disguised as a scan.

**Every absence scan below is subject to two standing requirements**, because
an absence scan is the easiest kind of check to pass for the wrong reason. A
renamed directory, a moved corpus, a typo in a glob, or a pattern broken by an
escaping change all produce a clean result that means nothing.

1. **Counted, non-empty coverage.** Each scan reports the number of files and
   the number of candidate sites it actually examined, asserts that number is
   greater than zero, and fails closed when its input set is empty or when the
   count falls below a committed floor. A scan that examined nothing is a
   failure, not a pass.
2. **Planted negative controls.** Each scan ships fixtures containing exactly
   the forbidden references it exists to catch, and the scan is required to
   **reject** them. A scan that cannot demonstrate a detection on a planted
   violation has not been shown to detect anything. Controls live outside the
   scanned corpus, or are neutralised in the committed tree and injected during
   the check, so they never themselves become violations.

- **M1 No product surface.** Mechanical: a scan finds no Gas City reference in
  `flake.nix`, `nixos-modules/`, `packages/`, `docs/reference/`,
  `docs/how-to/`, `docs/explanation/`, `README.md`, or
  `docs/contributing/critical-subsystems.md`, and no `d2b.*` option, manifest
  field, or schema property whose name or description mentions it. Mentions
  outside those surfaces are permitted, so `docs/adr/`, `docs/contributing/`,
  `specs/`, `.d2b-orchestration.toml`, `changelog.d/`, and `CHANGELOG.md` are
  not scanned for presence.

  Coverage and controls: the scan reports the file count per scanned surface
  and fails if any named surface contributed zero files, which catches a
  renamed or relocated directory. Planted controls place a Gas City reference
  in each of a Nix option description, a flake output attribute, a
  `docs/reference/` page, and `README.md`, and each must be rejected.

  Reviewer judgement, stated as such: a `CHANGELOG.md` or `changelog.d/`
  mention must describe a contributor-process decision and must not present Gas
  City as a d2b capability. No scanner can distinguish those two sentences, so
  this half is a review rule that the follow-on contributor documentation
  carries, and pretending otherwise would be the kind of unevaluable stopping
  condition this repository rejects.

- **M2 d2b is never contacted, whether or not d2b is running.** The contributor
  will be running d2b while working on d2b, so this condition proves
  independence rather than absence, and it must not require `d2bd` to be
  stopped or any VM to be down. Three parts, all with d2b running normally:

  1. **Configuration scan.** The city configuration, every pack the deployment
     imports, and the whole formula set contain no reference to `d2bd`, the
     `d2b` binary, `d2b-priv-broker`, `/run/d2b/public.sock`,
     `/run/d2b/priv.sock`, `/run/d2b/unsafe-local-helper.sock`, the
     guest-control transport, or any d2b service or unit name.
  2. **Execution trace.** A full workflow run under a syscall or filesystem
     trace shows no `connect` to any socket under `/run/d2b/`, no `execve` of
     `d2b`, `d2bd`, or `d2b-priv-broker`, and no read of d2b service state.
  3. **Controlled denial.** The same workflow completes with the worker
     identity denied access to `/run/d2b/` and with no d2b binary on its
     `PATH`. A dependency that the trace missed fails here.

  Coverage and controls: part 1 reports the count of configuration files,
  packs, and formulas scanned and fails on an empty set, which catches a pack
  set that failed to resolve and was silently scanned as nothing. Part 2
  reports the count of traced syscalls and fails if the trace captured none,
  which catches a tracer that attached to the wrong process or exited early.
  Planted controls add a formula step that invokes `d2b vm list` and one that
  connects to `/run/d2b/public.sock`; part 1 must reject the first and part 2
  must reject the second.

  Done when all three pass, with coverage asserted and controls rejected, on a
  host with d2b up and at least one VM running.

- **M3 The standalone contributor surface is unaffected and self-sufficient.**
  With Gas City absent from the host, `node scripts/copilot/check-bindings.mjs`,
  `bash tests/unit/meta/adr-index-coverage.sh`, and `make check-tier0` pass;
  every skill and agent under `.github/skills/` and `.github/agents/` runs to
  completion without the external configuration repository, the `gc` binary, or
  any Gas City service; and no file under those two directories references Gas
  City, `d2b-gascity-configs`, `.d2b-orchestration.toml`, or the approval store.

  Coverage and controls: the reference scan reports how many agent and skill
  files it read and fails if either directory contributed zero, which is the
  failure mode if the surface is ever relocated. A planted control adds a Gas
  City dependency to one agent file and one skill file, and both must be
  rejected.

  This deliberately does not freeze bytes. Those directories will change for
  unrelated contributor-process reasons, and an equality assertion against a
  historical tree would either be edited away on its first legitimate failure
  or block work it was never meant to govern. What must hold forever is
  independence, and independence is what is asserted.

- **M4 A run survives a restart.** A workflow paused at a gate resumes at the
  same node after the orchestrator process is killed and restarted, with no
  duplicated or skipped step.
- **M5 Durable artifact-bound approval, proven on the deny path.** A spec
  revision is approved from Discord, the approval survives a restart of both
  the orchestrator and the approval store, and editing one byte of the artifact
  causes the next gate evaluation to **deny** with the remediation named. Done
  when the deny path is exercised by a test, not only the approve path.
- **M6 Discord closes a durable gate.** A clarification question reaches
  Discord, the answer returns into the same planner context, and a gate closes
  as a result. Whether upstream Discord support exposes buttons and modals or
  only slash commands plus signature-verified webhook interactions is **not
  proven** at the measured commit and is this item's first question.
- **M7 Task evidence is present for every task shape.** Every completed task
  record carries a non-empty pre-change failure proof and post-change
  validation naming a command or observation, including tasks with no unit
  test. Done when a task whose record leaves either field empty, or fills it
  with an assertion rather than a command or observation, fails the stage.
- **M8 Simplification reverts only itself.** A simplifier changes an
  implemented task, validation fails, and only the simplifier's change is
  reverted. Done when a test asserts the post-revert tree equals the
  pre-simplification tree byte for byte.
- **M9 tasks.md import fidelity, with actionable failures.** One real
  `tasks.md` from `specs/` imports into a validated DAG preserving IDs,
  parallel markers and dependencies, at least two independent tasks run
  concurrently, and a re-import after an unrelated edit creates no duplicate
  task. Done when four deliberately malformed lines, each breaking a different
  required element (the checkbox, the task identifier, the marker shape, the
  phase header), are each rejected with a message naming **which** expectation
  failed and what was found there, in addition to quoting the line. A rejection
  that only quotes the line fails this item.
- **M10 The panel bridge is proven or the stage parks with a named resume.**
  Either a Gas City run causes the existing d2b panel skill and `xtask delivery
  wave panel-request | panel-attest | seal` to run unchanged and drive a wave
  to sealed, with a deliberately mis-bound lane **rejected** by `panel-attest`;
  or the bridge is shown not to work and the stage parks on a durable gate
  whose message names the concrete resume command with its bead id
  substituted. In the parked case, closing the gate is refused while no
  matching `panel-attest` record exists, and succeeds once one does. Done when
  one of the two paths is demonstrated end to end, the other is not silently
  substituted, and the refuse-then-succeed pair is observed in the parked case.
  A run that reports a panel outcome without a corresponding `panel-attest`
  record fails this item.
- **M11 The publishing credential is mechanically unreachable from the
  worker.** From the worker identity, with a full agent session running: no
  GitHub token is present in the environment; no readable file, git credential
  helper, `~/.config/gh`, or push-authorised SSH key yields one; the publisher's
  credential material is unreadable including anything under a per-unit
  credentials directory belonging to another principal; and a direct `git push`
  and a direct pull-request creation against `vicondoa/d2b` both **fail for
  lack of a credential**. Done when each of those is attempted and observed to
  fail, not when a policy document says it should.

  This is the one MVP condition that is not negotiable on schedule grounds. The
  helper protocol of D14 may be prototyped, revised, or replaced; the
  non-reachability property it exists to provide may not ship unproven, because
  without it D16's lint is the only thing standing between an agent and the
  remote.

- **M12 Publication requires an approval bound to an immutable commit.** The
  run stops before push. The publisher rejects each of: a commit with no
  approval record; an approval whose recorded commit hash differs from the
  commit presented; an approval recorded against a gate node other than the
  publication gate; an approval whose decision is not `approve`; and **any
  mutable reference offered in place of a commit hash**, including a branch
  name, a tag name, `HEAD`, and a symbolic ref. The formula-set lint of D16
  rejects a formula that references `push-branch` or `create-pr` outside the
  publisher step. Done when each of the five publisher rejections and the lint
  rejection is individually observed.

- **M13 Discord ingress is allowlisted, fails closed, and diagnoses.** A
  message from any Discord user id other than the configured operator, and
  where a guild is configured a message from any other guild or channel,
  produces no approval, no response, no clarification answer, no gate
  transition, and no workflow state change of any kind, and is rejected **at
  ingress** rather than filtered downstream. Done when four things are
  observed: a foreign-author message rejected before any workflow state is
  touched; a message from the configured operator in a non-configured channel
  likewise rejected; an unset or empty allowlist rejecting the configured
  operator too, proving the default is closed rather than open; and each of
  those rejections emitting exactly one local diagnostic naming its rejection
  class and carrying the received and configured identity digests, with no raw
  Discord id present in the diagnostic.

- **M14 The dashboard refuses remote reach and local mutation.** Two parts,
  both required, because they defend against different principals.

  1. **Non-loopback refusal.** The listening socket is bound to a loopback
     address, and a connection attempt from any non-loopback address of the
     host is refused. Observed on a host that has at least one non-loopback
     address.
  2. **Mutation refusal from loopback.** Connecting **from loopback**, as a
     local agent would, every mutation endpoint refuses to change state:
     the session respond path, any approve or deny control, sling, bead
     update, and bead close. Each returns a refusal and leaves orchestrator
     state unchanged, verified by reading the state back rather than by
     trusting the response code alone.

  Part 2 is the one that matters for agents. A deployment that passes part 1
  and fails part 2 has no dashboard control at all, because every agent it runs
  is already inside loopback.

- **M15 The worker cannot originate or forge an approval.** Running as the
  worker identity, with a full agent session active: every write path to the
  durable approval store is attempted and observed to fail, including direct
  file creation and modification, replacement of an existing record, deletion,
  and any socket or helper the deployment exposes to the worker. Then the
  end-to-end case: a run that fabricates an approval for a commit it controls
  and invokes the publisher is **refused by the publisher**, which reads the
  store itself and finds no matching record.

  Done when both the write attempts and the fabricated-publication attempt are
  observed failing. Read access from the worker is expected to succeed and is
  not a finding.

  M11 and M15 are a pair and neither is sufficient alone. M11 without M15
  leaves an agent able to manufacture the precondition and publish through the
  sanctioned path; M15 without M11 leaves an agent able to publish directly.

- **M16 The worker cannot read or attach to orchestrator state.** Running as
  the worker identity: `ptrace` attachment to an orchestrator process fails;
  the orchestrator's `/proc/<pid>` memory, environment, and file-descriptor
  entries are unreadable; the orchestrator's durable state directory and the
  protected integration object store are not writable; and a write attempt
  against an existing integration object fails. The worker's own socket to the
  orchestrator accepts only the closed request set and rejects a connection
  whose peer uid is not the expected one, observed by connecting from a third
  identity. Done when each is attempted and observed to fail, and the
  legitimate request set is observed to succeed.

- **M17 Worker egress is default-deny with the required paths open.** From
  inside the worker's network namespace, with planted targets on both sides:
  every d2b bridge address, a d2b guest address, a host LAN address, a
  link-local address including `169.254.169.254`, and the host's loopback
  dashboard and orchestrator ports are each unreachable; and the configured
  model endpoint, the configured substituter or package registry, and the
  configured DNS resolver are each reachable. Done when every denied control is
  observed refused and every allowed control is observed to succeed.

  Both halves are required. A namespace that denies everything passes a
  deny-only check while making the workflow unusable, and a namespace that
  allows everything passes an allow-only check while providing no isolation.

  **Confinement lifetime.** A third part covers the teardown race of D15. An
  agent is started that deliberately ignores its stop request and stays alive
  for the whole of D5's grace period. Throughout that interval, and up to the
  instant it is forcibly terminated, it is re-tested against the denied set
  above and must still be unable to reach any d2b bridge, guest address, LAN
  address, link-local address, or host loopback port. The namespace, its
  interface, its DNS proxy, and its firewall rules are then observed to be torn
  down **only after** that process has exited, verified by ordering the
  teardown events against the process exit rather than by a timing assumption.
  A run in which teardown is observed before the last agent exit fails this
  item outright.

- **M18 Approval and audit history is append-only, kind-bound, authenticated,
  atomically installed, path-contained, singly-owned, and bounded to stated
  defaults.** Twelve parts.

  1. **Immutability.** Attempts to modify history fail from every principal
     that is not the append helper, including `approval-controller`,
     `publisher`, and the retention timer: overwriting an existing record,
     truncating the store, and deleting an individual record are each refused.
     Re-appending identical content is not a modification and is covered by
     part 5.
  2. **Caller and kind authorization.** The helper's socket enforces the D12
     matrix over all **fifteen** caller-and-kind combinations: three accepts,
     one per authorised pair; six cross-kind rejections, being each of those
     three callers offering each of the two kinds it does not own; and six
     categorical rejections, being `agent-worker` and `orchestrator` each
     offering all three kinds. Every combination is exercised; none is inferred
     from another. A caller offering the helper-only `seal` kind is also
     rejected.
  3. **Kind binding.** The negative control is a `publisher` connection that
     submits a request whose fields are an approval's fields. It is rejected by
     the schema for its authorised kind, and afterwards **no file whose name
     begins `approval-` exists that the publisher caused**. The same is checked
     in reverse for `approval-controller` against `publication-attempt`. A test
     that only asserts the request was rejected does not satisfy this part; the
     store is enumerated afterwards.
  4. **Request schema and bounds.** A request carrying any field outside the
     schema of its authorised kind is rejected rather than ignored, verified
     with added `path`, `name`, `filename`, `period`, and `kind`-override
     fields. A canonical envelope of exactly 4096 bytes is accepted and one of
     4097 is rejected, so the bound is tested at its edge rather than in the
     middle.
  5. **Append atomicity, durability, and idempotent retry.** A reader
     concurrent with an append never observes a partial record. Injected
     crashes are exercised at four points: after the temporary is created;
     after it is written and `fsync`ed; after the rename but before the
     directory `fsync`; and after the directory `fsync` but before the
     acknowledgement reaches the caller. The first two leave no visible record.
     The third leaves a complete, readable record. The fourth is retried by the
     caller and returns **idempotent success**, and the retry is observed to
     `fsync` the period directory before acknowledging, so the record is
     durable afterwards even though the original crash preceded that sync.

     Corruption is distinguished from idempotency: with a file planted under a
     valid record name whose bytes do not hash to it, the retry **fails closed**
     and reports corruption, naming `gascity-audit verify --record` in the
     error, and does not overwrite or repair. Short writes and `EINTR` are
     exercised by injection and must complete the record rather than truncate
     it. An acknowledged append survives immediate power loss.
  6. **Temporary hygiene and budget.** Every abandoned path unlinks its
     temporary: failed write, failed `fsync`, failed rename, post-creation
     validation failure, and the `EEXIST` idempotent path, where the unlink is
     observed to happen **before** verification returns. Ten thousand
     consecutive colliding retries of the same record leave the temporary count
     and temporary bytes unchanged from their starting values. Temporary bytes
     are counted against both the dedicated temporary budget and the total
     store budget, verified by observing both counters move during an in-flight
     append; exceeding the temporary budget fails the append rather than
     evicting anything.
  7. **Path containment.** Two layers, because the derived-name path is pure
     hexadecimal and an end-to-end test alone would pass without ever
     exercising resolution.

     Unit: the internal component resolver is called directly with `..`, an
     absolute component, a component containing a separator, a symlinked
     component, and a magic-link component, and must refuse each. Each case
     asserts that resolution was attempted and refused, so a test cannot pass
     merely because no caller can supply a name.

     Integration: only physically possible controls are planted, since the
     request carries no filename. A symlinked store root, a symlinked period
     directory, and a symlinked record name are each planted, and each must be
     refused with no file created outside the store.
  8. **Descriptor hygiene.** Every descriptor is close-on-exec from creation,
     verified by inspecting `/proc/<pid>/fdinfo` flags for the lock, the store
     root, `periods/`, `quarantine/`, the current period, the listening socket,
     and an accepted connection, and by confirming a spawned child inherits
     none. With a required close-on-exec variant made unavailable, startup
     **fails** rather than falling back to a post-creation `fcntl`.
  9. **Single ownership and adoption.** With one helper running and holding the
     lock, a second instance started against the same store **fails to start or
     parks** with the typed conflict error, and is observed to have unlinked
     nothing, appended nothing, and opened no period for writing: an in-flight
     temporary belonging to the first instance is still present and intact
     afterwards. After the first exits and the lock is released, a new instance
     adopts the store and reconciles per temporary: a temporary whose final
     record exists is verified and unlinked; a temporary with no final record,
     and one that is short or does not parse, are removed as uncommitted
     fragments. Reconciliation appends no record. There is no path in which
     cleanup precedes lock acquisition.
  10. **Publication audit completeness.** Every publication attempt, success
      and failure alike, produces exactly one `publication-attempt` record,
      verified by counting records across a successful publication, a
      publication refused for a missing approval, and a publication refused for
      a ref rather than a hash.
  11. **Redaction.** No durable record, log line, error message, or operator
      output contains a raw Discord id, a raw run handle, a credential, a
      remote URL carrying authentication, or a **quarantine directory token**.
      Quarantined periods appear only as a period date plus a keyed correlation
      id. Verified by scanning the store, the log stream, and the text of a
      `retention-audit-append-failed` error with counted coverage and a planted
      control.
  12. **Sealing, verification, and retention.** Sealing writes the `seal`
      manifest as the period's last record and changes no other bytes.
      `gascity-audit verify --period` detects a **physically planted** missing
      record, by removing one record file from a sealed period out of band and
      observing the manifest comparison report it; it also detects an extra
      file not in the manifest. `gascity-audit verify --record` detects a
      digest mismatch.

      Event logs are trimmed at whichever of 14 days, 512 MiB aggregate, or the
      64 MiB per-file cap binds first. Approval and publication audit periods
      older than the 365-day floor are removed as **whole sealed periods**,
      never in place: renamed from `periods/` into `quarantine/` with
      `RENAME_NOREPLACE`, **both** the `periods` and `quarantine` descriptors
      `fsync`ed, the `retention-deletion` record appended, and only then the
      contents removed. The two-descriptor sync is asserted directly, since
      syncing one or syncing a common parent is a different and insufficient
      operation.

      Injected crashes are exercised at four points and the invariant checked
      after each: before the rename, leaving the original sealed directory
      intact and visible; after the rename but before either `fsync`, which is
      the exactly-one-location case above; after both `fsync` calls but before
      the record is appended, which recovery finishes by appending the record
      and then removing; and after the record but before removal completes,
      which recovery finishes by removing **without appending a second
      record**, since the record is already present and stage 2 of the recovery
      rule runs unconditionally on that basis. At no point does a
      `retention-deletion` record exist for a period still visible in
      `periods/`, and at no point is a visible sealed period observed to fail
      `verify --period`.

      The already-audited case is asserted on its own, not only as a crash
      outcome: a quarantine whose `retention-deletion` record is already
      present at the start of a recovery pass is observed to be removed
      completely, with its empty period directory unlinked and its shard
      `fsync`ed, and with no duplicate record appended. A pass that leaves such
      a quarantine in place fails this item.

      Failure handling is tested directly: with the append helper made
      unavailable, a due deletion renames into quarantine, fails to append,
      **retains the quarantine for retry**, and emits
      `retention-audit-append-failed` naming the period date, correlation id,
      append error, and remediation. `gascity-audit status` then lists that
      period; `gascity-audit converge` run while the helper is still broken
      **does not** remove it and does not skip the append; after the helper is
      repaired, `converge` completes the sequence, the record appears, and the
      quarantine clears. The timer is also observed to refuse touching the
      current unsealed period.

      Mount and link-limit assertions, each producing its own distinct error
      rather than a shared one:

      - With `periods/` and a quarantine shard on **different filesystems**,
        startup refuses with `audit-store-cross-filesystem`.
      - With them on the **same device but different mounts**, arranged with a
        bind mount inside the store, startup refuses with
        `audit-store-cross-mount`. This case is the reason the check is two
        layered: a device-only comparison passes it, so a test suite that omits
        it would certify a deployment that fails at its first retention run.
      - A later `EXDEV` at retention time maps to whichever of those two
        conditions holds and **parks** retention with that error. Neither case
        ever falls back to a copy, verified by observing that the source period
        remains present and unmodified and that no partial copy exists at the
        destination.
      - A destination `EMLINK` is driven by exhausting a shard's link count,
        and the rename is observed to **succeed into the next shard** after
        revalidating that shard's device and mount identifier, with both the
        `periods` descriptor and the receiving shard synced.
      - **The parent sync is proven by a crash after the rename, not before
        it.** When a new shard must be created, `quarantine/` is observed
        `fsync`ed before the period is moved into it. The injection that
        establishes why is placed **after the period has been renamed into the
        newly created shard and before the later source and destination
        directory syncs**.

        At that point neither directory has been made durable, so the rename
        may or may not have reached disk, and both outcomes are legitimate. The
        assertion is therefore not that the period ends up in quarantine. It is
        that after recovery the period is durably reachable from **exactly one**
        valid location: either its original `periods/<date>`, if the rename did
        not persist, or the durable quarantine shard, if it did. Never neither,
        never both, and never an orphan reachable from no tracked parent.

        The parent sync is what makes the second branch safe. If the rename
        persisted, the shard that now contains the period is itself durable,
        because its directory entry in `quarantine/` was synced before anything
        moved into it. Remove that sync and the same injection admits the
        forbidden outcome: the rename persists, the shard entry does not, and
        the period is reachable from neither `periods/` nor `quarantine/`. That
        is the failure this assertion exists to detect, and it is invisible to
        any test that merely checks the period is somewhere.

        A crash injected **before** the rename is a control only. It leaves the
        period in `periods/` untouched, which is a correct outcome under every
        variant of this code including one with no parent sync at all, so it
        demonstrates nothing about the sync and must not be reported as
        covering it. Both injections are run, and only the post-rename one is
        credited with the property.

      - With every shard in the bounded set exhausted, the helper refuses with
        `audit-quarantine-link-limit` naming the period date and correlation
        digest and pointing at `gascity-audit status` and `converge`, and is
        observed to delete nothing to make room.
      - **Link count reaches steady state rather than climbing.** Retention is
        run repeatedly over many simulated days, and the link count of each
        shard is sampled throughout: it does not increase monotonically,
        because each audited quarantine's now-empty period directory is
        unlinked and its shard `fsync`ed at step 6. Then the reclamation path
        is exercised directly: with the append helper broken, quarantines
        accumulate and a shard approaches its limit; after the helper is
        repaired, `gascity-audit converge` is observed to process the audited
        outstanding quarantines, remove their directories, and **reduce** the
        shard's link count, with no operator ever unlinking anything by hand.

      With `renameat2` unavailable or `RENAME_NOREPLACE` unsupported, startup
      refuses with a typed error rather than selecting an alternative install
      path. Where the 256 MiB audit cap would require deleting a period younger
      than the floor, deletion is **refused** and the condition reported.

      Quarantine naming: a quarantined directory's physical name is observed to
      be its canonical UTC date plus an opaque token, and its seal manifest is
      observed to carry the same date, so recovery can identify a period
      without reading its contents. Separately, no log line, audit record,
      error, or operator output contains the token or a path, verified
      alongside the redaction scan of part 11.

- **M19 The deployed `gc` resolves from the locked package, and no other `gc`
  is reachable.** Four parts, all on the running deployment.

  1. **Provenance.** The `gc` binary the service executes resolves to a Nix
     store path produced by the `gascity` package of the
     `llm-agents.nix` revision recorded in the configuration repository's
     `flake.lock`, verified by comparing the resolved store path against the
     path that revision evaluates to rather than by comparing version strings.
  2. **Version agreement.** That binary's `gc version` output satisfies
     `gascity_compat` in `.d2b-orchestration.toml`, and the check is performed
     against the binary's own output rather than against the flake attribute or
     the module's intent.
  3. **No alternate binary.** The unit's `ExecStart` names that store path, and
     the `PATH` of every principal that can invoke `gc` contains no other `gc`:
     none in `/usr/local/bin`, none in a user profile, none in a development
     shell left on the path, and none earlier in `PATH` than the intended one.
     Verified by resolving `gc` from each principal's environment and asserting
     a single candidate equal to the expected store path.
  4. **No manual dependency restatement.** The configuration repository's
     module does not add `beads`, `dolt`, `flock`, `gitMinimal`, `jq`, `lsof`,
     `procps`, or `tmux` to the service `PATH` itself; the wrapped binary
     supplies them. A build that drops the wrapper is caught by part 1 rather
     than masked by a duplicated list.

     Coverage and control, per the standing requirement: the scan reports the
     number of module files and service definitions it examined, asserts each
     is greater than zero, and fails closed on an empty set, which catches the
     module having been relocated. A planted control injects one of those eight
     package names into a service `PATH` in a scratch copy of the module and
     must be rejected, so the scan is shown to detect rather than merely to
     return clean.


- **M20 Startup and shutdown are exact reverses of one ordering chain.** Three
  parts, all traced.

  1. **Startup order, including the chain edge.** The five units become ready
     in the order of D5's table: append helper, worker netns, dashboard,
     orchestrator, Discord ingress. The helper is observed to hold the store
     lock before any other unit starts; the netns unit is observed to exit zero
     only after the namespace, interface, resolver and deny-by-default rules
     are all verified, not merely after it ran; the dashboard is observed
     serving in backend-not-ready mode while the orchestrator is still
     starting; the orchestrator adopts state before becoming ready; and an
     interaction arriving before ingress opens is refused rather than queued
     into a run still being recovered. The trace shows no unlink, no
     reconciliation, and no period open for writing preceding lock acquisition.
     Together with M18 part 9's overlap case, this closes the window in which a
     second instance could clean up a first instance's state.

     The dashboard-to-orchestrator edge is asserted specifically, by reading
     the resolved unit properties rather than by inferring it from one observed
     boot: `gascity-orchestrator.service` lists both
     `gascity-dashboard.service` and `gascity-worker-netns.service` in `After=`
     and only the netns in `Requires=`. A single successful boot proves nothing
     here, because two units that are merely siblings under a common `After=`
     can happen to start in the desired order and then stop in the wrong one.

  2. **Spawn is gated on proven readiness, not on unit ordering.** With the
     orchestrator running and ready, the worker netns unit is stopped or its
     namespace removed out of band, and a spawn is then requested: it is
     **refused**. This is the case unit ordering cannot cover, since ordering
     described only the initial start, and it is the case in which an
     unconfined agent would otherwise appear.

  3. **Shutdown is the reverse.** A full stop is traced and the units are
     observed stopping in exactly the reverse of the order they started:
     ingress, orchestrator, dashboard, netns, append helper. The reversal is
     asserted as a property of the recorded start and stop sequences rather
     than by matching against a second hand-written list, so a future unit
     inserted into the chain is covered without editing this item.

     Within that reversal: an interaction injected after shutdown begins and
     before any drain step is **actively refused**, with the trace showing the
     refusal preceding the first park or drain event, and an interaction that
     is merely dropped, queued, or accepted-then-discarded fails this item; an
     in-flight task either completes or parks such that a subsequent start
     resumes rather than restarts it; an agent ignoring its stop request is
     terminated at the configured bound; the namespace is torn down only after
     that termination, per M17's confinement-lifetime part; the dashboard is
     observed still serving reads during the drain and stopping only after
     state settles; no append is left acknowledged but unsynced and no
     temporary survives a clean shutdown; and the lock is released after every
     writer has exited.

  **The trace itself must be shown to work.** These parts assert the *absence*
  of events in an interval, and a tracer that attached late, filtered wrongly,
  or captured nothing satisfies an absence assertion trivially. Each run
  therefore includes a planted, detectable control event of the same class it
  is asserting about: a deliberate unlink of a scratch file inside the traced
  interval, which the trace must capture. A run whose trace does not contain
  the planted event is a failed run, regardless of what else it showed.

  All three parts run against the generic module, not against d2b-specific
  configuration, since D5 assigns this behaviour to the module layer and it
  must hold for any consumer.

- **M21 `gascity.nix` is generic, proven by scan rather than by review.** The
  generic module repository is checked in its own CI, and the check is subject
  to the standing coverage and planted-control requirements above.

  1. **No d2b identifier anywhere.** A case-insensitive scan of the entire
     tracked tree, including `flake.nix`, `flake.lock`, module files, helper
     packages, tests, and documentation, finds no `d2b` identifier. The match
     is at identifier boundaries rather than raw substring, and content-address
     literals are excluded: `narHash`, `sha256`, `hash` and `vendorHash` values
     and store-path components are opaque digests in which the three characters
     can occur by chance, and a scan that fails on one of those will be
     disabled the first time it does. Excluding them is what keeps the check
     alive.
  2. **No d2b dependency.** No flake input, overlay, or package reference
     resolves to `vicondoa/d2b` or any repository under it, verified against
     the resolved `flake.lock` rather than the input names in `flake.nix`, so
     an input aliased to an innocuous name is still caught.
  3. **Option namespace is Gas City-specific.** Every option the module
     declares sits beneath `services.gascity`, and no declared option name,
     description, example, or default contains a d2b identifier. Verified by
     evaluating the module's option set and inspecting the declarations, not by
     grepping the source, so an option assembled from string fragments is
     still checked.
  4. **Counted coverage.** The scan reports the number of files, flake inputs,
     and declared options it examined, asserts each is greater than zero, and
     fails closed on an empty set. A repository that scanned nothing passes
     nothing.
  5. **Planted controls.** Four violations are planted and each must be
     rejected: an option named with a d2b identifier; an option whose default
     is a d2b path; a flake input resolving to a d2b repository under a
     neutral alias; and a d2b string inside a systemd unit definition. A scan
     that cannot demonstrate these four detections has not been shown to detect
     anything.

  This condition belongs to the generic repository and runs there. It is listed
  in this record because the split D4 makes is only worth its cost if the
  generic layer stays generic, and that property degrades silently: one d2b
  path added under deadline is invisible in review and permanent in practice.

## Consequences

**d2b gains nothing, and that is the intended outcome.** A consumer's
experience of the framework is unchanged in every particular: same options,
same manifest, same CLI, same docs, same changelog. The only visible trace in
this repository is an ADR, contributor documentation, and a four-key dotfile.
The cost of that discipline is that Gas City gets none of d2b's guarantees
either. It carries no compatibility promise, no deprecation policy, and no
support obligation, and a contributor who builds a habit on it is depending on
something the project has not promised to keep.

**Contributor infrastructure has a way of becoming a product surface.** The
realistic failure is incremental: a `d2b.*` option "just for the service user",
a `checks` output "so CI can run it", a `docs/how-to/` page "since people
ask". M1 exists precisely because that drift is invisible in any single commit
and obvious only in aggregate.

**Five principals and a network namespace are more operational surface than a
contributor tool usually carries.** That is the honest cost of running
autonomous agents on a workstation that also hosts d2b development. Four of the
principals are `users.users` entries with units and the fifth is a root-owned
helper service, the namespace is a handful of firewall rules, and none of it
needs d2b, but it is five services to keep running and a firewall policy that
will block something legitimate the first time a task needs an endpoint nobody
listed. The alternative is not "less configuration"; it is a design in which an
agent reaches the operator's LAN, the d2b bridges, and the publishing
credential, and where the only thing between it and `v3` is a prompt.

**The retention defaults are a guess, and the audit floor is the one that will
be wrong first.** Fourteen days and 512 MiB for event logs, and a 365-day floor
under a 256 MiB backstop for approval and publication audit, are conservative
workstation values chosen without production data. Event logs are the volatile
side and will simply trim earlier than expected on a busy fortnight. The audit
side is the side to watch: it is deliberately configured so that a full store
**refuses to delete** rather than silently discarding history, which converts a
disk-budget problem into an operator-visible failure. That is the correct
direction for an audit trail and it does mean a misconfigured deployment can
wedge on storage rather than quietly losing evidence. Both sets of values live
only in the external deployment repository, so tuning them never touches d2b.

**The controls now assume an agent is hostile, while the workflow assumes it is
useful.** That tension is deliberate and unresolved. An agent gets a model
credential, a writable worktree, and outbound access to model and package
endpoints, because it cannot do the job otherwise, and everything else is
denied. A compromised agent can still produce a plausible bad change, argue for
it in a task record, and have it reach the panel. What it cannot do is publish
that change, reach d2b's networks, read the orchestrator's memory, or rewrite
the record of what a human approved.

**The worker identity is not isolated, and D17 says so rather than implying
otherwise.** Agents run as `agent-worker` with its privileges and reach the
model credential and their own worktrees. A prompt-injected or malfunctioning
agent can do anything that identity can do inside its namespace, and the
workflow shape does not prevent it. What it cannot do rests on separations
rather than rules: no publishing credential, no ability to originate an
approval, no ability to move the bytes after approval, no reach into
orchestrator state, and no route to d2b or the LAN. Everything else is scope,
gating, and review, which reduce the chance of a bad change reaching `v3` and
do not contain a bad agent. Anyone deploying this should size `agent-worker`
accordingly.

**"Local" is not a trust boundary, and the dashboard is where that bites.**
The single-user scope makes it tempting to treat loopback as a perimeter, and
for humans it is one. For agents it is not: the upstream dashboard exposes a
respond path that answers an agent's own permission prompts. Read-only mode and
the worker's separate network namespace are therefore both requirements rather
than hardening options, and the cost is real: run observation is all the
dashboard provides, so anything an operator might have done through its
controls now happens through Discord or a shell. That is the correct trade, but
it is a trade.

**Digested identifiers make forensics harder.** Storing keyed digests instead
of raw Discord ids and run handles means an operator reading the audit trail
cannot see who or what at a glance, and correlating a record with a Discord
message requires computing the digest. That is accepted because the durable
store outlives the incident that motivated it and accumulates identifiers
forever, but it is a genuine loss of convenience and the deployment
documentation has to explain how to compute the digest for a known id.

**D2 closes the obvious escape hatch, on purpose, and that has a price.** The
project that would most naturally supply a sandbox here is the project this
tooling is used to develop, and this record forbids using it. The cost is that
the isolation gap above has no in-scope remedy: closing it requires a new ADR
and new work rather than a configuration change. The benefit is that a broken
`d2bd`, a mid-flight provider refactor, or a d2b change that breaks VM
lifecycle cannot take the contributor workflow down with it. Development
tooling that cannot survive a regression in the thing being developed is not
development tooling.

**Two contributor workflows now exist and both must be maintained.** A
contributor who does not opt in uses `/speckit-*` plus `/d2b-autopilot` exactly
as today, and D3 keeps that path first-class and sufficient. Every process
change has to be considered against both, and the failure mode is a rule that
lands in one and silently does not hold in the other. D3's guard catches
deletion and conditioning, not divergence of intent.

**Deployment is split across four layers, and two of them are repositories
that do not exist yet.** `numtide/llm-agents.nix` supplies the package,
`vicondoa/gascity.nix` will supply the generic module, `d2b-gascity-configs`
will supply d2b policy, and `/etc/nixos` instantiates. That is three pins to
keep aligned and two repositories to create, review, and maintain before any of
this runs. The honest cost of the extra layer is that a change spanning generic
machinery and d2b policy now spans two repositories and two reviews, and a
version skew between them is a new failure mode that did not exist when the
module and the policy were one file.

What buys that back is testability and blast radius. The generic layer can have
NixOS VM tests for shutdown ordering, restart adoption, lock contention and
namespace denial that need no d2b checkout, no Discord application and no pack
set; and a defect in d2b workflow policy cannot reach the principal split or
the append store, because they are not in the same repository. Given that layer
2 is where the security machinery lives, that separation is worth one more pin.

`/etc/nixos` remains outside version control here, so a working deployment
cannot be reproduced from the shared repositories alone. That is the correct
place for machine-specific and secret-adjacent configuration, but it means the
configuration repository must document what `/etc/nixos` is expected to
provide, and a missing expectation shows up as a runtime failure rather than an
evaluation error.

**The generic module is a commitment to a second audience.** Writing
`gascity.nix` as a reusable module means someone other than this deployment may
use it, which is the point but is also an obligation: options become a surface,
defaults become a contract, and a change that suits d2b but breaks a
hypothetical consumer is now a real consideration. If it turns out nobody else
ever uses it, the split will have cost a repository to buy testing isolation
alone. That is still a reasonable trade for the machinery in D5, D12, D14 and
D15, but it should be recorded as a bet rather than a certainty.

**Reviews get slower and more expensive, on purpose.** An opted-in run carries
per-task spec and quality reviews, an optional 17-lane compound filter, and the
binding ten-seat panel, on top of validation. The ten seats are already a
deliberate cost recorded in ADR 0048; this record adds two review layers in
front of them and removes nothing.

**Operational dependence on Discord and a temporary dashboard.** A Discord
outage means the human surface is gone and runs park at gates. The dashboard
repository describes itself as a temporary workspace and has no auth model, so
D13 confines it to loopback observation; when it folds back into `gc`, its
approval semantics must be re-measured before D13 can be relaxed.

**Single-user scope is cheap now and will be the expensive thing to change.**
D6 removes a permissions matrix, a role model, a tenancy story, and a dashboard
authentication layer, none of which this deployment needs. The bill arrives if
a second person ever needs access, because there is then no authentication
surface to extend and no notion of an actor other than the configured operator
anywhere in the approval records. That is a rewrite of the human-identity half
of the design, not a configuration change, and it deserves its own record. The
trade is accepted because building multi-party access control for one person is
speculative work whose only certain outcome is more code to keep correct.

The half that does **not** get cheaper is the agent boundary. D14's separation
costs the same for one operator as for ten, because agents are the thing it
constrains and the deployment runs exactly as many of them either way.

**The approval store is new security-relevant software outside d2b's gates.**
It is the authority for every human decision in the run and it lives in the
configuration repository, so it does not get d2b's panel, lints, or fail-closed
test corpus unless that repository builds equivalents. This is the largest
honest cost of D4 and it is accepted, because the alternative is landing
contributor tooling in a framework's product surface, which D1 forbids for
better reasons.

**The importer will drift.** `tasks.md` has no schema and no version and its
producer is a language model. D19 makes drift loud at import time rather than
silent at execution time and puts a human confirmation in the path, but it
cannot make the format stable.

**Rollback is cheap.** Every artifact this record authorizes lives outside this
repository except this file, the contributor documentation, and
`.d2b-orchestration.toml`. Abandoning Gas City means deleting three files from
d2b, archiving one repository, and reverting one import in `/etc/nixos`. That
asymmetry is the main reason D1 and D4 are shaped the way they are.

## Alternatives considered

**Treat Gas City as a d2b capability and ship it.** Rejected, and this is the
correction that produced this revision. Exposing it as a flake output, a
`d2b.*` option block, or a `docs/how-to/` page would put an orchestration
concern into the product surface of a desktop microVM framework, where it fails
the bar that every d2b user plausibly wants it and the framework cannot do the
right thing without it. It would also create obligations the project should not
take on: a compatibility promise, a deprecation policy, and a support surface
for a tool whose upstream released `1.4.0` nine days before this record and is
moving fast. Contributor tooling can be replaced next quarter; a shipped
capability cannot.

**Run agents in d2b microVMs, with a Gas City runtime provider, a declared VM
pool, and git-bundle artifact return.** Rejected and removed. Earlier drafts of
this record carried this design, first as the centrepiece and then as an
optional second part, and both were wrong for the same reason: it makes the
contributor environment depend on the artifact under development, so a
regression in d2b's VM lifecycle would block the workflow used to fix it. It
also put an unproven adapter, a NixOS module, a workspace transfer protocol, a
guest credential story, and a cleanup and orphan-recovery matrix on the path to
a change whose actual value is durable state and dependency-ordered execution.
Nothing about the isolation upgrade is argued against here on its merits; it is
simply a different decision, and D2 sends it to its own ADR where it can be
argued properly.

**Make Gas City the required contributor workflow.** Rejected. It would make a
fast-moving external dependency a precondition for contributing to d2b, and it
would strand anyone who cannot or will not run a Discord bot, a dashboard, and
an orchestrator daemon to change a Nix module. D3 keeps the standalone skills
first-class, and the two paths converge at the same panel and the same seal.

**Keep the whole NixOS module in `d2b-gascity-configs`, with no separate
`gascity.nix`.** Rejected, and this is the choice this revision makes rather
than defers. What the module has to contain is now substantial and almost
entirely generic: five principals and their privilege separation, a network
namespace with a default-deny firewall, an append-only store with an exclusive
lock and an atomic install path, rotation and retention with a quarantine
protocol, a read-only dashboard, and the ordered shutdown and adoption sequence
of D5. Fusing that with d2b's rig path, formulas, Discord identity and egress
allowlist produces one repository where a change to a firewall primitive and a
change to a workflow policy touch the same files and share a review.

The concrete cost is testability. Generic lifecycle behaviour wants NixOS VM
tests that assert shutdown ordering, restart adoption, lock contention, and
namespace denial, and those tests should not require a d2b checkout, a Discord
application, or a pack set to run. Splitting lets layer 2 be tested on its own
terms and lets layer 3 stay small enough to read as policy. The price is one
more repository and one more pin, which D4 accepts.

**Put the module in `llm-agents.nix` instead.** Not assumed, and not
requested. It supplies packages today, exposing per-system `packages` and
`overlays.shared-nixpkgs` with no `nixosModules` output, and it is upstream and
outside our control. Making this design depend on an upstream adding and then
maintaining a NixOS module with our principal split, our shutdown ordering, and
our append-only store would couple a security-relevant boundary to a repository
that describes itself as automatically updated daily. If that flake later grows
a module we want, adopting it is a decision to take then, on evidence.

**Keep the whole deployment in `d2b-gascity-configs`, with nothing in
`/etc/nixos`.** Rejected. Secret material, uid assignment, and network exposure
are properties of one machine, and a shared repository is the wrong authority
for them. The split costs reproducibility from the repositories alone, which D4
accepts and the configuration repository documents.

**Let Gas City run the ten-seat panel through `compound-engineering`.**
Rejected on measured grounds. The compound pack has 17 selector-gated lanes, no
per-lane model or effort record, no unanimity, and no seat identity, so it
cannot produce a record `ensure_panel_binding` and `panel-attest` would accept.
Relaxing the delivery constants to accommodate it would weaken the one gate in
this repository that has already caught findings a static gate missed.

**Use the dashboard's Approve and Deny as the approval mechanism.** Rejected.
The wire types carry `kind`, `prompt`, `options` and `request_id` and no
artifact identity, so an "approval" would bind to a session prompt rather than
to bytes. That is precisely the failure the operator asked to avoid, and it is
worse than Discord because it looks like an approval UI.

**Rely on Gas City's `gate` beads as the durable approval record.** Rejected.
Gate states are lifecycle states, not decisions, and there is no artifact
binding and no invalidation on change. The gate is kept as the blocking
transport, which is what it is good at, and the decision record is owned
elsewhere.

**Run on `k8s` or `herdr` for better isolation than a host worktree.**
Rejected for the first deployment. Both are real builtin runtimes, but each
adds an operational substrate the operator does not run today, and tmux remains
required regardless because it is the registered fallback. Adding
infrastructure to partially close the D17 gap is a decision that deserves its
own record alongside the sandbox question, not a quiet default here.

**Run everything as one identity and rely on the publication lint.** Rejected,
and this is one of the corrections that produced this revision. An earlier
draft put the GitHub push and pull-request credential in the same account that
runs uncontained agents, and then relied on a lint over the formula set to keep
publishing confined to one step. That is not a control: the agents that can
edit formulas are the agents the lint constrains, the credential is reachable
by any subprocess they spawn regardless of what the formula says, and the
design brief's own rule is that implementation agents do not hold publishing
authority. Splitting the identity costs one `users.users` entry and a helper,
and it converts the rule from prompt-level to mechanical. The lint survives as
defence in depth in D16, which is the right rank for it.

**Collapse the identities, since it is a single-user machine anyway.**
Rejected, and the reasoning that makes it tempting is the reasoning that makes
it wrong. D6's scope means one *human* uses this deployment; it says nothing
about how many autonomous agent processes run under it, which is the population
D14 constrains. The publishing credential and the approval-write authority are
withheld from agents, not from the operator, who retains both through the
controller and publisher identities. Merging them into the worker would trade
the design's only mechanical controls for the removal of two `users.users`
entries.

**Withhold the GitHub credential but let the worker write approvals.**
Rejected, and this was the defect in the previous form of D14. It looks like a
boundary and is not one: an agent that can write the approval store approves
its own revision and then calls the publisher through the sanctioned path,
which verifies an approval that exists and publishes. No credential leaks, no
lint fires, and the branch lands. Authority over a precondition is authority,
and the fix is to separate it with the same mechanism used for the credential
rather than a weaker one.

**Separate the roles by file mode within one uid, or by a wrapper script.**
Rejected. Same-uid separation is defeated by the trivial expedient of not using
the wrapper, and an agent that can run arbitrary commands as an identity has
every authority that identity holds regardless of how the paths are laid out.
It can also `ptrace` its same-uid peers and read their memory and file
descriptors, so even a well-behaved wrapper leaks what it was meant to guard.
The non-goals say this explicitly because it is the shape a hurried
implementation reaches for when more accounts feel like ceremony.

**Use `LoadCredential=` to confine the GitHub token to one unit while sharing a
uid.** Rejected on a factual correction. An earlier draft offered this as a
two-identity variant; it does not work. Credentials materialise in
`$CREDENTIALS_DIRECTORY` readable by the unit's own user, so any process
sharing that uid, including a network-facing ingress parser, can read them.
`LoadCredential` is a good delivery mechanism, keeps secrets out of the store
and out of the general filesystem, and this repository already uses it in
`nixos-modules/guest-control.nix:297`; it is not a cross-principal boundary and
this record no longer treats it as one.

**Let the publisher accept a branch or ref that the run has prepared.**
Rejected. A ref is a name whose target the worker can change, so approval
verification and push would consult two different sets of bytes with a window
between them, and nothing about the record afterwards would reveal it. The
publisher takes a commit hash, which is the bytes.

**Let the workflow supply the approval prompt text and artifact identity.**
Rejected. It makes the approval controller a confused deputy: benign prose is
shown to the operator while the approval binds to a hash of different content,
and the operator's genuine approval authorises something they never saw. The
controller reading the immutable object itself costs a little duplicated work
and removes the whole class.

**Run agents in the host network namespace, since the firewall is a
workstation concern.** Rejected. The host on which this runs is precisely the
host running d2b microVM bridges, so an agent in the host namespace can reach
the guest networks whose isolation d2b exists to provide. That would let
contributor tooling undercut the product from the outside without touching a
single d2b component, which is a worse version of the coupling D2 forbids.

**Rely on the dashboard's loopback bind instead of read-only mode.** Rejected.
Loopback is a control against remote humans and no control at all against local
agents, and the dashboard's respond endpoint is exactly the capability an agent
would want. Read-only mode is measured, is one environment variable, and is
enforced in the proxy rather than in the UI, so it holds against a caller that
never loads the frontend.

**Build multi-user access control now.** Rejected. A permissions matrix, a role
model, and a dashboard authentication layer would be written for a second
principal who does not exist, tested against a threat model nobody has stated,
and maintained by the one person they were supposed to protect against. D6
takes the scope as given and states the cost of reversing it in Consequences
rather than paying it up front. Note that this rejection is about **human**
principals only; the agent-principal controls in D13 and D14 are built now
precisely because those principals do exist today.

**Give the publisher the operator's general GitHub identity.** Rejected. It is
the one credential in the system with lasting consequences outside this
repository, and D14 scopes it to `vicondoa/d2b` precisely because D17 admits
the worker identity running agents is not contained. Branch protection on `v3`
and the human merge remain the last line rather than the only one.

**Keep engineering evidence in the repository under `.gc/`, as the brief
proposes.** Rejected twice over: `.gc/` is a name Gas City already owns for
city state, and spec section 12.5 forbids validation output, transcripts and
attestation payloads from entering Git at all. Evidence stays in the external
delivery state root that `storage.rs` already refuses to place inside a working
tree.

**No repo-local manifest at all.** Rejected. Gas City reads nothing from the
rig repository, so without D7 there is no file that travels with the code and
no place for a rename or a compatibility break to be caught in the commit that
causes it. The manifest is deliberately four keys so that it cannot become a
second configuration system, and it is contributor metadata rather than a
product surface, per D1.
