# ADR 0053: Gas City as contributor infrastructure, not a d2b capability

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0015](0015-daemon-only-clean-break.md) (daemon-only clean
  break, and its prohibition on host-singleton framework services),
  [ADR 0035](0035-efficiency-and-simplification-roadmap.md) (efficiency and
  simplification roadmap), [ADR 0046](0046-d2b-3-provider-control-plane.md)
  section 12.3 and
  [`docs/specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md)
  sections 12.2, 12.5 and 12.6,
  [ADR 0048](0048-copilot-native-agent-surface.md) (Copilot-native agent
  surface). This record changes none of them.
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
| `vicondoa/d2b` | `v3` | `5ccb12a2edaac85b4735ee95d697067bd3d339af` | 2026-08-02 |

Gas City's latest tagged release at that commit is `1.4.0` (2026-07-24,
`CHANGELOG.md`); the tree carries an `[Unreleased]` section on top of it.

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

**D4. Three layers of ownership, and d2b owns almost none of it.**

- **The host's `/etc/nixos`** owns the deployment instantiation: the identities
  of D13 and their privilege separation, ownership and mode of the approval
  store, per-unit credential delivery, service supervision, the publisher
  helper's exposure, the dashboard's read-only mode and loopback bind, network
  exposure, and anything else specific to one machine. This is operator-private
  configuration outside every repository named here.
- **`vicondoa/d2b-gascity-configs`** owns the reusable deployment
  configuration: a NixOS module that `/etc/nixos` imports, `city.toml`,
  `packs.lock`, pack imports and pins, the rig binding, the Discord, dashboard
  and GitHub wiring, provider and upstream environment, secret references by
  name rather than value, the workflow formulas, the artifact approval store,
  and the `tasks.md` importer.
- **`vicondoa/d2b`** owns this record, the follow-on contributor documentation,
  the Spec Kit artifacts under `specs/`, and the repo-local manifest of D6.
  Nothing else. No Gas City concern lands in `packages/` or `nixos-modules/`.

That the reusable layer is a NixOS module does not make it a d2b module: it is
imported by the operator's host configuration and is invisible to `flake.nix`
and to `d2b.*`, per D1.

Secrets are referenced, never inlined, in the configuration repository, and
their values exist only where `/etc/nixos` places them.

**D5. One local, single-user deployment, and what that scope does not
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
- **The approval reviewer identity is that one operator.** D11 records it for
  provenance and for binding a decision to a human, not to choose among
  reviewers or to evaluate a permission.
- **The dashboard binds loopback only** and gains no authentication layer,
  because there is no second *human* on the machine to authenticate. That is
  the limit of what this scope buys: loopback excludes remote humans and
  admits every local process, so D12 additionally requires read-only mode.
- **Nothing multi-party is designed.** No role model, no permissions matrix, no
  tenancy, no shared or hosted instance, no remote access path, no high
  availability, no failover, and no federation across cities.

What it does not remove: **local agent processes remain untrusted principals.**
"Single-user" is a statement about humans. The deployment still runs autonomous
agents, each of which is a local principal that can open loopback sockets, read
what its identity can read, and write what its identity can write. Every
control in D12 and D13 exists for those principals and none of them is relaxed
by the fact that one person is at the keyboard.

Concretely, the **worker and publisher separation, and the worker's inability
to write approvals, both stand unchanged** under D13. Those boundaries are
between the operator and an agent process, not between one human and several.
Collapsing them because "it is only me" would delete the design's only
mechanical controls while leaving every reason for them intact, and D13 forbids
it.

The same distinction applies to the Discord allowlist. It answers "which human
may speak to this deployment", which is a small question here because the
answer is one person. It does not answer "what may an agent do", which is the
question D13 answers, and the two must not be conflated because both happen to
be touched by the word "single-user".

**D6. One tiny repo-local manifest, with a d2b-owned schema.** Gas City reads
nothing from the rig repository, so this file is read by the configuration
repository's tooling only and must not imitate upstream syntax. It is
`.d2b-orchestration.toml` at the repository root and carries exactly `schema`,
`spec_root`, `gascity_compat` (a version range for `gc`), and `panel_authority`
(whose only legal value is `xtask-delivery`). It is contributor metadata, not a
configuration surface: it exists so that a rename of `specs/` or a
compatibility break is caught in the same commit that causes it. `.gc/` is
forbidden inside this repository; Gas City owns that name for city state.

**D7. Execution is upstream's supported local model.** Sessions run on the
`tmux` runtime, which is both the default and the registered fallback, and each
task works in a git worktree created from the checkout and removed on
completion. Both are what upstream ships and tests. No alternative backend is
configured in the first deployment.

**D8. Gas City owns durable execution; Spec Kit owns planning; neither owns
review.** For an opted-in run, Gas City owns workflow state, the task DAG,
dependency execution, retries, waits, routing, notification, runtime desire,
and non-convergence reporting. Spec Kit owns `constitution`, `specify`,
`clarify`, `plan`, `tasks`, `analyze` and their artifacts under `specs/`.
`speckit.implement` is not used for opted-in runs, and neither is
`/d2b-autopilot`: exactly one executor drives a given run, and for an opted-in
run that executor is Gas City.

**D9. Per-task execution is Superpowers-shaped, with an evidence rule that
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

**D10. Gas City orchestrates the panel stage; the panel itself stays d2b's and
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

Upstream `compound-engineering` may run as an **additional, non-binding**
pre-panel filter whose only output is findings for the integrator to fix before
the panel round. A clean compound review never sets `signoff`, never
substitutes for a seat, and never shortens the roster. `pr-pipeline`'s
`mol-pr-ship` may supply the readiness report; it does not gate the merge. The
standalone panel skill stays usable outside Gas City.

**D11. Approvals are artifact-bound and fail closed.** The approval record is
owned by the configuration repository's approval store, not by a Gas City gate,
a Discord message, or the dashboard. It records at minimum the run, the node,
the artifact path, the `sha256` of the exact artifact bytes, a decision from
the closed set `{approve, revise, rescope, abort}`, the reviewer identity, and
the timestamp. Per D5 the reviewer identity is always the one configured
operator, and a decision carrying any other identity is rejected rather than
recorded. A Gas City gate is the **transport** that blocks the run; Discord is
the **surface** that collects the decision; neither is the **authority**.

An approval is honoured only when the recorded `sha256` equals the artifact's
current bytes. On mismatch the run is **denied**, not warned, and the error
names the remediation: re-request approval on the new revision. This mirrors
section 12.6's content-invalidation rule that the panel already enforces, and
it is the direct answer to the upstream gate bug commit `d1e86f6` fixed.

Human approval is required at the constitution, spec, plan, task-DAG, and
publication gates. Analysis and per-task review are autonomous with escalation.

**The store has exactly one writer, and it is not the worker.** Durable
approval records are created only by D13's approval-controller identity, after
it has verified the Discord platform signature and applied D5's allowlist. The
worker identity, under which every agent runs, has no write path to the store
by any route: not a file, not a socket it may send a write to, not a helper
that writes on its behalf. Read access for the worker is permitted, since a run
must observe whether its gate is satisfied, and reading cannot forge.

This is the half of the boundary that is easy to omit. Withholding the GitHub
credential from agents accomplishes nothing on its own if an agent can write
the approval that makes the publisher act; the credential stays where it is and
the branch is published anyway, through the sanctioned path. M15 tests for it
directly.

**D12. The dashboard runs read-only, bound to loopback, and is not an
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

Discord, gated by D5's allowlist and D11's approval store, is the only human
interaction surface. The dashboard is for run observation, stage state,
transcripts, live activity and debugging. It may not carry an artifact
approval, may not answer a pending interaction, and may not be exposed beyond
loopback or fronted by a proxy; each of those is a new record, not a
configuration change.

**D13. Three host identities, because withholding the credential is not enough
if the precondition can be forged.** The design brief's rule is that
implementation agents do not hold publishing authority. A previous form of this
decision withheld the GitHub credential from the worker and had the publisher
verify a D11 approval before acting, which is defeated by a shorter path than
credential theft: if the worker can write the approval store, an agent
manufactures an approval for a revision it controls and then invokes the
publisher **legitimately**. The credential never leaks and the boundary still
fails. Authority over the precondition is therefore as load-bearing as
authority over the credential, and must be separated with the same mechanism.

Three Unix identities, established by `/etc/nixos`. This is host contributor
infrastructure and involves no d2b component.

- **Worker** runs `gc`, its sessions, and every agent. It holds the
  model-provider credential and read/write access to the checkout and its
  worktrees. It holds **no** GitHub publishing credential and has **no write
  path to the approval store**: it cannot create, amend, delete, or replace a
  durable approval record, and it cannot present one it authored. An agent that
  decides to `git push` fails for lack of a credential; an agent that decides
  to approve its own work fails for lack of write authority. Neither failure
  depends on a prompt.
- **Approval controller** receives Discord interactions, verifies the platform
  signature, applies D5's one-user and one-guild-and-channel allowlist, and is
  the **sole writer** of durable approval records. It runs no agent and holds
  no GitHub credential.
- **Publisher** holds the one GitHub credential scoped to `vicondoa/d2b` with
  push and pull-request permission. It runs no agent, and it **independently
  reads and verifies** the approval record and its `sha256` against the exact
  revision presented, rather than trusting any claim from the caller.

Three accounts on one machine is cheap. Each is a `users.users` entry and a
unit; the deployment is single-user and local, so there is no directory, no
role mapping, and no lifecycle to manage.

**A two-identity variant is permissible only if it is equally mechanical.**
Collapsing the approval controller and publisher into one **gatekeeper**
identity satisfies the same threat, because the property that matters is that
neither authority is reachable from the worker, not that they are separated
from each other. It qualifies only when all of the following hold, and it does
not qualify on any weaker basis:

1. The gatekeeper's uid differs from the worker's, and the approval store is
   owned by the gatekeeper with a mode that grants the worker no write access.
   Cross-uid ownership is the control. Same-uid separation by file mode,
   directory convention, a wrapper script, or an instruction to agents is not
   acceptable and does not satisfy this clause.
2. The GitHub credential reaches only the publishing unit, through systemd
   per-unit credentials (`LoadCredential=` into `$CREDENTIALS_DIRECTORY`),
   never through the gatekeeper's environment, home directory, or a
   world-of-that-uid readable path. This repository already uses that mechanism
   in `nixos-modules/guest-control.nix:297` and
   `nixos-modules/components/observability/stack.nix`, so it is established
   practice here rather than a novel claim.
3. The deployment documents that it has merged the network-facing Discord
   ingress parser with the credential holder, so a parsing defect in ingress
   now reaches the publishing credential.

Three identities is the recommended default precisely because of clause 3: the
approval controller parses attacker-influenced input from a network service and
the publisher does not, and keeping them apart bounds what a bug in the former
grants. The two-identity variant trades that for one fewer account.

The worker reaches the publisher only through a **narrowly exposed one-shot
helper** that accepts an integrated ref or revision, verifies the D11 approval
for exactly those bytes, and then pushes and opens the pull request itself. It
takes no branch name, remote, refspec, or command from the caller beyond the
revision under approval, and it exits after one publication.

**The helper's exact protocol, transport, and privilege-drop mechanism are
prototype and specification work**, and this record deliberately does not
invent them; several plausible shapes exist on a NixOS host and choosing among
them needs measurement rather than assertion. What is not deferred are the two
properties the identity split exists to provide, both of which are MVP
acceptance conditions and neither of which depends on which protocol wins:
M11 requires mechanical proof that the publishing credential is unreachable
from the worker, and M15 requires mechanical proof that the worker cannot write
or forge an approval record.

Credential rules that apply to every identity:

- **Model access** is whatever the configured providers require, scoped to
  model inference, and reaches the worker only. Where a model provider is a
  cloud service, its credential is a model-provider credential and is
  permitted; it is named in the configuration and scoped as narrowly as that
  provider allows.
- **Excluded outright** are credentials for unrelated infrastructure: compute,
  storage, deployment, CI administration, secret managers, or any cloud service
  the workflow does not use for model inference; credentials for another
  repository or organisation; and any credential the configuration does not
  name a use for.
- Credentials are never passed between the two identities, and redaction of
  credential values from logs, state files, and error text is a requirement on
  the configuration repository's own tooling.

**D14. Publication is one step, gated by a human, with the lint as defence in
depth.** Pushing a branch and creating a pull request happen only in a
publisher step, only after a recorded artifact-bound approval under D11 against
the exact integrated tree, and only through D13's helper. If the upstream
`github` pack's `push-branch` and `create-pr` commands are used, they are
reachable from that step and from nowhere else, and the formula set carries a
lint asserting it.

That lint is a third layer, not the control. The controls are D13's two
separations: a formula that reached a publishing command from the wrong step
would still fail because the worker holds no publishing credential, and a run
that manufactured its own approval would still fail because the worker cannot
write one and the publisher re-verifies the record itself. Ordering them this
way is deliberate, since a lint over a formula set is exactly the kind of guard
an agent editing formulas could regress.

Automatic publication is not configurable in the first deployment; the
capability is absent rather than disabled. The publisher is not the merger:
`v3` is protected and the merge stays the operator's, as it is today.

**D15. State the trust boundary honestly.** Agents run as the worker identity
with that identity's privileges, in git worktrees, with the model credential
reachable. There is no sandbox, no isolation of an agent from its sibling
worktrees, and no confinement of an agent to the checkout. The blast radius of
a misbehaving or compromised agent is the worker identity and everything it can
reach.

What that blast radius **excludes**, mechanically rather than procedurally, is
the pair of authorities that together constitute publication. A compromised
agent cannot push to `vicondoa/d2b` or open a pull request, because the
credential is not present in its identity. It also cannot manufacture the
approval that would let it invoke the publisher legitimately, because the
approval store is written by a different identity and the publisher verifies
the record itself rather than accepting the caller's word. Both halves are
required: either one alone leaves a complete path from a compromised agent to a
published branch. That pair is the only real containment property in this
design, and D13 is where it lives.

The dashboard is inside the blast radius in the sense that a worker agent can
reach it over loopback, which is why D12 requires read-only mode rather than
treating the loopback bind as a control. A local agent is a local principal;
"only I can reach it" is a statement about humans and D5 says so explicitly.

Everything else is scope and gating rather than containment: model access
scoped to inference, human approval at five named gates, the publication lint,
branch protection on `v3`, per-task worktrees, and a delivery state root that
stays outside every working tree. Those reduce the chance of a bad change
reaching `v3`; they do not contain a bad agent inside the worker identity. Any
claim stronger than that is unsupported by this design, and D2 forecloses the
obvious way to strengthen it until a separate ADR takes that up.

**D16. Everything imported is pinned.** Packs are pinned in `packs.lock`, whose
schema records `version`, `commit` and `fetched` per pack; imports pin a commit
with `Import.Version = "sha:<hex>"`. `gc` is pinned to a released version. The
configuration repository documents upgrade and rollback for each pinned
artifact, and an upgrade that changes a pack's formula step identifiers is
treated as a breaking change to the run.

**D17. The importer is a lenient parser with a strict output.** `tasks.md` has
no schema and no format version, so the importer parses what is reliable (the
`- [ ]` checkbox, the `T\d{3,}` identifier, the `[P]` and `[US\d+]` markers,
and `## Phase N` headers) and treats the rest as advisory. Prose dependencies
and prose file paths are extracted best-effort and then **confirmed by a human
at the task-DAG approval gate**, which is already in the flow. An unparseable
line fails the import with the line quoted; it is never silently dropped.
Re-import is keyed by task ID and must not duplicate unchanged tasks.

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
- No claim that this design isolates agents, beyond the two separations D13
  establishes and M11 and M15 prove. See D15.
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
- **No same-uid separation anywhere in the trust model.** A boundary asserted
  by file mode within one uid, by a wrapper script, by directory convention, by
  environment scrubbing, or by an instruction to agents does not count as a
  boundary in this design and may not be substituted for a distinct identity.
- No Discord ingress from any identity other than the one configured operator,
  and no allowlist that defaults open when unset or empty.
- **Single-user scope is never a reason to collapse identities or relax D12.**
  D5 removes multi-human concerns; it removes nothing about what a local agent
  process may do, and D12 and D13 stand independently of it.
- No credential beyond those D13 names, no credential value committed to
  either repository, and no publishing credential reachable from the worker
  identity.
- No assertion that a Gas City step can invoke an in-session Copilot slash
  command. The panel bridge is unproven; see D10 and M10.
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

- **M1 No product surface.** Mechanical: a scan finds no Gas City reference in
  `flake.nix`, `nixos-modules/`, `packages/`, `docs/reference/`,
  `docs/how-to/`, `docs/explanation/`, `README.md`, or
  `docs/contributing/critical-subsystems.md`, and no `d2b.*` option, manifest
  field, or schema property whose name or description mentions it. Mentions
  outside those surfaces are permitted, so `docs/adr/`, `docs/contributing/`,
  `specs/`, `.d2b-orchestration.toml`, `changelog.d/`, and `CHANGELOG.md` are
  not scanned for presence.

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

  Done when all three pass on a host with d2b up and at least one VM running.

- **M3 The standalone contributor surface is unaffected and self-sufficient.**
  With Gas City absent from the host, `node scripts/copilot/check-bindings.mjs`,
  `bash tests/unit/meta/adr-index-coverage.sh`, and `make check-tier0` pass;
  every skill and agent under `.github/skills/` and `.github/agents/` runs to
  completion without the external configuration repository, the `gc` binary, or
  any Gas City service; and no file under those two directories references Gas
  City, `d2b-gascity-configs`, `.d2b-orchestration.toml`, or the approval store.

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
- **M9 tasks.md import fidelity.** One real `tasks.md` from `specs/` imports
  into a validated DAG preserving IDs, parallel markers and dependencies, at
  least two independent tasks run concurrently, and a re-import after an
  unrelated edit creates no duplicate task. Done when a deliberately malformed
  line fails the import with the line quoted.
- **M10 The panel bridge is proven or the stage parks.** Either a Gas City run
  causes the existing d2b panel skill and `xtask delivery wave
  panel-request | panel-attest | seal` to run unchanged and drive a wave to
  sealed, with a deliberately mis-bound lane **rejected** by `panel-attest`; or
  the bridge is shown not to work and the stage parks on a durable gate for a
  contributor to run the panel by hand. Done when one of those two is
  demonstrated and the other is not silently substituted. A run that reports a
  panel outcome without a corresponding `panel-attest` record fails this item.
- **M11 The publishing credential is mechanically unreachable from the
  worker.** From the worker identity, with a full agent session running: no
  GitHub token is present in the environment; no readable file, git credential
  helper, `~/.config/gh`, or push-authorised SSH key yields one; and a direct
  `git push` and a direct pull-request creation against `vicondoa/d2b` both
  **fail for lack of a credential**. Done when each of those is attempted and
  observed to fail, not when a policy document says it should.

  This is the one MVP condition that is not negotiable on schedule grounds. The
  helper protocol of D13 may be prototyped, revised, or replaced; the
  non-reachability property it exists to provide may not ship unproven, because
  without it D14's lint is the only thing standing between an agent and the
  remote.

- **M12 Publication requires an approval, at both layers.** The run stops
  before push; the helper rejects a revision with no recorded artifact-bound
  approval, and rejects an approval whose `sha256` does not match the revision
  presented; and the formula-set lint of D14 rejects a formula that references
  `push-branch` or `create-pr` outside the publisher step. Done when the two
  helper rejections and the lint rejection are each observed.

- **M13 Discord ingress is allowlisted and fails closed.** A message from any
  Discord user id other than the configured operator, and where a guild is
  configured a message from any other guild or channel, produces no approval,
  no response, no clarification answer, no gate transition, and no workflow
  state change of any kind, and is rejected **at ingress** rather than filtered
  downstream. Done when three things are observed: a foreign-author message
  rejected before any workflow state is touched; a message from the configured
  operator in a non-configured channel likewise rejected; and an unset or empty
  allowlist rejecting the configured operator too, proving the default is
  closed rather than open.

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

- **M15 The worker cannot write or forge an approval.** Running as the worker
  identity, with a full agent session active: every write path to the durable
  approval store is attempted and observed to fail, including direct file
  creation and modification, replacement of an existing record, deletion, and
  any socket or helper the deployment exposes to the worker. Then the end-to-end
  case: a run that fabricates an approval for a revision it controls and
  invokes the publisher is **refused by the publisher**, which reads the store
  itself and finds no matching record.

  Done when both the write attempts and the fabricated-publication attempt are
  observed failing. Read access from the worker is expected to succeed and is
  not a finding.

  M11 and M15 are a pair and neither is sufficient alone. M11 without M15
  leaves an agent able to manufacture the precondition and publish through the
  sanctioned path; M15 without M11 leaves an agent able to publish directly.

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

**The worker identity is not isolated, and D15 says so rather than implying
otherwise.** Agents run as the worker identity with its privileges and reach
the model credential. A prompt-injected or malfunctioning agent can do anything
that identity can do, and neither the worktree nor the workflow shape prevents
it. What it cannot do is publish, and that now rests on two separations rather
than one: it holds no publishing credential, and it cannot write the approval
that would make the publisher act on its behalf. Everything else is scope,
gating, and review, which reduce the chance of a bad change reaching `v3` and
do not contain a bad agent. Anyone deploying this should size the worker
identity accordingly, and should notice that the separations are ordinary Unix,
need no d2b component, and convert the two most consequential authorities in
the system from prompt-level rules into mechanical ones.

**"Local" is not a trust boundary, and the dashboard is where that bites.**
The single-user scope makes it tempting to treat loopback as a perimeter, and
for humans it is one. For agents it is not: every agent this deployment runs is
already inside it, and the upstream dashboard exposes a respond path that
answers an agent's own permission prompts. Read-only mode is therefore a
requirement rather than a hardening option, and the cost is real: run
observation is all the dashboard provides, so anything an operator might have
done through its controls now happens through Discord or a shell. That is the
correct trade, but it is a trade.

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

**Deployment is split across three places, one of which is not version
controlled here.** `/etc/nixos` holds the instantiation, so a working
deployment cannot be reproduced from the two repositories alone. That is the
correct place for machine-specific and secret-adjacent configuration, but it
means the configuration repository must document what `/etc/nixos` is expected
to provide, and a missing expectation shows up as a runtime failure rather than
an evaluation error.

**Reviews get slower and more expensive, on purpose.** An opted-in run carries
per-task spec and quality reviews, an optional 17-lane compound filter, and the
binding ten-seat panel, on top of validation. The ten seats are already a
deliberate cost recorded in ADR 0048; this record adds two review layers in
front of them and removes nothing.

**Operational dependence on Discord and a temporary dashboard.** A Discord
outage means the human surface is gone and runs park at gates. The dashboard
repository describes itself as a temporary workspace and has no auth model, so
D12 confines it to loopback observation; when it folds back into `gc`, its
approval semantics must be re-measured before D12 can be relaxed.

**Single-user scope is cheap now and will be the expensive thing to change.**
D5 removes a permissions matrix, a role model, a tenancy story, and a dashboard
authentication layer, none of which this deployment needs. The bill arrives if
a second person ever needs access, because there is then no authentication
surface to extend and no notion of an actor other than the configured operator
anywhere in the approval records. That is a rewrite of the human-identity half
of the design, not a configuration change, and it deserves its own record. The
trade is accepted because building multi-party access control for one person is
speculative work whose only certain outcome is more code to keep correct.

The half that does **not** get cheaper is the agent boundary. D13's separation
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
producer is a language model. D17 makes drift loud at import time rather than
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

**Keep the whole deployment in `d2b-gascity-configs`, with nothing in
`/etc/nixos`.** Rejected. Secret material, the service account, and network
exposure are properties of one machine, and a shared repository is the wrong
authority for them. The split costs reproducibility from the repositories
alone, which D4 accepts and the configuration repository documents.

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
infrastructure to partially close the D15 gap is a decision that deserves its
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
defence in depth in D14, which is the right rank for it.

**Collapse the identities, since it is a single-user machine anyway.**
Rejected, and the reasoning that makes it tempting is the reasoning that makes
it wrong. D5's scope means one *human* uses this deployment; it says nothing
about how many autonomous agent processes run under it, which is the population
D13 constrains. The publishing credential and the approval-write authority are
withheld from agents, not from the operator, who retains both through the
controller and publisher identities. Merging them into the worker would trade
the design's only mechanical controls for the removal of two `users.users`
entries.

**Withhold the GitHub credential but let the worker write approvals.**
Rejected, and this was the defect in the previous form of D13. It looks like a
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
The non-goals say this explicitly because it is the shape a hurried
implementation reaches for when a third account feels like ceremony.

**Rely on the dashboard's loopback bind instead of read-only mode.** Rejected.
Loopback is a control against remote humans and no control at all against local
agents, and the dashboard's respond endpoint is exactly the capability an agent
would want. Read-only mode is measured, is one environment variable, and is
enforced in the proxy rather than in the UI, so it holds against a caller that
never loads the frontend.

**Build multi-user access control now.** Rejected. A permissions matrix, a role
model, and a dashboard authentication layer would be written for a second
principal who does not exist, tested against a threat model nobody has stated,
and maintained by the one person they were supposed to protect against. D5
takes the scope as given and states the cost of reversing it in Consequences
rather than paying it up front. Note that this rejection is about **human**
principals only; the agent-principal controls in D12 and D13 are built now
precisely because those principals do exist today.

**Give the publisher the operator's general GitHub identity.** Rejected. It is
the one credential in the system with lasting consequences outside this
repository, and D13 scopes it to `vicondoa/d2b` precisely because D15 admits
the worker identity running agents is not contained. Branch protection on `v3`
and the human merge remain the last line rather than the only one.

**Keep engineering evidence in the repository under `.gc/`, as the brief
proposes.** Rejected twice over: `.gc/` is a name Gas City already owns for
city state, and spec section 12.5 forbids validation output, transcripts and
attestation payloads from entering Git at all. Evidence stays in the external
delivery state root that `storage.rs` already refuses to place inside a working
tree.

**No repo-local manifest at all.** Rejected. Gas City reads nothing from the
rig repository, so without D6 there is no file that travels with the code and
no place for a rename or a compatibility break to be caught in the commit that
causes it. The manifest is deliberately four keys so that it cannot become a
second configuration system, and it is contributor metadata rather than a
product surface, per D1.
