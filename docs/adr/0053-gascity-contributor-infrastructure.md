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

- **The host's `/etc/nixos`** owns the deployment instantiation: the five
  principals of D13 and their privilege separation, ownership and mode of the
  approval store and the protected integration object store, the root-owned
  append helper, per-unit credential delivery, the peer-credential-checked
  socket, the worker network namespace and firewall policy of D14, service
  supervision, the publisher's exposure, the dashboard's read-only mode and
  loopback bind, log rotation and retention, and anything else specific to one
  machine. This is operator-private configuration outside every repository
  named here.
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

  **A rejection is diagnosed locally, never silent.** Silent unresponsiveness
  is indistinguishable from a broken token, a dead service, or a bot that was
  never invited, and the operator would have no way to tell which. Every
  rejection emits one local, bounded, redacted diagnostic naming the rejection
  class from a closed set (`unconfigured-allowlist`, `identity-mismatch`,
  `guild-mismatch`, `channel-mismatch`, `signature-invalid`) together with the
  received and configured identity digests of D11, never the raw ids. Digests
  are what makes it actionable: the operator can compare the received digest
  against the configured one and see whether they are dealing with the wrong
  account or an empty configuration, without the diagnostic itself becoming a
  place where platform identifiers accumulate in plaintext.
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
typing a command is a worse gate than no gate, and the whole point of D9 is
that Gas City cannot report a review that did not happen.

Upstream `compound-engineering` may run as an **additional, non-binding**
pre-panel filter whose only output is findings for the integrator to fix before
the panel round. A clean compound review never sets `signoff`, never
substitutes for a seat, and never shortens the roster. `pr-pipeline`'s
`mol-pr-ship` may supply the readiness report; it does not gate the merge. The
standalone panel skill stays usable outside Gas City.

**D11. Approvals are artifact-bound, digest-only, append-only, and fail
closed.** The approval record is owned by the deployment's approval store, not
by a Gas City gate, a Discord message, or the dashboard. Each record carries:
the gate node identifier, the artifact identity, a decision from the closed set
`{approve, revise, rescope, abort}`, a reviewer digest, a run digest, and the
timestamp. For the publication gate the artifact identity is D13's immutable
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
handle across records while the raw value stays unrecoverable. Per D5 the
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
by D13's root-owned `append-helper`, which exposes one append operation and no
update, replace, truncate, or per-record delete operation of any kind. Neither
`agent-worker` nor `orchestrator` nor `approval-controller` nor `publisher` may
write, rewind, or truncate it directly.

**The store layout is fixed, and the helper derives every component of it.**

```
<store root>/
  periods/
    2026-08-02/          <- current period, pinned by the helper
      <64 hex>.rec       <- one immutable record
    2026-08-01/          <- sealed period
  quarantine/
    2026-07-01.<token>/  <- a period removed from history, pending cleanup
```

The helper opens the store root once, then opens or creates `periods/` and
`quarantine/` fd-relatively beneath it, then opens or creates the current
period directory fd-relatively beneath `periods/`. The period component is a
canonical UTC `YYYY-MM-DD` string the **helper** computes from its own clock; a
caller cannot name, select, hint at, or influence it. Records are written as
final basenames inside the pinned current-period descriptor, never at the store
root and never in a path a caller can address.

Rotation seals the current period and switches the pinned descriptor to the new
date; sealing changes no bytes. Retention operates one level up, renaming a
whole child of `periods/` into `quarantine/`. Because `periods/` and
`quarantine/` are siblings under one root, that rename is a directory-to-
directory move within a single filesystem, which is what makes whole-period
removal atomic.

**The helper authenticates every caller and binds it to one record kind.** Its
Unix socket checks the peer credentials of each connection (`SO_PEERCRED`, or
the platform equivalent) against a closed matrix. There are three record kinds
and three permitted callers, and the mapping is one to one:

| Caller | Peer identity | May append | Any other kind |
| --- | --- | --- | --- |
| Approval controller | `approval-controller` | `approval` | reject |
| Publisher | `publisher` | `publication-attempt` | reject |
| Retention timer | uid 0 | `retention-deletion` | reject |
| Anyone else, including `agent-worker` and `orchestrator` | any | nothing | reject |

Cross-kind requests are the point of the matrix, not an afterthought: a
compromised publisher cannot forge an approval, a compromised controller cannot
forge a publication audit trail, and neither can synthesise a retention record
to explain a period it removed. The retention timer is admitted precisely
because the retention rule below requires it to record its own deletions, and a
deletion it could not record would otherwise force either an unaudited gap or a
second write path. Being uid 0 grants it no additional kinds. No caller can
erase or alter anything already written, and the publisher re-reads and
verifies the record it acts on rather than trusting a value passed to it.

**The request carries record bytes and nothing else.** A request has exactly
two fields, the record kind and the canonical record bytes. There is no
filename, path, path component, period selector, or directory field to attack,
and the request schema **denies unknown fields**, so a caller that invents one
is rejected rather than having it ignored. Canonical record bytes are bounded
at **4096 bytes**; a record is a small set of digests, hashes, identifiers and
timestamps, and anything larger indicates a caller doing something other than
what this store is for.

The helper bounds the length, canonicalises the encoding, computes the digest
itself, and derives the basename from that digest as 64 lowercase hexadecimal
characters plus the fixed suffix `.rec`. It validates the derived basename
against that exact shape before use, so even a defect in derivation cannot
produce a name containing a separator, a traversal component, a leading dot, or
anything outside the permitted alphabet.

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

**Every descriptor is close-on-exec.** The pinned store root, `periods/`,
`quarantine/`, the current period, every temporary, every verification open,
and the listening and accepted append sockets are all opened `O_CLOEXEC`, or
have `FD_CLOEXEC` set atomically at creation where a call lacks the flag. The
helper runs as root and may spawn a child; a descriptor to the store leaking
across an `exec` would hand a write path to whatever it ran.

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
2. write the complete bounded record, looping until every byte is written so a
   short write is completed rather than truncating the record, and retrying
   `EINTR` a bounded number of times; then `fsync` the file;
3. install it under the content-addressed final name with
   `renameat2(RENAME_NOREPLACE)` against the same pinned descriptor;
4. `fsync` the current-period directory;
5. only then acknowledge the append.

Every syscall in that sequence retries `EINTR` boundedly and treats exhaustion
of the bound as a failure rather than as success.

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
name is the digest of the bytes, so an existing file under that name should be
those exact bytes, which happens whenever a caller retries after a crash
between step 4 and step 5. The helper opens the existing record anchored under
the pinned descriptor, verifies its length equals the request's length and its
bytes hash to the same digest, confirms it is a regular file in its durable
final state, and then returns **idempotent success**. If the length or the
bytes disagree, the helper does not overwrite, does not repair, and does not
succeed: it fails closed and reports corruption, because a mismatch under a
content-addressed name means something outside this design has written to the
store.

**Readers open final names only.** Temporaries carry a prefix that cannot occur
in a derived basename, and every reader enumerates the store accepting only
names matching the record shape. A temporary is never opened, never counted,
and never treated as history.

**Stale temporaries are reclaimed, and that is not a history mutation.** A
crash between steps 1 and 3 leaves a temporary with no final name. On start,
and on its periodic sweep, the helper unlinks temporaries older than a bounded
threshold that no live append holds. This does not contradict append-only: a
file that never received a final name was never a record, is invisible to every
reader, and its removal changes no history that anything could have observed.

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
wholly present and byte-identical to when it was sealed, or wholly absent.

**Removal is a rename first, a delete second.** Deleting a populated directory
in place is not atomic: a power loss part way through leaves a visible sealed
period missing an arbitrary subset of its records, which is exactly the state
the invariant above forbids and is indistinguishable from tampering. The timer
therefore proceeds in this order:

1. `renameat2(RENAME_NOREPLACE)` the whole period directory from `periods/`
   into `quarantine/`, both descriptors pinned and both opened `O_CLOEXEC`.
2. `fsync` **both directories that the rename modified**: the `periods`
   descriptor, which lost an entry, and the `quarantine` descriptor, which
   gained one. Syncing a common parent is not equivalent and is not what this
   requires; a cross-directory rename changes two directories and both must be
   made durable before anything depends on the move having happened.
3. Append the `retention-deletion` record naming the quarantined period, its
   record count, and its digest.
4. Only then remove the quarantined directory's contents recursively.

Ordering the two `fsync` calls before the record is what keeps the audit trail
honest. A record appended before the rename was durable could describe a
deletion that a crash then un-did, leaving history asserting the removal of a
period that is still present.

A crash leaves exactly one of two states: the original sealed directory,
untouched and still visible, or a quarantined directory that is no longer part
of history. There is no third state in which a visible sealed period is
partially deleted.

**`periods/` and `quarantine/` must share a filesystem, and `EXDEV` is a
configuration error.** The whole-period guarantee rests on the rename being
atomic, and a rename across filesystems is not merely unsupported, it is
unimplementable atomically: the copy-then-delete that would substitute for it
has a window in which the period exists in neither place or in both, which is
precisely the partial state this design forbids. So there is no copy fallback,
and there will not be one. The helper stats both directories at startup and at
activation and asserts they share a `st_dev`, refusing to start with a typed
fail-closed configuration error if they do not. An `EXDEV` observed later is
the same typed error rather than a trigger for a degraded path.

**Recovery resumes, and never deletes unaudited.** On start the timer
enumerates `quarantine/` and finishes what it finds: if the
`retention-deletion` record for a quarantined period is absent it appends it,
then completes the recursive removal. If the append fails, for any reason
including the append helper being unavailable, the quarantined directory is
**retained and retried** rather than removed, because a period deleted without
its record is a gap the history cannot explain.

**A retained quarantine is reported, not merely accumulated.** Letting the
256 MiB backstop be the first signal would surface a broken append helper as a
disk-capacity message weeks later, which tells the operator nothing about the
cause. The timer instead emits the closed error `retention-audit-append-failed`
on the first failed attempt, naming the quarantined period, the underlying
append error, and the remediation: inspect the `append-helper` unit, then run
the status command below.

The configuration repository owns an operator surface for this,
`gascity-audit status`, which lists every quarantined period with its cause,
the last append error observed for it, its age, its size on disk, and the
remediation for that cause. It is a read-only inspection command; it cannot
delete a quarantined period, because doing so would be an unaudited deletion by
another name. Clearing a quarantine happens only by fixing the append path and
letting the timer complete the sequence it started.

Each retention deletion is therefore recorded through the same append path as
everything else, is equally immutable, and is written by the one caller
authorised for that kind.

**Only the approval controller may originate a record, and it is not the
worker.** A record reaches the append helper only from D13's
`approval-controller`, and only after that principal has verified the Discord
platform signature, applied D5's allowlist, and derived the artifact identity
itself under D13. `agent-worker` has no origination path by any route: not a
file it can write, not a socket that accepts a record from it, not a helper
that appends on its behalf. Read access for the worker is permitted, because a
run must observe whether its gate is satisfied and reading cannot forge.

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

**D13. Five host principals: four dedicated unprivileged identities plus a
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
| `agent-worker` | Model credential, its own worktrees, a restricted network namespace (D14) | Read orchestrator state or memory, write the integration store, write approvals, publish |
| `approval-controller` | Discord ingress, the verifying key, the allowlist | Hold a GitHub credential, run agent code, rewrite approval history |
| `publisher` | The one GitHub credential scoped to `vicondoa/d2b` | Run agent code, accept a mutable ref, write approvals |
| `append-helper` | Root-owned; sole writer of the approval and audit store, admitting three authorised caller-and-kind pairs per D11 | Expose any update, replace, truncate, or per-record delete operation |

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

**D14. The agent-worker runs in a restricted host network namespace with
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
  simply not addressable from inside it. This is why D12's read-only
  requirement and this decision are complementary rather than redundant: D12
  holds if the namespace is ever misconfigured, and this holds if the
  dashboard's mode is ever wrong.

Allowed, only when named in the configuration:

- the configured model provider endpoints;
- package registries and Nix substituters the tasks need;
- DNS to a specified resolver;
- GitHub **read** access, if and only if a task genuinely requires fetching
  rather than working from the checkout. Publication is unaffected: the worker
  has no credential to publish with under D13, so read reachability grants it
  nothing.

The policy is default-deny with an explicit allowlist, not default-allow with
exceptions, so an endpoint nobody considered is refused rather than reached.
Unix-domain sockets are not affected by the network namespace, which is why
D13's peer-credential-checked socket remains the worker's channel to the
orchestrator and does not require relaxing anything here.

M17 tests this with planted denied targets and allowed controls, because a
firewall that denies everything, including what the workflow needs, passes a
naive "is it blocked" check while being useless.

**D15. Publication is one step, gated by a human, audited on every attempt,
with the lint as defence in depth.** Pushing a branch and creating a pull
request happen only in a publisher step, only against the immutable commit hash
approved under D11, and only through D13's publisher. If the upstream `github`
pack's `push-branch` and `create-pr` commands are used, they are reachable from
that step and from nowhere else, and the formula set carries a lint asserting
it.

**Every publication attempt emits exactly one audit record, whether it
succeeds or fails.** Publication is the only effect this system has outside the
host, so an attempt that leaves no trace is the one case where operational
evidence matters most. The record is appended through D11's append-only helper
and carries: the commit hash, the approval record's own digest, the outcome as
one of a closed set, the failure class when it failed, and the timestamp. It is
bounded in size and redacted, carrying digests rather than the reviewer's raw
identity or the run handle, and it never carries a credential, a token, a
remote URL with embedded authentication, or captured command output. Exactly
one record per attempt: a refusal before the push is still an attempt, and a
retry is a second attempt with its own record.

The lint is a third layer, not the control. The controls are D13's separations:
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

**D16. State the trust boundary honestly.** Agents run as `agent-worker` with
that identity's privileges, in git worktrees, inside D14's network namespace,
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
from inside D14's namespace, and if that namespace is ever misconfigured, D12's
read-only mode still refuses the mutation. Neither is presented as sufficient
alone. A local agent is a local principal; "only I can reach it" is a statement
about humans, and D5 says so explicitly.

Everything else is scope and gating rather than containment: model access
scoped to inference, human approval at five named gates, the publication lint,
branch protection on `v3`, per-task worktrees, and a delivery state root that
stays outside every working tree. Those reduce the chance of a bad change
reaching `v3`; they do not contain a bad agent inside the worker identity. Any
claim stronger than that is unsupported by this design, and D2 forecloses the
obvious way to strengthen it until a separate ADR takes that up.

**D17. Everything imported is pinned.** Packs are pinned in `packs.lock`, whose
schema records `version`, `commit` and `fetched` per pack; imports pin a commit
with `Import.Version = "sha:<hex>"`. `gc` is pinned to a released version. The
configuration repository documents upgrade and rollback for each pinned
artifact, and an upgrade that changes a pack's formula step identifiers is
treated as a breaking change to the run.

**D18. The importer is a lenient parser with a strict output.** `tasks.md` has
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
- No claim that this design isolates agents, beyond the two separations D13
  establishes and M11 and M15 prove. See D16.
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
- **No caller-supplied paths into the store.** The append helper derives every
  record name from the record bytes it was given and every period component
  from its own clock, resolves fd-relative from a pinned descriptor with
  symlink and magic-link refusal, and rejects a request carrying any field
  beyond the record kind and the record bytes rather than ignoring it.
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
  helper protocol of D13 may be prototyped, revised, or replaced; the
  non-reachability property it exists to provide may not ship unproven, because
  without it D15's lint is the only thing standing between an agent and the
  remote.

- **M12 Publication requires an approval bound to an immutable commit.** The
  run stops before push. The publisher rejects each of: a commit with no
  approval record; an approval whose recorded commit hash differs from the
  commit presented; an approval recorded against a gate node other than the
  publication gate; an approval whose decision is not `approve`; and **any
  mutable reference offered in place of a commit hash**, including a branch
  name, a tag name, `HEAD`, and a symbolic ref. The formula-set lint of D15
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

- **M18 Approval and audit history is append-only, authenticated, atomically
  installed, path-contained, and bounded to stated defaults.** Nine parts.

  1. **Immutability.** Attempts to modify history fail from every principal
     that is not the append helper, including `approval-controller`,
     `publisher`, and the retention timer: overwriting an existing record,
     truncating the store, and deleting an individual record are each refused.
     Re-appending identical bytes is not a modification and is covered by
     part 4.
  2. **Caller and kind authorization.** The helper's socket enforces the D11
     matrix over all **fifteen** caller-and-kind combinations: three accepts,
     one per authorised pair (`approval-controller` with `approval`,
     `publisher` with `publication-attempt`, uid 0 retention timer with
     `retention-deletion`); six cross-kind rejections, being each of those
     three callers offering each of the two kinds it does not own; and six
     categorical rejections, being `agent-worker` and `orchestrator` each
     offering all three kinds. Every combination is exercised; none is inferred
     from another.
  3. **Request schema.** A request carrying any field beyond the record kind
     and the record bytes is rejected rather than ignored, verified by sending
     requests with added `path`, `name`, `filename`, and `period` fields. A
     record of exactly 4096 canonical bytes is accepted and one of 4097 is
     rejected, so the bound is tested at its edge rather than in the middle.
  4. **Append atomicity, durability, and idempotent retry.** A reader
     concurrent with an append never observes a partial record, verified by
     enumerating the store throughout a large append and confirming every
     visible name is a complete record. Injected crashes are exercised at four
     points: after the temporary is created; after it is written and `fsync`ed;
     after the rename but before the directory `fsync`; and **after the
     directory `fsync` but before the acknowledgement reaches the caller**.
     The first two leave no visible record and a reclaimable temporary. The
     third leaves a complete, readable record. The fourth is the one that
     matters for callers: the caller never saw success, retries the identical
     append, and the helper returns **idempotent success** after verifying the
     existing record's length and digest, without writing anything and without
     creating a duplicate.

     Corruption is distinguished from idempotency: with a file planted under a
     valid record name whose bytes do not hash to that name, the same retry
     **fails closed** and reports corruption rather than succeeding or
     overwriting. Short writes and `EINTR` are exercised by injection and must
     complete the record rather than truncate it. An acknowledged append
     survives immediate power loss. On restart, stale temporaries are
     reclaimed, no reader has ever counted one, and reclamation appends no
     record.
  5. **Path containment.** Two layers, because the derived-name path is pure
     hexadecimal and an end-to-end test alone would pass without ever
     exercising resolution.

     Unit: the internal component resolver is called directly with `..`, an
     absolute component, a component containing a separator, a symlinked
     component, and a magic-link component, and must refuse each. A test that
     passes only because the caller cannot supply a name does not satisfy this
     part, so each case asserts that resolution was attempted and refused.

     Integration: only physically possible controls are planted, since the
     request carries no filename to poison. A symlinked store root, a symlinked
     period directory, and a symlinked record name are each planted, and each
     must be refused with no file created outside the store. Impossible
     controls from an earlier draft, an absolute or `..` filename in a request,
     are not tested because the schema of part 3 has no field to carry them.
  6. **Descriptor hygiene.** Every descriptor the helper holds is
     close-on-exec, verified by inspecting `/proc/<pid>/fdinfo` flags for the
     pinned store root, `periods/`, `quarantine/`, the current period, the
     listening socket, and an accepted connection, and by confirming that a
     child process spawned by the helper inherits none of them.
  7. **Publication audit completeness.** Every publication attempt, success and
     failure alike, produces exactly one `publication-attempt` record, verified
     by counting records across a successful publication, a publication refused
     for a missing approval, and a publication refused for a ref rather than a
     hash.
  8. **Redaction.** No durable record contains a raw Discord id, a raw run
     handle, a credential, or a remote URL carrying authentication, verified by
     scanning the store with counted coverage and a planted control.
  9. **Retention.** Daily rotation runs on schedule and sealing changes no
     bytes. Event logs are trimmed at whichever of 14 days, 512 MiB aggregate,
     or the 64 MiB per-file cap binds first. Approval and publication audit
     periods older than the 365-day floor are removed as **whole sealed
     periods**, never in place: the period is renamed from `periods/` into
     `quarantine/` with `RENAME_NOREPLACE`, **both** the `periods` and
     `quarantine` descriptors are `fsync`ed, the `retention-deletion` record is
     appended, and only then are the contents removed. The two-descriptor sync
     is asserted directly, since syncing one or syncing a common parent is a
     different and insufficient operation.

     Injected crashes are exercised at four points and the invariant checked
     after each: before the rename, leaving the original sealed directory
     intact and visible; after the rename but before either `fsync`; after both
     `fsync` calls but before the record is appended, which recovery finishes
     by appending the record and then removing; and after the record but before
     removal completes, which recovery finishes by removing. At no point is a
     **visible** sealed period observed missing any record it held when sealed,
     and at no point does a `retention-deletion` record exist for a period that
     is still visible in `periods/`.

     Failure handling is tested directly: with the append helper made
     unavailable, a due deletion renames into quarantine, fails to append,
     **retains the quarantine for retry** rather than removing it, and emits
     `retention-audit-append-failed` naming the period, the append error, and
     the remediation. `gascity-audit status` then lists that period with its
     cause, last error, age, and size, and offers no deletion action. The timer
     is also observed to refuse a partial deletion and to refuse touching the
     current unsealed period.

     Configuration assertions: with `periods/` and `quarantine/` placed on
     different filesystems, startup **refuses** with the typed configuration
     error rather than starting and falling back to a copy, and a later
     `EXDEV` produces that same typed error. With `renameat2` unavailable or
     `RENAME_NOREPLACE` unsupported on the store filesystem, startup refuses
     with a typed error rather than selecting an alternative install path.
     Where the 256 MiB audit cap would require deleting a period younger than
     the floor, deletion is **refused** and the condition reported, verified by
     driving the store past the cap with synthetic records.

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

**The worker identity is not isolated, and D16 says so rather than implying
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
producer is a language model. D18 makes drift loud at import time rather than
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
infrastructure to partially close the D16 gap is a decision that deserves its
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
defence in depth in D15, which is the right rank for it.

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
and maintained by the one person they were supposed to protect against. D5
takes the scope as given and states the cost of reversing it in Consequences
rather than paying it up front. Note that this rejection is about **human**
principals only; the agent-principal controls in D12 and D13 are built now
precisely because those principals do exist today.

**Give the publisher the operator's general GitHub identity.** Rejected. It is
the one credential in the system with lasting consequences outside this
repository, and D13 scopes it to `vicondoa/d2b` precisely because D16 admits
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
