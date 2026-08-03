# ADR 0053: Gas City as contributor infrastructure, not a d2b capability

- Status: Proposed
- Date: 2026-08-02
- Related: [ADR 0015](0015-daemon-only-clean-break.md) (daemon-only clean
  break), [ADR 0035](0035-efficiency-and-simplification-roadmap.md),
  [ADR 0046](0046-d2b-3-provider-control-plane.md) and its validation and
  delivery contract in
  [`docs/specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md);
  section 12.5 cited here is a section of that specification, not of ADR 0046.
  Also
  [ADR 0048](0048-copilot-native-agent-surface.md) (Copilot-native agent
  surface). This record changes none of them.
- Scope: contributor workflow ownership and external configuration. It decides
  properties and repository ownership; it deliberately does not freeze
  mechanisms that prototypes P0 through P8 must settle first.
- Unblocks: the prototype program below, then a follow-on specification in
  `docs/contributing/`.

## Context

This repository already runs a heavyweight engineering process: Spec Kit
planning, thirteen committed Copilot agents, five d2b skills, a ten-seat panel,
and an attest/seal/merge-eligibility gate in `packages/xtask/src/delivery/`.
What it lacks is **durable orchestration**: a run lives inside one agent
session, and a context handoff or a closed terminal loses the position.

The decision is to put a durable orchestrator underneath that process, and to
do it by **extending Gas City's existing workflow library rather than
reimplementing it**. An earlier revision of this record hand-rolled an
eleven-stage pipeline, a five-principal security model, a custom append-only
audit store and a network namespace. An expert review against the running
system found that roughly forty percent of that had to be replaced rather than
amended: it reinvented `build-base`, picked the one pack-composition shape the
loader breaks, and specified lifecycle and isolation machinery for a Gas City
that does not exist. This revision replaces those parts and shortens the record
by moving mechanism to prototypes and specs, keeping only properties here.

**d2b is the repository this system works on. It is not part of how the system
runs.** No `d2bd`, broker, microVM, Provider, Runtime, Service, guest transport
or Credential service participates. That constraint is unchanged and is the
subject of D2.

### What category this belongs to

**Gas City is contributor infrastructure. It is not a d2b feature.** d2b's
product surface is its flake outputs, NixOS option schema, versioned manifest
and bundle contract, daemon and broker protocols, CLI, and the Diataxis
documentation tree. Gas City is none of those. It is tooling some contributors
use to work **on** this repository, in the same category as `.github/agents/`,
`scripts/copilot/`, the `Makefile` and `tests/tools/`. A d2b consumer must be
able to adopt, configure and run the framework without learning it exists.

### What was measured, and the gap that matters

| Repository | Ref | Commit | Date |
| --- | --- | --- | --- |
| `gastownhall/gascity` | `main` | `38ed358fae0e8238834eb778a23a664fe4cb8954` | 2026-08-02 |
| `gastownhall/gascity` | `v1.4.0` | `a7297c5` | 2026-07-24 |
| `gastownhall/gascity-packs` | `main` | `0b9574272814ba175950731b73c2cc201804ee61` | 2026-08-01 |
| `gastownhall/gascity-dashboard` | `main` | `fdd2d636751963ca786d06c9e43369fc6a71f7e2` | 2026-07-17 |
| `github/spec-kit` | `main` | `d1e86f638277a99b82715c22c90558cd58d3cffd` | 2026-07-31 |
| `numtide/llm-agents.nix` | `main` | `7b99fc4bbb8a7c2fff82c2708d8636c1cbc65661` | 2026-08-03 |
| `vicondoa/d2b` | `adr-gascity-orchestration` | `b5881b2a6a42cd6e4db89c662c9df853f6a98d55` | 2026-08-02 |

**The evidence commit is not the deployed binary, and this record says so.**
`llm-agents.nix` builds tag `v1.4.0`, which is `a7297c5`; the `main` commit
above is **213 commits ahead**. Every behavioural claim sourced from `main`
is therefore a claim about a compiler the deployment does not run, including
control-dispatcher and step-topology changes. D19 and P7 resolve this: either
the evidence is re-measured at the built commit, or `gascity.nix` carries a
commit-pinned package override sourced through `llm-agents.nix`. Until then,
every `main`-sourced statement below is marked as needing P7 confirmation.

Three packaging facts follow from the same file and matter operationally:
`ldflags` set `-X main.commit=nixpkgs`, so `gc version` cannot report an
upstream commit; `doCheck = false`, so upstream's tests never run; and the
wrapper PATH is `beads dolt flock gitMinimal jq lsof procps tmux` with **no
`python3`**, while every `discord` and `github` pack service and command is a
python3 script.

### The Gas City workflow library we extend

`gascity/formulas/build-base.formula.toml` is a virtual full-lifecycle build
contract that concrete methodology packs override. It declares the stages
`prepare`, `requirements`, `plan`, `plan-review`, `decompose`, `implement`
(drain) or `implement-same-session`, `summarize-implementation`, `review`,
`finalize`, `publish`; extension seams as vars including `planning_formula`,
`decomposition_formula`, `implementation_formula`,
`implementation_item_formula`, `code_review_formula`, `review_fix_formula` and
`max_iterations`; postures `interaction_mode` in
`{interactive, autonomous, headless}` and `review_mode` in
`{report, agent, interactive}`; per-stage artifact schemas under
`gc.build.*.v1`; and `push` and `open_pr` vars defaulting to `"false"`.

`fix-loop-base.formula.toml` is the review-fix contract (`plan-fixes`,
`apply-fixes`, `re-review`, with `findings_path` and `max_iterations`).
`publish.formula.toml` consumes `push` and `open_pr` and runs its `push` and
`open-pr` steps under the `gc.publisher` role. `build-from-plan`,
`build-from-requirements`, `build-from-decompose`, `build-from-convoy` and
`build-from-review` are cataloged entry routes, each with a `-base` variant.

This library is the spine of D6. The earlier revision cited none of it.

### Pack composition: siblings, not nesting

`internal/config/pack.go` stamps binding names at load, with an explicit
comment that **at the city level all agents from an import get the city's
binding, overriding any nested bindings, because the city is the root of
composition**. Upstream formulas hardcode binding-qualified run targets such as
`gc.run-operator`, `gc.implementation-worker`, `gc.publisher`,
`superpowers.implementer` and `superpowers.code-quality-reviewer`.

So a composite pack that imports the upstream packs and is itself imported by
the city rewrites every one of those targets to `d2b.<name>`. Formulas still
compile, because run targets are not validated at compile time; the run then
stalls on its first dispatch. A second failure mode is name collapse: two
imported packs sharing a local agent name collapse to one qualified name, which
per the pack spec fails city loading outright.

The shape that works is the one `superpowers/pack.toml` itself uses: the
**city** imports each pack under its canonical binding, as siblings. Cross-pack
`extends` still works because formula names resolve globally.

### What the engine does not have

Four measured absences shape D6, D7 and D9, and the earlier revision assumed
past all of them:

- **No ask-human watcher.** `[steps.gate]` creates a real blocking bead, but
  its `type` vocabulary (`human`, `mail`, `gh:pr`) is doc-comment only: the
  parser does not validate it and no bundled watcher acts on it. Zero bundled
  formulas use `gate`. A gate bead blocks until something explicitly closes it,
  which today means `gc bd close <bead-id>`.
- **`until` is not a loop.** It expands exactly one iteration at compile time.
- **`check` is the only engine loop**, and it must not be combined with `gate`,
  `loop`, `expand`, `assignee` or `retry`.
- **`extends` drops composition rules.** Per formula-spec-v2 section 1.7,
  `advice` and `pointcuts` are dropped entirely, and when both parent and child
  declare `compose`, then `branch`, `gate` and `aspects` are dropped from
  **both** sides. A methodology formula that extends a base and declares
  `compose.gate` for human approvals loses them silently at compile time. That
  is the same fail-open class as the spec-kit `d1e86f6` bug this record cites
  elsewhere, and it is why D6 uses explicit gate steps with `needs` edges.

An earlier revision cited `internal/session/waits.go` as the gate mechanism.
That is the **session** wait subsystem, not the formula gate bead; the citation
was wrong and is corrected here.

**Formula names are global and last-wins**, staged as symlinks under
`.beads/formulas/`; only agents are binding-qualified. A d2b formula named
`review` would silently shadow `gascity/formulas/review`, making
`build-base`'s `code_review_formula` default resolve to ours for every run,
with no error. Hence the `d2b-` prefix rule and the winner doctor check in D5.

### The panel gate: envelope neutral, transport not

`PanelRecord` is a strict fourteen-field struct and `validate` checks it
against the stored `PanelRequest`; nothing in either cares who produced it.
What is not neutral is ingestion: `panel-attest` takes `--records DIR`,
`read_record_dir` requires a directory, `record_file_name(role)` fixes
`<role>.json`, and `PanelRequest.record_files` enumerates those names.

Three further facts bound D8. `validate_record_set` **refuses any set
containing a finding**, so the gate has no notion of a round and only ever sees
a final unanimous set. It enforces **distinct** `run_id`, `receipt_locator` and
`output_sha256` across the ten records. And `PanelRequest` is
`deny_unknown_fields` with no round ordinal, prior tip or evidence reference,
so a "round manifest" would be a new artifact class rather than a transport
adapter.

**Receipt checking today is purely syntactic.** `validate_receipt_locator`
requires the string to start with `"{provider}://"` and contain no control
characters. There is no resolution, no trusted source and no receipt store.

Separately, `.scratch/panel/<round>/` working files are **not read by the gate
at all**; they feed `make-records.mjs`, which emits the records the gate reads.

### Delivery state is single-uid and outside every Git tree

`verify_anchored_directory` requires mode `0700` and `st_uid == geteuid()` for
every delivery state directory, artifacts are `0600` with an owner check, and
`StateRoot::prepare` refuses any root inside an enclosing Git working tree,
walking ancestors for a `.git` entry. The check is uid equality, not
`access(2)`, so there is no group-mode escape.

Two consequences the earlier revision missed. A separate publisher uid
**cannot read** the seal and panel records, so it cannot render a PR body from
them. And if the city directory is the config-repo checkout, no path beneath it
can host delivery state, including `.gc/`.

### Runtime, control plane, supervisor, publisher, Discord

- **Runtime.** The supported model is the `tmux` session provider, which is
  both default and registered fallback, with work in host git worktrees.
  Upstream's own trust document says operator-configured commands "are a
  feature, **not a sandbox**". No per-session uid option exists in any surface
  reviewed. So agents run as the Gas City service identity, and any claim of a
  separate worker uid is false unless the `exec:` provider plus a
  privilege-dropping wrapper is proven (P8).
- **Control plane is loopback.** The supervisor API is HTTP on
  `127.0.0.1:8372`; the `bd` beads provider resolves a Dolt server **TCP**
  port. Agent steps are literally instructed to run `gc bd mol current`,
  `gc bd update --notes`, `gc bd close`, `gc discord reply-current`. Any
  confinement that blocks host loopback stops every step from recording its
  result.
- **Supervisor is a systemd USER unit.** `gc` installs, enables, kills and
  restarts `gascity-supervisor.service`, or
  `gascity-supervisor-<base>-<hash>.service` for a non-default `GC_HOME`, and
  restarts it itself on binary drift. There is no `gascity-orchestrator`
  binary or unit upstream. Version 1.4.0's upgrade notes require **one
  store-scoped `control-dispatcher` per graph-owning scope**, or a rig-owned
  graph fails before instantiation.
- **Publisher.** The `github` pack's `push-branch` and `create-pr` require a
  GitHub **App**: `--installation-id` plus an `app` config carrying `app_id`
  and `private_key_pem`, at `<city>/.gc/services/github/data/config.json`,
  mode `0640`, shared with the webhook and admin services. `git_push_branch`
  runs `git push <url> <ref>:refs/heads/<branch>` with **no `-C`**, so it is
  cwd-dependent, and `runDiscoveredCommand` execs pack commands with no
  `cmd.Dir`.
- **Discord.** `discord-interactions` has publication visibility `public` and
  requires inbound public HTTPS. The gateway path delivers normalized events
  **into named agent sessions**, and agents reply with `gc discord publish`.
  Neither is a non-agent approval ingress.

### `.gc/` is used by upstream, and drains cap at 100

Task worktrees are linked worktrees under `<repo>/.gc/worktrees/<id>`; upstream
check steps point at `.gc/scripts/checks/build-artifact-valid.sh`; and
`mol-pr-ship` writes `.gc/pr-pipeline/ship/<branch>.md` after `cd`-ing to the
repo top level. A blanket prohibition on `.gc/` inside the rig contradicts
normal operation.

`drain` scatters a convoy into one-member unit convoys with `max_units` capped
at **100**, reporting `limit_exceeded` beyond that, and in `separate` mode
creates all item roots in parallel. Dependency edges come only from `needs` and
`depends_on` resolved at compile time, so a drain does not represent an
arbitrary imported DAG.

### Spec Kit artifacts are a prose convention

Commands are namespaced `speckit.*`. A `tasks.md` line looks like
`- [ ] T014 [US1] Implement ... (depends on T012, T013)`. Task ids and phase
headers are reliable; dependencies are parenthetical prose, file paths are
unmarked prose, there is no schema and no format version. `templates/commands/
tasks.md` declares a handoff to `speckit.implement`, so substituting the
executor is a supported shape.

### What we leverage upstream, pinned

Commit-pinned URLs at the commits above. The right-hand column names what we
consume and what we do not inherit; anything absent is not inherited by
default. Where the deployed binary is v1.4.0 rather than `main`, P7 governs.

| Component | Pinned URL | What we use, and what we do not inherit |
| --- | --- | --- |
| Build contract | [`gascity/formulas/build-base.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/build-base.formula.toml) | The full-lifecycle stage set and every extension seam: `planning_formula`, `decomposition_formula`, `implementation_formula`, `code_review_formula`, `review_fix_formula`, `interaction_mode`, `review_mode`, `push`, `open_pr`. We override seams; we do **not** re-implement stages. |
| Plan entry route | [`gascity/formulas/build-from-plan-base.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/build-from-plan-base.formula.toml), [`build-from-plan.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/build-from-plan.formula.toml) | Route A and Route B extend the `-base` variant. |
| Decompose entry route | [`gascity/formulas/build-from-decompose-base.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/build-from-decompose-base.formula.toml) | Route C extends this, skipping requirements and plan. |
| Requirements entry route | [`gascity/formulas/build-from-requirements-base.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/build-from-requirements-base.formula.toml) | Available if Route A is ever driven from requirements rather than a plan. |
| Review-fix loop | [`gascity/formulas/fix-loop-base.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/fix-loop-base.formula.toml) | `d2b-panel-fix` extends it for panel fix rounds. We do **not** invent a fix loop. |
| Publication | [`gascity/formulas/publish.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/gascity/formulas/publish.formula.toml) | The `publish` contract, its `push` and `open-pr` steps and the `gc.publisher` role. We override the command bodies per D9; we do **not** inherit the `github` pack's App requirement by default. |
| Pack format and imports | [`docs/reference/specs/pack-spec.md`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/docs/reference/specs/pack-spec.md) | `[imports.<binding>]`, reserved directory layout, binding namespace. We do **not** rely on `version` or `requires_gc`, which are parsed and not enforced. |
| Binding stamping | [`internal/config/pack.go`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/internal/config/pack.go#L770-L785) | The reason D5 forbids nesting: city-level imports override nested bindings. |
| Formula semantics | [`docs/reference/specs/formula-spec-v2.md`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/docs/reference/specs/formula-spec-v2.md) | `extends`, step `expand`, `check` as the only engine loop, and section 1.7's rule that `compose.branch`, `gate` and `aspects` are dropped when both sides declare `compose`. |
| Supervisor lifecycle | [`cmd/gc/cmd_supervisor_lifecycle.go`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/cmd/gc/cmd_supervisor_lifecycle.go), [`cmd/gc/drift.go`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/cmd/gc/drift.go) | The real unit name, user scope and self-restart-on-drift behaviour D12 integrates with. |
| Control plane | [`internal/supervisor/config.go`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/internal/supervisor/config.go), [`cmd/gc/cmd_commands.go`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/cmd/gc/cmd_commands.go) | Loopback `:8372` and the Dolt TCP port that D11 must not block; also `runDiscoveredCommand` having no `cmd.Dir`. |
| Trust posture | [`docs/reference/trust-boundaries.md`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/docs/reference/trust-boundaries.md) | The statement that configured commands are "a feature, not a sandbox", which D10 quotes rather than contradicts. |
| Pack pinning | [`internal/packman/lockfile.go`](https://github.com/gastownhall/gascity/blob/38ed358fae0e8238834eb778a23a664fe4cb8954/internal/packman/lockfile.go) | `packs.lock` recording `version`, `commit`, `fetched`. The `commit` is the durable pin. |
| Implementation discipline | [`superpowers/formulas/superpowers-development.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/superpowers/formulas/superpowers-development.formula.toml), [`superpowers/pack.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/superpowers/pack.toml) | Used as `implementation_formula`. `pack.toml` is also the worked example of sibling importing. We do **not** inherit a per-task simplification step; there is none. |
| Optional prefilter | [`compound-engineering/formulas/compound-code-review.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/compound-engineering/formulas/compound-code-review.formula.toml) | Optional non-binding prefilter only. Never the binding panel: 17 selector-gated lanes, no per-lane model or effort, no unanimity, no seat identity. |
| Readiness | [`pr-pipeline/formulas/mol-pr-ship.formula.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/pr-pipeline/formulas/mol-pr-ship.formula.toml) | Optional simplify and readiness reporting. It stops at a report and publishes nothing, and it writes under `.gc/pr-pipeline/`. |
| GitHub publishing scripts | [`github/scripts/github_intake_common.py`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/github/scripts/github_intake_common.py), [`github/commands/push-branch.sh`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/github/commands/push-branch.sh), [`github/commands/create-pr.sh`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/github/commands/create-pr.sh) | Read as the reference implementation and as evidence of the App requirement and cwd dependence. D9 chooses a PAT path for v1 unless P4 says otherwise. |
| Discord topology | [`discord/README.md`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/discord/README.md), [`discord/pack.toml`](https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/discord/pack.toml) | Outbound notification and Q&A only in v1, per D13. We do **not** treat it as an approval controller. |
| Spec Kit planning | [`templates/commands/specify.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/commands/specify.md), [`clarify.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/commands/clarify.md), [`plan.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/commands/plan.md), [`tasks.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/commands/tasks.md), [`analyze.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/commands/analyze.md) | The planning chain. We do **not** use [`implement.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/commands/implement.md). |
| Spec Kit task format | [`templates/tasks-template.md`](https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/templates/tasks-template.md) | The id and marker conventions D15's importer parses. No schema exists. |
| Gas City package | [`packages/gascity/package.nix`](https://github.com/numtide/llm-agents.nix/blob/7b99fc4bbb8a7c2fff82c2708d8636c1cbc65661/packages/gascity/package.nix) | The `gc` binary and its wrapped PATH. Records that it builds v1.4.0, sets `main.commit=nixpkgs`, disables checks, and omits `python3`. |
| d2b panel policy | [`packages/xtask/src/delivery/model.rs`](https://github.com/vicondoa/d2b/blob/b5881b2a6a42cd6e4db89c662c9df853f6a98d55/packages/xtask/src/delivery/model.rs) | `PANEL_ROLES` and the pinned provider, model and effort policy, unchanged by this record. |
| d2b panel gate | [`packages/xtask/src/delivery/panel.rs`](https://github.com/vicondoa/d2b/blob/b5881b2a6a42cd6e4db89c662c9df853f6a98d55/packages/xtask/src/delivery/panel.rs) | `PanelRecord`, `PanelRequest` and `validate_record_set`, all preserved byte-identical by D8. |
| d2b delivery storage | [`packages/xtask/src/delivery/storage.rs`](https://github.com/vicondoa/d2b/blob/b5881b2a6a42cd6e4db89c662c9df853f6a98d55/packages/xtask/src/delivery/storage.rs) | The `0700` and uid-equality checks and the outside-every-Git-tree rule that D10 designs around. |
| d2b panel skill | [`.github/skills/d2b-panel-round/SKILL.md`](https://github.com/vicondoa/d2b/blob/b5881b2a6a42cd6e4db89c662c9df853f6a98d55/.github/skills/d2b-panel-round/SKILL.md) | The standalone producer, which stays supported. Its working-file layout is not a contract. |

## Decision

**D1. Gas City is contributor infrastructure and acquires no d2b product
surface.** No flake output, no `d2b.*` option, no `nixos-modules/` content, no
manifest field, schema, wire message, broker op or CLI verb, nothing under
`docs/{reference,how-to,explanation}/` or in `README.md`, no critical-subsystems
row. Mentions are permitted in contributor surfaces including changelog prose;
what is forbidden is framing Gas City as a d2b capability. M1 scans the product
surfaces and states plainly which half a reviewer must judge.

**D2. No d2b component runs, hosts or isolates any part of this system.** Not
`d2bd`, the broker, microVMs, any Provider, Runtime, Zone, Resource or Service,
the guest-control transport, or the Credential service. A d2b-backed sandbox is
out of scope and requires its own ADR. Contributor tooling does not consume the
framework it is used to develop, so a regression in d2b's VM lifecycle cannot
block the workflow used to fix it.

**D3. Gas City is opt-in, and the standalone surface stays first-class.** The
thirteen agents, the five d2b skills and the `speckit-*` skills remain usable
and supported for contributors who never touch Gas City. Nothing may delete,
rename, gate or condition them, and none may acquire a Gas City dependency.

**First-class does not mean frozen.** D8 requires the standalone panel to
submit a resolvable `StandaloneHarnessReceipt`, which the current skill does
not capture. That is a **clean break, delivered atomically**: the same
implementation change updates the skill and its adapter to capture the receipt
locator automatically, adds a preflight that requires a supported harness
resolver, and makes the panel **refuse before dispatch** when one is absent.
There is no legacy acceptance path and no warning-and-continue, because a
panel that runs ten seats and then discovers it cannot attest has wasted the
expensive part and produced nothing.

Existing users get a cutover, not a deprecation window. This is contributor
tooling with one operator, so no window is required; what is required is that
the remedy is explicit. The preflight has one operator spelling, `make panel-preflight`, which runs the
existing binding checks and the harness receipt resolver and version checks
together. `scripts/copilot/check-bindings.mjs` remains what the target invokes;
it is no longer a second operator-facing spelling of the same step, because two
names for one preflight is how a contributor ends up running the older one. Its
failure names the unsupported or missing resolver and the pinned Copilot CLI
version to upgrade to.

**D4. Four layers of ownership.**

- **`numtide/llm-agents.nix`** supplies the `gc` package. Pinned by the config
  repo; never a d2b flake input.
- **`vicondoa/gascity.nix`** owns the generic reusable NixOS module: service
  wiring around Gas City's real supervisor topology, state and lock paths,
  egress policy primitives, dashboard posture, health checks, module tests, and
  the package option including the D19 override. It contains **nothing
  d2b-specific**: no d2b name, path, option, default, formula, panel concept or
  Discord identity. Its option namespace is rooted at `services.gascity`.
- **`vicondoa/d2b-gascity-configs`** owns the d2b instance and policy layer:
  the city `pack.toml` and its sibling imports, `packs.lock`, the
  `d2b-engineering` methodology pack, the rig binding, Discord identity values,
  the d2b egress allowlist, approval policy, and secret references by name.
- **The host's `/etc/nixos`** owns machine-private instantiation: secret
  values, uid assignment, local paths, and the import of the config module.

**`vicondoa/d2b`** owns this record, the follow-on contributor documentation,
the Spec Kit artifacts under `specs/`, and the delivery gate in
`packages/xtask/src/delivery/` including D8's stdin path. Those are contributor
tooling, not product surface. Neither flake is ever a d2b input.

This record authorizes creating `vicondoa/gascity.nix` and
`vicondoa/d2b-gascity-configs` later; it does not create them now.

**D5. The city imports packs as siblings; `d2b-engineering` is thin.** The city
`pack.toml` imports, under canonical bindings, `[imports.gc]` for the gascity
pack, `[imports.superpowers]`, and optionally `[imports.compound-engineering]`,
`[imports.pr-pipeline]` and `[imports.discord]`, **alongside**
`[imports.d2b]` pointing at `d2b-gascity-configs/packs/d2b-engineering`. Every
import is pinned by `commit` in `packs.lock`, because import `version` is
parsed and not enforced.

`d2b-engineering` must **not** import the upstream packs and must not be a
transitive wrapper. City-level stamping would rewrite `gc.run-operator`,
`superpowers.implementer` and every other qualified target to `d2b.<name>`,
which compiles and then fails at first dispatch; and two imported packs sharing
a local agent name would collapse and fail city loading outright. The operator
entry surface is delivered instead by `[catalog]` entries and one thin
`commands/` wrapper, which costs nothing and breaks nothing.

Two lints are mandatory in the pack's `doctor/`, both consequences of measured
engine behaviour: **every d2b formula name is prefixed `d2b-`**, and a check
asserts the resolved global winner for each, because formula names are global
and last-wins and an unprefixed `review` would silently become
`build-base`'s default `code_review_formula` for every run. The second lint
rejects `compose.branch`, `compose.gate` and `compose.aspects` anywhere in the
pack, because `extends` drops them from both sides.

**D6. Routes are overrides of the upstream build formulas, and gates are
explicit steps.** `d2b-engineering` supplies methodology, not lifecycle:

| Formula | Extends | Sets |
| --- | --- | --- |
| `d2b-build` | `build-from-plan-base` | `implementation_formula = superpowers-development`, `code_review_formula = d2b-panel`, `review_fix_formula = d2b-panel-fix`, `interaction_mode`, `review_mode`, `push`, `open_pr` |
| `d2b-build-from-adr` | `build-from-plan-base` | Same, plus the ADR admitted as the approved plan artifact |
| `d2b-bugfix` | `build-from-decompose-base` | Same, with requirements and plan absent |
| `d2b-panel` | none | A `check` loop over the seats, per D7 |
| `d2b-panel-fix` | `fix-loop-base` | Panel findings as `findings_path` |

- **Route A, feature.** Prompt text, target rig, base branch; optional
  constraints, issue, cited ADRs. Full Spec Kit planning through the adapter,
  producing `specs/NNN-slug/`, then the plan route.
- **Route B, Accepted ADR.** The ADR is admitted as the approved plan artifact.
  Admission is fail-closed: `Status: Accepted` required, content digest
  recorded as a run var, and a `check` script re-verifying the digest so a
  changed ADR fails the run rather than being silently re-interpreted. A
  decomposition is still produced, because an ADR decides architecture and not
  task breakdown; it may not contradict the ADR, and a clarification whose
  answer would change a decision is an escalation.
- **Route C, bug.** Issue or defect description, affected area, optional
  reproduction. It skips requirements and plan by entering at decompose, so it
  produces **no product spec**. Escalation to Route A or to an ADR is mandatory
  on a closed list: requirement ambiguity, architecture change, trust-boundary
  change, schema or public-contract change, or unbounded scope.
- **Route D, resume.** Not a formula. Re-attachment to the existing workflow
  root by `gc.graphv2_root_key`, plus `gc formula version-check <bead-id>` for
  formula drift and id-keyed task re-import. An earlier revision claimed
  `pour = true` was the opt-in for durable resumability; that is false for
  formulas v2, which materialises root, step and control beads on
  instantiation, and no upstream v2 formula sets `pour`.

**Human gates are explicit `[steps.gate]` beads with `needs` edges**, never
`compose.gate`. There is no bundled watcher for gate types, so a gate blocks
until closed and the closing action is the operator's, per D13.

**D7. Gas City orchestrates the binding panel, and binding assurance comes from
the dispatch record.** For an opted-in run Gas City owns orchestration
end to end: it dispatches the ten seats, drives the rounds, supplies each seat
its snapshot, diff and validation context, collects verdicts, and blocks
progression until the roster is unanimous. It does not own semantics, which are
unchanged: closed ten-role roster, independent seats, pinned provider, model
and effort policy, validation evidence supplied rather than re-run, delta
review after round one, content change invalidating prior sign-off, `signoff`
true iff `recommendations` is empty, unanimity, records bound to the same
`candidate_id`, `content_id` and `snapshot_sha256`, and no lane attesting its
own work.

**Mechanically the panel is a `check` loop**, because `check` is the only
engine loop and cannot be combined with `gate`, `loop`, `expand`, `assignee` or
`retry`. `d2b-panel` is one step whose `[steps.check]` carries `max_attempts`
and whose exec mode invokes the admission script; the ten seats are dispatched
beneath it. Fix rounds use `d2b-panel-fix` extending `fix-loop-base`. Round
history lives in beads. P2 proves this shape or routes to an external driver
invoked from a single step.

**Binding assurance is derived, not asserted.** An earlier revision made
unattended orchestration conditional on cryptographically trusted receipts,
which contradicted the requirement that Gas City orchestrate the panel in the
first version: every run would park and the deployment would satisfy one
decision by violating the other. That rule is withdrawn. Instead:

- All ten seats are dispatched through **one provider profile owned by the
  config repo**, so the intended binding is configuration rather than a claim.
- The admission tool **re-derives** `provider`, `model_version` and
  `reasoning_effort` from the **orchestrator's own dispatch record**, an
  immutable bounded record produced at the Gas City dispatch boundary, and not
  from producer-supplied JSON. A producer string is never accepted as proof of
  binding.
- `receipt_locator` is retained as provenance. This record does **not** demand
  a per-request cryptographic receipt from the model provider, because no such
  artifact is known to exist for `github-copilot` and requiring one would make
  the design unimplementable.
- If the deployed Gas City cannot export such a dispatch record, that export is
  a **d2b-specific adapter this record authorizes building**, and it is
  **P2's** exit condition. There is no fail-open: until the adapter exists, the
  Gas City panel path is not shipped and the standalone skill remains the
  supported producer with its existing observed-harness receipt path. What is
  forbidden is shipping a Gas City panel that accepts self-asserted bindings.

Residual risk, stated: a producer that fabricates a dispatch record defeats
this. The structural defence is unchanged, that the panel model is deliberately
not the coding model, so a lane cannot both author and attest.

**Orchestration is not authority, and one narrow protected boundary separates
them.** Gas City sessions all share one uid (D10), so a task or reviewer agent
can write anything another session could write. If dispatch attestations,
approvals, panel summaries or publisher handoffs were session-written state, an
agent could forge all four and the gate would be decoration. The expert
simplification that removed the previous multi-principal design was right about
the mechanisms and wrong to leave nothing in their place; this restores the
smallest boundary that closes the forgery path, and no more.

`d2b-gascity-configs` owns a **panel-and-approval controller**: a small service
running under its **own uid, distinct from the Gas City session identity**. It
is not in generic `gascity.nix`, which stays d2b-free, and not in d2b product
code. Gas City formulas and check scripts reach it through a
**peer-authenticated Unix socket**, and the controller rejects any peer whose
credentials are not the expected orchestrator identity.

It exposes **three separate endpoints, each with its own closed operation set**,
because one socket with one authorization check would let the orchestrator peer
reach approval operations and the shared uid would make that an agent-reachable
path:

| Endpoint | Authenticated peer | Permitted operations | Everything else |
| --- | --- | --- | --- |
| Orchestrator | `gascity` uid | Request panel dispatch; read dispatch and gate status | Refused; approval operations are **absent from this endpoint**, not merely denied |
| Operator | operator uid | Submit a decision from the closed set; read status | Refused |
| Publisher | `publisher` uid | Consume the bound publication manifest | Refused |

An agent sharing the `gascity` uid therefore cannot issue an approval even by
speaking the protocol correctly, because the operation does not exist on the
endpoint its identity can reach.

It does exactly three things:

1. **Dispatch.** It launches or authorizes panel dispatch under a fixed
   provider, model and effort profile that it owns, and emits the trusted
   dispatch record. If the seats must themselves run as Gas City sessions, the
   trusted binding still comes from the controller's own dispatch, never from
   session-written state. P2 must prove that; if it cannot, the Gas City panel
   does not ship.
2. **Approval.** It receives operator decisions on the operator endpoint and
   writes a protected append-only approval and audit record.
3. **Manifest.** It emits the canonical panel and publication manifest, bound
   to the exact integration commit, carrying the panel identifiers and signoff
   and the approval digest.

**Gas City beads mirror blocking state; they are not approval authority.** A
gate bead reflects that something is waiting and unblocks when the controller
says so. An agent that closes a bead has closed a bead, not manufactured an
approval, because the publisher and the gate both verify the controller's
record rather than the bead's state.

Required properties, with mechanism deferred to the spec: a distinct uid; peer
identity checks on every call; append-only, write-once records; binding to an
exact commit; and **no signing or approval key readable by any agent**. The
detailed store layout, retention algorithm and sync sequence are spec-level and
are deliberately not frozen here, which is the mistake the previous revision
made in the other direction.

**D8. Panel ingestion gains two typed inputs, and nothing else changes.**

`PanelRecord` and `PanelRequest` are preserved byte-identical, including
`deny_unknown_fields`. Two neutral inputs are added beside the existing
`--records DIR`:

- `--records-stdin`, a record stream; and
- an **injectable trusted dispatch boundary**, `--dispatch-record PATH` or a
  file descriptor, or `--dispatch-receipt-stdin`, carrying the controller's
  dispatch record for the Gas City producer.

No host path is hardcoded; the boundary is injected as **bounded message bytes
on stdin or over the authenticated socket**, so tests supply fixtures. A path
variant remains available only if the path is controller-owned and resolved
anchored and fd-relative per D20 under all three `openat2` resolve flags, and
descriptor passing follows D9's discipline if an implementation reaches for it
at all.

**Two variants of trusted binding evidence, one verifier.** The verifier
accepts exactly two forms and rejects everything else:

1. **`GasCityControllerDispatch`** - the record or receipt emitted by D7's
   controller when it owned the dispatch.
2. **`StandaloneHarnessReceipt`** - an **opaque harness-issued run or receipt
   locator**, resolved through an authoritative harness or session receipt
   resolver that returns the binding. The adapter may carry the locator; it may
   **not** mint trust from model or effort strings a human typed. A locator
   that cannot be resolved is a typed error and the submission fails closed.

An earlier revision accepted an "operator-recorded observation" here. That was
wrong: a value a person transcribes into an adapter is a self-asserted string
with a longer story attached, and the verifier cannot tell the two apart. The
replacement moves the trust to whatever the harness itself issued and to a
resolver that can be asked, so nothing in the path depends on the honesty of
the transcription.

The standalone producer therefore remains fully functional with **no Gas City
and no controller**, which D3 requires, but it does require an authoritative
resolver. The standalone skill and its adapter are updated to capture the
harness-provided receipt locator **automatically** rather than prompting for
it, which is the change that keeps D3 satisfied without reintroducing a typed
string.

Both variants converge on the same typed verifier and then on
`validate_record_set`. **Raw producer strings are rejected for both**: a
record's own claim about its binding is never evidence, whichever producer
wrote it.

**The standalone cutover is gated before dispatch, not after.** The panel
refuses to dispatch its seats when no supported resolver is present, so the
failure costs a preflight rather than ten reviews. Partially running the panel
and failing at admission is specifically what this gate exists to prevent.

**Resolution goes through an injected interface, not a hardcoded process.**
Preflight and admission both take a `HarnessReceiptResolver` as a parameter:
an interface that maps an opaque receipt locator to a resolved binding or to a
typed failure. The production adapter implements it against the harness or
session resolver; tests inject a mock. Nothing in the verifier opens a fixed
socket path or shells a fixed process, because a hardcoded dependency is
untestable exactly where the security property lives, and every negative case
below would otherwise need a real broken harness to reproduce.

**Failures are distinct variants, not one invalid-state error**, because the
remedies differ and a single error forces the contributor to guess which one
applies:

| Variant | Remedy it names |
| --- | --- |
| `HarnessResolverMissing` | Install or configure a supported resolver; run the preflight |
| `HarnessVersionUnsupported { current, supported }` | Upgrade to the pinned Copilot CLI version, both values shown as parsed versions |
| `HarnessReceiptUnresolvable` | The locator did not resolve; re-run the round so the harness issues a fresh receipt |
| `HarnessReceiptBindingMismatch` | The resolved binding contradicts the record; the panel must be re-run under the pinned binding |
| `SelfAssertedBindingRejected` | A record supplied binding strings instead of a locator; update the adapter to capture the receipt |

**The preflight is one repository command: `make panel-preflight`.** It runs
the existing binding checks together with the harness receipt resolver and
version preflight, and it is what must pass **before any seat is dispatched**.
Contributors get one command to remember rather than a list, and every error
variant below names it.

Every variant renders an **exact** remedy as a sequence of fixed repository
commands. A message saying "update the adapter" is not a remedy; the
contributor has to know which command, in which order.

**No error prints the panel invocation or its arguments.** An earlier revision
required the exact active `/d2b-panel-round <mode> ...` line to appear in the
error text. That is withdrawn: argv carries paths, branch names and free-form
operator input, and an error is copied into terminals, issues and logs, so
printing it reintroduces exactly the values D17 excludes from every other
surface. Resumption is instead addressed by the bounded correlation alias.

`make panel-resume PANEL_REF=<alias>` reads protected local panel state and
replays the stored invocation **without printing it**. The alias has the same
fixed grammar and bounded length as D17's correlation alias, so it is safe in
an error, a log line and a terminal, and it is actionable only through the
protected mapping.

- `HarnessResolverMissing` and
  `HarnessVersionUnsupported { current, supported }`: install or upgrade to the
  pinned supported Copilot CLI per the repository setup documentation, then
  `make panel-preflight`, then
  `make panel-resume PANEL_REF=<alias>`.
- `HarnessReceiptUnresolvable` and `HarnessReceiptBindingMismatch`:
  `make panel-preflight`, then `make panel-resume PANEL_REF=<alias>`.
- `SelfAssertedBindingRejected`: `make panel-migrate`, then
  `make panel-preflight`, then `make panel-resume PANEL_REF=<alias>`.

**`make panel-migrate` is a wrapper that fails closed**, not a documented pair
of git commands. It brings the checked-out standalone skill and adapter to the
pinned supported revision, and it **refuses** rather than proceeding when the
working tree is dirty or when the update would conflict, reporting which of the
two it hit and leaving the tree untouched. Telling a contributor to run
`git rebase origin/v3` inside an error message invites exactly the
half-finished rebase that then fails the preflight for a second, unrelated
reason; a wrapper that refuses on a dirty tree keeps the operator surface
stable and the failure legible.

**The whole error enum carries a custom redacting `Debug`, not just one
variant.** A derived `Debug` on the enum leaks whatever any variant happens to
hold today and, worse, whatever a field added next year happens to hold;
protecting one variant leaves the others one refactor away from a leak. So the
entire panel receipt error enum implements `Debug` explicitly and redactingly,
and `Display` exposes only a closed reason, the bounded alias, the fixed
remedy commands, and bounded safe version newtypes where the variant has them.

`HarnessVersionUnsupported` in particular carries `current` and `supported` as
bounded parsed version newtypes or equally closed-safe fields, never raw
strings. A version probe shells an external binary, so its stdout is
attacker-influenced in exactly the deployment where the harness is wrong.

No round manifest is added to xtask. The gate refuses any set containing a
finding and has no round concept, so **rounds stay in the producer**: Gas City
keeps round and fix history in beads and submits only the final unanimous set.
An optional round-history artifact may be stored alongside and digested by the
seal, but the gate does not interpret it. The `<role>.json` directory remains
the standalone skill's adapter. Adapters translate shape only; none may supply
a default, relax a policy value, infer a seat, reuse a record across rounds, or
accept a mismatched binding, and every refusal is identical from every producer.

**Snapshot and request generation follow the Git candidate**, never authorship
or tool history. Absent panel evidence means "panel required and not yet
satisfied", never "unsupported changes", and the message names the active
route, the producer, the unmet condition and the next action, including the
concrete `gc bd close <bead-id>` where a parked gate is what blocks.

**D9. Publication is required in v1, and its mechanism is chosen for one
operator.** After the artifact-bound approval of D17, publication runs through
the upstream `publish` formula and the `gc.publisher` role, with `push` and
`open_pr` set only at that point.

The command bodies are overridden rather than inherited. The `github` pack's
`push-branch` and `create-pr` require a GitHub App with `app_id` and
`private_key_pem` in a `0640` file shared with the webhook and admin services,
and `git_push_branch` runs without `-C` while `runDiscoveredCommand` sets no
`cmd.Dir`, so an unset working directory pushes from whatever repository the
caller happened to be in. For a single-operator deployment v1 uses instead: a
**repo-scoped fine-grained PAT** delivered only to the publisher unit via
`LoadCredential=`; an explicit handoff repository; the full-sha push command
specified below; and create-or-edit PR handling through `gh`.
**P4 has no App fallback.** If P4 fails, the publishing
design is blocked and must be redesigned through a new review; it must not
silently adopt the `github` pack App path while still claiming credential
isolation, because that path shares a `0640` key file with the webhook and
admin services and cannot be publisher-exclusive and functional at once. If an
App is ever chosen, every service using the key must be isolated and D10 is
rewritten.

**The publisher verifies independently and is idempotent.** It does not trust
claims from the Gas City caller. Before acting it verifies the controller's
bound publication manifest and the exact approval against the commit it is
being asked to push, and refuses on any mismatch.

**Two push paths, neither of them a bare force.** An unconditional `+` would
let a concurrent change on the remote be silently discarded, so the publisher
distinguishes the two cases it can actually be in:

- **Initial branch.** No remote branch exists, so the push is plain:

  ```
  git -C <handoff-repo> push origin <full-sha>:refs/heads/<branch>
  ```

- **Existing PR branch, after a rebase or amend.** The publisher does **not**
  read remote state and call whatever it finds "expected": a fresh read races
  the very concurrent push the lease exists to catch, and would happily lease
  against someone else's commit. The expected value comes from the
  controller-bound manifest, which carries **`expected_previous_remote_sha`**
  captured from the known PR branch state *before* the replacement candidate
  was produced. The publisher uses that exact value:

  ```
  git -C <handoff-repo> push origin <full-sha>:refs/heads/<branch> \
    --force-with-lease=refs/heads/<branch>:<expected-old-sha>
  ```

  If the remote has moved since the manifest captured that value, the lease
  fails and the publisher **refuses** rather than retrying with a wider hammer.
  If the manifest carries no `expected_previous_remote_sha` at all, a
  non-fast-forward update is **refused outright**: absent expected state is not
  permission to force.

  **A refusal is actionable, and it never suggests forcing.** The lease-failure
  error directs the operator to `d2b-gc publish status <run-id>`, which shows
  the manifest's expected sha, the current remote sha, and the commits that
  landed between them. The remedy is to reconcile or rebase through the normal
  integration route, which produces a new candidate and a new approval, and
  then to rerun `d2b-gc publish <run-id>`. At no point does the output offer a
  force without a lease, because the whole purpose of the refusal is that
  something arrived which nobody has reviewed.

  The missing-expected-state error is separately actionable: it names how to
  regenerate a publication manifest from the current approved candidate, so an
  operator whose manifest predates the branch is told to produce a current one
  rather than left to guess.

**A crash between push and pull request is recoverable without forcing.** Before
pushing, and again after a lease failure, the publisher reads the remote
branch's current target and compares it against two values it already holds:

- **already `<full-sha>`** - the push completed before the crash. The publisher
  treats the push as done and continues to create-or-edit the pull request.
- **the manifest's `expected_previous_remote_sha`** - the branch is where the
  manifest expected, so the leased update proceeds.
- **anything else** - a stale lease refusal with the remedy above.

This read recognises an already-completed target; it is **never** used to
manufacture a new expected sha. The distinction is the whole point: adopting
whatever the remote currently holds as "expected" is the fresh-read mistake D9
rejects, while recognising that the remote already holds the exact sha we were
asked to publish is idempotency.

Retry is safe in both paths. The publisher looks for an existing pull request
by head branch, creates one if absent and edits the body if present, so a
retried publication converges rather than failing or duplicating.

**The body is transferred as bounded bytes, not named.** A caller-supplied body
path is a time-of-check-to-time-of-use hole: the file can be swapped between
verification and read. The controller sends the bounded manifest and body to
the publisher as **message bytes over the authenticated socket or on stdin**,
which is the preferred form because it has no file to substitute and no
descriptor to mismanage.

Descriptor passing is not required and should not be reached for by default. If
a chosen implementation does pass one, then exactly **one** descriptor is
passed; the sender opens it safely and **closes its own copy immediately after
`sendmsg` returns, unconditionally, on success and on failure alike**; the
receiver takes it with `MSG_CMSG_CLOEXEC` and owns and closes it; and unbounded
descriptor reception is refused.

**A refusing receiver still owns what arrived, on every refusal path.** The
receiver takes ownership of every descriptor the moment `recvmsg` returns, by
wrapping each into an owning type or an explicit guard **before** any
validation runs. Every subsequent refusal then closes them all: an unexpected
descriptor, a duplicate, more than one, and equally a **malformed or invalid
payload arriving alongside an otherwise valid expected descriptor**.

That last case is the one a natural implementation misses. Validating the
descriptor set first and the payload second makes the payload check an early
return past a descriptor nobody owns yet, so a `?` on a parse error leaks it.
Owning first and validating second removes the class rather than adding another
check. Refusing without draining leaks precisely the descriptors an attacker
chose to send, which turns a validation check into a resource exhaustion path.

The unconditional close matters more than the success path does. Ownership
transfers only if the descriptor was actually received, so a `sendmsg` that
fails leaves the sender holding the only copy, and a sender that closes solely
on success leaks one descriptor per failure. Failures are exactly the case that
repeats. Neither side accumulates descriptors across repeated transfers,
successful or failed, which is a property a test can count rather than a
discipline a comment can request.
This record does not claim that setting close-on-exec after creation is atomic,
because it is not. A path variant remains possible only if the path is
controller-owned and resolved anchored and fd-relative per D20, meaning
`openat2` with `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS`
or proven-equivalent semantics; bytes over the socket remain preferred.

Temporary and handoff state is removed on both success and failure, with
bounded recovery for what a crash leaves behind.

**Exactly one bounded, redacted audit record per publication attempt**, success
or failure alike, written through the controller's append-only path. A refusal
before the push is still an attempt.

**Merging stays human.** No merge, no auto-merge, no merge queue in v1.


**The PR body is rendered by the orchestrator from the seal**, not by the
publisher, because delivery state is readable only by its owning uid. It
carries the panel result as `10/10 unanimous` taken from the attested record
set, the integration commit, `snapshot_sha256`, `candidate_id`, `content_id`,
the round count, a per-seat table of role, verdict and receipt locator, a
validation evidence summary by reference or digest, the route input summary,
the verification matrix summary, the simplification outcome, unresolved risks,
and an explicit statement that merge requires human action. It carries no
transcripts, credentials, raw identifiers or authenticated URLs, and it is
bounded in bytes. PR creation is impossible while any finding stands, a seat is
missing, the snapshot is stale, the binding is underived, or the publication
approval is absent or bound to different bytes.

**D10. Three identities in v1, and the boundary is stated honestly.** The
delivery state root is `0700`, uid-owned, checked by uid equality rather than
`access(2)`, and refused inside any enclosing Git working tree. That makes the
earlier five-principal matrix unimplementable, but it does not license a single
uid either, because Gas City sessions share one identity and would otherwise be
able to forge the very records that gate publication.

- **`gascity`** owns Gas City, the delivery state root at a path outside every
  checkout, and runs the supervisor and all agent sessions.
- **`controller`** runs D7's panel-and-approval controller: distinct uid,
  peer-authenticated socket, append-only approval and audit records, and the
  signing or binding key for the publication manifest. No agent can read that
  key.
- **`publisher`** holds the repo-scoped PAT and runs only the publish step.

Three is the minimum that closes the forgery path. Collapsing `controller` into
`gascity` lets any agent write approvals; collapsing `publisher` into
`controller` puts the PAT beside the approval key.

**Agents share the `gascity` uid, and that is a prototype posture, not a
production one.** The supported runtime is `tmux` in host worktrees, upstream
states that configured commands are "a feature, not a sandbox", and no
per-session uid option exists. So there is no `agent-worker` identity today.
The honest blast radius is: an agent has everything `gascity` has, including
the delivery state root and the model credential. What it does **not** have is
the publishing PAT, the controller's approval key, or the ability to originate
an approval or a dispatch record, because all three live behind D7's boundary.

D18 records the remaining consequence: while agents share the `gascity` uid and
the supervisor mutation endpoint is reachable from it, the configuration is
acceptable for prototype exploration and **not** for the production unattended
workflow. P8 is the gate that resolves it.

Publication is an **orchestrator-produced, publisher-consumed handoff**: the
orchestrator renders the body from the seal, the controller binds it into the
manifest, and the publisher receives the manifest, the full commit sha and the
approval digest through the transfer of D9. The publisher never reads the
delivery root.

**Approval integrity is the controller's append-only record**, not Gas City
bead state and not a custom root-owned store re-specified here. The properties
that must hold: the approval is bound to the exact bytes, a mismatch denies
rather than warns, the record is write-once and owned outside the `gascity`
uid, and the publisher re-verifies before acting.

**D11. Egress is filtered by owner uid or cgroup, and loopback is allowed.**
The control plane is loopback HTTP on `127.0.0.1:8372` plus a Dolt **TCP**
port, and agent steps are instructed to run `gc bd update`, `gc bd close` and
similar. A network namespace without host loopback would stop every step from
recording its result while an egress test still passed, so the earlier
namespace design is withdrawn.

v1 uses host nftables rules keyed on owner uid or cgroup that **allow loopback
and the local control plane**, and deny d2b bridges and guest addresses,
RFC1918 and other LAN ranges except explicitly required exceptions, and
link-local including metadata addresses. Required egress is allowed by
explicit allowlist only: the configured model endpoints, Nix substituters and
package registries, and DNS to the configured resolver. The exact mechanism and
allowlist resolution belong to the follow-on spec after **P3**.

**D12. The module integrates with Gas City's real supervisor, and requires a
control dispatcher.** `gc` installs, enables and restarts
`gascity-supervisor.service`, or `gascity-supervisor-<base>-<hash>.service`
under a non-default `GC_HOME`, as a systemd **user** unit, and restarts it
itself on binary drift. There is no `gascity-orchestrator` unit upstream, and a
system-scope unit running `gc supervisor run` would race the user unit `gc`
restarts.

`gascity.nix` therefore owns Gas City's **own** unit at its real name and
scope, pinning `GC_HOME` and the supervisor systemd scope and either delegating
auto-restart to `gc` or disabling it and owning restart itself. That choice is
**P-gated**, not decided here. Dashboard, egress and any notification units are
ordinary neighbours rather than links in a fabricated chain, and the earlier
claim that shutdown order comes free from `After=` is withdrawn, because that
holds only for units systemd itself stops and a user-scope supervisor is not in
that graph.

**One store-scoped `control-dispatcher` per graph-owning scope is mandatory**;
v1.4.0's upgrade notes require it or a rig-owned graph fails before
instantiation. The module configures it.

Lifecycle **properties** are retained and their exact unit matrix is not, but
two of them need mechanism the module must actually wire rather than inherit:

- **Agent sessions get a real grace period.** Do not assume systemd's defaults
  supply it. A user-scope supervisor stopped by the system manager can take its
  whole cgroup with it, killing tmux panes immediately, so the module must
  configure the supervisor unit's stop behaviour so the supervisor gets time to
  **drain** sessions and systemd escalates only after a bounded interval.
  Adoption and reconciliation are a **start** concern, not a stop one, and
  conflating them in the stop interval hides which phase a failure came from.
  Whether `KillMode=mixed` is the right expression of that is **prototype
  gated**, not asserted here; the property is that no immediate cgroup-wide
  kill of agent panes occurs before the drain bound expires.
- **The confinement outlives every process it confines**, across scopes. Once
  P3 and P8 select the confinement mechanism, `gascity.nix` must wire or
  supervise that ordering explicitly. Calling the egress and confinement units
  "ordinary neighbours" is not enough: neighbours have no ordering relationship,
  and a confinement torn down while an agent still runs is an unconfined agent
  at the least supervised moment.

The remaining properties are unchanged: ingress stops accepting before work
drains, durable state is consistent before owners exit, and startup adopts
before it cleans.

**D13. Discord is notification, status and Q&A in v1; approvals are local.**
`discord-interactions` requires inbound public HTTPS, which a local host does
not have without a tunnel, and the gateway delivers events **into agent
sessions**, which are exactly the principal that may not originate an approval.
So in v1 Discord carries notifications, status and clarification Q&A, and the
artifact-bound approval decision is taken by the operator locally through the
controller's CLI.

**The CLI encodes the closed decision set** `{approve, revise, rescope, abort}`,
and every one of them is a decision the controller records against the exact
artifact identity. The gate bead is **transport**: the controller closes it
after recording **any** valid decision, so no decision strands a run on a bead
that nothing will ever close. A mandatory **decision-router** step then reads
the protected record, not the bead, and routes:

| Decision | Router action |
| --- | --- |
| `approve` | Continue to the next stage |
| `revise` | Invalidate the current artifact approval and route back to the producing stage or its fix loop |
| `rescope` | Park or terminate the current run and attach or create the successor route, linked to the source run, requiring its own approvals |
| `abort` | Cancel and close the remaining workflow and stop |

**Rescope is idempotent by construction.** The successor's identity is derived
deterministically from the source run id together with the protected decision
record's id or digest, so the same decision always names the same successor.
The router looks for an existing linked successor before creating one; a retry,
including one after a crash between creation and completion, **attaches to and
returns the existing successor** rather than creating a second. A rescope that
could produce two successors would fork the work and leave one of them
unapproved.

Two consequences worth stating. **A naked `gc bd close` is not authority**: the
controller performs the close after recording, and the router consults the
record, so a bead closed by anything else advances nothing, and a bead whose
state disagrees with the record is a diagnosed mismatch in which **the record
governs**. And the router reports actionable status, naming the stage it routed
to and the command that resumes or inspects the run, rather than leaving the
operator to infer what a closed bead meant.

Ingress remains restricted to the one configured operator identity and channel.
A rejection emits a bounded local diagnostic naming **only its closed rejection
class**. Reviewer, run and Discord digests appear in the protected audit record
and **never in ordinary logs, diagnostics or error text**, because a digest
that is printed on every rejection accumulates in exactly the places that are
least protected and most often shared.

**P5** may prove a non-agent approval consumer without inbound public HTTPS; if
it passes, a Discord approval adapter may be enabled later under the same
artifact-binding rules. The connection requirement is honestly satisfied in v1
by notifications and Q&A.

**D14. Check scripts are module- or pack-owned and unwritable by agents.**
Formula `check` scripts are executed by the orchestrator's control dispatcher,
and upstream formulas point at repo-relative paths such as
`.gc/scripts/checks/build-artifact-valid.sh`. If that resolves inside an
agent-writable rig tree, an agent that edits the script executes code as the
orchestrator, which defeats every other control in one step.

**P0 resolves the base directory and is the highest-priority prototype.**
Regardless of its outcome, the requirement stands: every script referenced by a
`check` step in a privileged or orchestrator-run context is owned by the module
or the pack, lives outside every agent-writable tree, and is not writable by
the `gascity` session identity. Acceptance plants a hostile edit and observes
refusal.

**D15. Task import creates beads with explicit edges, or chunks by phase.**
`drain` caps `max_units` at **100** and reports `limit_exceeded` beyond, and in
`separate` mode creates item roots in parallel; dependency edges come only from
`needs` and `depends_on` resolved at compile time. A drain therefore does not
represent an arbitrary imported DAG.

The importer either creates beads directly with blocking dependency edges
preserving arbitrary imported dependencies, or uses one drain per `## Phase N`
with at most 100 members chained by `needs`. It parses what is reliable in
`tasks.md`, treats prose dependencies and paths as advisory pending the human
task-DAG gate, fails an unparseable line while naming the expectation that was
not met, and is idempotent by task id. **P6** exercises a 120-task import and a
dependency-heavy control.

**D16. Repository hygiene replaces the `.gc/` prohibition.** Upstream uses
`.gc/` legitimately: linked worktrees under `<repo>/.gc/worktrees/<id>`, check
scripts under `.gc/scripts/checks/`, and `mol-pr-ship` writing
`.gc/pr-pipeline/ship/<branch>.md`. The blanket ban is withdrawn.

Instead: `.gc/` is gitignored in d2b; worktrees are relocated outside the d2b
checkout via `GC_WORKTREES_DIR` where supported; the follow-on spec names the
pack-written `.gc/` paths that remain; and no runtime evidence, state,
transcript or attestation payload is ever committed, which is the property
section 12.5 actually requires.

**D17. Approvals are artifact-bound and fail closed.** The record carries the
gate node, the artifact identity, a decision from
`{approve, revise, rescope, abort}`, a reviewer digest, a run digest and a
timestamp. For publication the artifact identity is the immutable integration
commit. An approval is honoured only when the recorded identity equals what is
in front of the consumer; on mismatch the run is denied, not warned, and the
error names the remediation. Identifiers are stored as deployment-keyed digests
rather than plaintext. Human approval is required at the constitution, spec,
plan, task-DAG and publication gates.

**Round input retention is bounded and something enforces it.** Exact
review-input bytes are retained 30 days or 2 GiB, whichever binds first;
content addresses, evidence references and per-seat attestations are retained
for the audit period as a floor. After the window a round can still be proven
to have reviewed a specific artifact and may no longer reproduce the bytes from
Gas City alone, which is the right trade when the bytes are recoverable from
Git.

Enforcement is named without freezing its layout: either a systemd timer in
generic `gascity.nix` operating on configured state directories, or a
pack-owned cleanup command the config repo schedules. A bound with no enforcer
is a comment, and the previous revision's mistake was specifying the on-disk
hierarchy instead of saying who runs the clock.

**Redaction is scanned, not asserted.** No durable record, log line, error
message or operator output contains any of: a credential or token; a raw
Discord id, user id or run handle; an opaque session or run secret; a URL
carrying authentication; a store or host filesystem path; an argument vector,
environment block or working directory; a socket path, unit name or PID; raw
terminal bytes; a shell or session name; raw command output; a span attribute
or metric label carrying any of the above; or a `Debug` rendering that exposes
them. Audit records may carry approved fixed digests and closed enum values
only, and protected observable surfaces carry explicit redacting `Debug`
implementations rather than derived ones.

**Metric labels are drawn only from closed, low-cardinality enumerations**:
component, operation, producer or provider class where that class is itself
bounded, outcome, and typed error code. An arbitrary string is refused as a
label value even when it carries nothing sensitive, because unbounded
cardinality is its own failure: a label whose domain is a run id or a branch
name degrades the metric backend regardless of what the string says.

**No surface is a catch-all for what the label could not hold.** An audit
record carries **only** fixed digests, closed enum values, and bounded numeric
and timestamp fields. There is no free-form or schema-defined text field, not
even a reviewed one, because a text field that exists will eventually carry
whatever the caller had. Ordinary local log lines are bounded and redacted
under the same rules and are likewise not a catch-all. An earlier revision said
detail belonged in a log line or an audit record instead of a label; that made
the audit trail the dumping ground for exactly the values the label rule exists
to exclude.

**Correlation is an alias, not the value.** Raw run ids and branch names appear
in neither logs nor audits. The controller issues a bounded **correlation
alias** with a fixed grammar and length, which ordinary local logs and error
text may carry; the audit record carries a digest. An operator resolves it with
`d2b-gc correlate <alias>`, which reads protected controller state and shows
the authorized run and branch mapping locally. Every log line and error that
references a run names the alias **and** that command, so the alias is
actionable rather than merely opaque.

The scan reports the number of records, log lines and error strings examined,
fails closed on an empty corpus, and ships one planted control per category
above that it must reject.

**A generic root-owned append sink carries the audit.** The controller's
approval record is the **authority**; the audit is durable evidence, and the
publisher verifies the protected approval rather than merely the audit trail.
Both need somewhere append-only that the `gascity` uid cannot rewrite.

That sink is generic: either a minimal root-owned append service supplied by
`gascity.nix` with **no d2b schema knowledge**, taking bounded typed records
from configured submitters, or a root-owned system service instantiated by the
d2b configs. Its required properties are root ownership, append-only and
write-once semantics, daily rotation, a bounded default retention, synchronous
flush before acknowledgement, and **no update or delete API**. Its directory
hierarchy and sync algorithm are spec-level and deliberately not frozen here;
the previous revision's mistake was specifying those instead of the properties.

**D18. The mutation surface to close is the supervisor's, not the dashboard
proxy's.** `DASHBOARD_READONLY` is scoped to the Node BFF's transport proxy;
`/v0/city/{city}/session/{id}/respond` is the **supervisor's** endpoint on
`127.0.0.1:8372`, which also serves its own SPA. Setting the dashboard flag
therefore does not stop a local process from answering its own pending
interaction directly.

**v1 must mechanically prevent agent sessions from reaching that endpoint.**
The process-trust option an earlier revision offered is withdrawn: "we rely on
process trust within the `gascity` identity" is not a control when the
principals it must exclude run under that identity.

A distinct worker uid is **not** sufficient on its own here, and an earlier
revision was wrong to list it as one. The endpoint is **TCP**, and a TCP socket
carries no `SO_PEERCRED`, so the server cannot learn the caller's uid from the
connection at all. The implementation gate must select and prove one of:

- **Move the mutation API to `AF_UNIX`** and authenticate the peer atomically
  with `SO_PEERCRED` or `SO_PEERPIDFD`; or
- place it behind an **authenticated proxy or token boundary** whose secret is
  not readable by agent sessions; or
- apply **socket-level filtering in the kernel**, such as a cgroup-attached BPF
  hook, that distinguishes agent sessions before the connection is served.

**User-space source-port to PID lookup is explicitly forbidden.** Resolving a
TCP connection back to a process by scanning `/proc` is racy by construction:
the port can be reused and the process can exit between the lookup and the
decision, so it authenticates nothing. A proxy that does this is not a
boundary.

Same-uid `tmux` satisfies none of the three, so **P8 and P3 must prove the
selected mechanism before ship**. Until one is proven, a one-uid `tmux`
deployment is acceptable for **prototype exploration only** and is not the
production unattended workflow while the mutation endpoint stays reachable.
Whichever mechanism is chosen must keep the `gc bd` control traffic reachable
through that same authenticated channel rather than by reopening the mutation
surface.

Acceptance probes `:8372` directly rather than the dashboard proxy.

**D19. Versions are a three-way pin, with a package override decision.** The
config repo revision, the `gc` package, and `packs.lock` commits must agree.
Because `llm-agents.nix` builds v1.4.0 while the design's evidence is largely
from `main`, and because the package sets `main.commit=nixpkgs` so `gc version`
cannot report an upstream commit, **P7 decides** between re-measuring at
v1.4.0 and adding a commit-pinned `packageOverride` in `gascity.nix` sourced
through `llm-agents.nix` with `-X main.commit=<sha>` and a smoke check.
`llm-agents.nix` remains the package source either way.

The generic module **may** add `python3` to the unit PATH, because the
`discord` and `github` pack scripts are python3 and the wrapper omits it. The
earlier blanket rule against restating any wrapper dependency is amended to
that single documented exception; the other eight packages stay the wrapper's.

**D20. This record decides properties and ownership, not mechanisms.** The
following are deliberately **not** frozen here and belong to the follow-on spec
after their prototypes: the panel dispatch-record wire shape; any audit store
layout, retention algorithm or sharding; unit names beyond Gas City's real
ones; the egress rule syntax and allowlist resolution; the publisher's exact
invocation; the Discord approval adapter, if P5 permits one. An ADR that
freezes the hard-to-reverse details while deferring the easy ones has the
trade backwards, which is what the previous revision did.

**One filesystem property is retained, because deferring it would defer a
vulnerability rather than a detail.** Any filesystem path a helper, controller,
publisher or audit sink owns and resolves is resolved **anchored and
fd-relative** from a pinned descriptor, refusing symlinks and magic links, so a
component swapped between validation and use cannot redirect a privileged write
or read. `openat2` with `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS`
satisfies it, as does an equivalent with **proven** equal semantics, such as a
component walk with `O_DIRECTORY | O_NOFOLLOW` and `fstat` verification of each
opened component. All three resolve flags are required together; dropping
`RESOLVE_NO_SYMLINKS` leaves a symlinked leaf traversable, which is the case a
planted control catches. This repository already uses that shape
in `packages/xtask/src/delivery/storage.rs`, so it is established practice
rather than a novel demand.

## Prototype gates

These are binding pre-specification gates. **No implementation of the affected
area begins before its prototype passes**, and a failure routes as stated
rather than being worked around. Accepting this ADR accepts the architecture
and the prototype program; it does not assert that any mechanism below already
works.

| ID | Experiment | Pass | Fail routes to |
| --- | --- | --- | --- |
| **P0** | Resolve `check.path`'s base directory: author a formula with a relative check path, cook and run it, observe which file executes and as which uid, then attempt to overwrite it from a task worktree. | The script resolves to a pack or module-owned path and is not writable by the `gascity` session identity. | D14 becomes a hard blocker: no privileged check step may reference a rig-writable path, and implementation stops until the module owns them. |
| **P1** | Import `gc`, `superpowers` and a two-formula `d2b-engineering` as **siblings** in a scratch city; run `gc doctor`, `gc formula show`, and cook a three-step `d2b-build` on a throwaway rig. Repeat with the packs **nested** under a composite pack. | Sibling variant resolves all agents and dispatches `gc.run_target` steps with no collision; nested variant **fails**. | D5 is wrong and the composition model must be redesigned before any pack work. |
| **P2** | `d2b-panel` as a `check` loop whose exec script shells `xtask ... panel-attest --records-stdin --dispatch-record <fd>` against a fixture wave; and the controller-emitted dispatch record the verifier re-derives binding from, with the seats running as Gas City sessions. | Round one non-unanimous spawns attempt two from the engine; round two unanimous closes the step and writes a seal; `max_attempts` is enforced by the dispatcher; binding is re-derived from the **controller's** dispatch record, and a session-written claim cannot change it. | The panel becomes an external driver invoked from one step. If the controller cannot own dispatch for session-run seats, the Gas City panel **does not ship** and the standalone producer remains the only supported one. No fail-open. |
| **P3** | From inside the proposed confinement, run `gc bd update`, `gc bd close`, a check script and a Discord post; then attempt the supervisor mutation endpoint and the controller socket from an agent session. | Control-plane calls succeed with loopback and the Dolt port reachable, **and** the mutation endpoint is refused while the controller socket rejects the agent peer. | D11's rules are wrong; egress filtering is re-derived until the control plane is reachable without reopening the mutation surface. |
| **P4** | Publish from a unit with an explicit `WorkingDirectory` on the integration store: `git -C <repo> push origin <sha>:refs/heads/<branch>`, then create-or-edit the PR, retried twice; body transferred over the socket or an open fd rather than by path. | The branch appears at exactly that sha, retry converges without duplicating, the body cannot be substituted by swapping a path, and an unapproved sha is refused. | **Publishing design is blocked and returns for redesign.** There is no App fallback: adopting the shared-key `github` pack path while claiming credential isolation is forbidden. |
| **P5** | Approve a gate end to end from Discord **without** inbound public HTTPS and without any agent session participating. | The gate bead closes and no agent session was involved. | D13 stands as written: Discord stays notification-only and approvals remain local. |
| **P6** | Import a 120-task `tasks.md` and a dependency-heavy one. | Dependencies are enforced, over-100 is handled by phase chunking or direct bead creation, and re-import is idempotent by task id. | The importer emits beads directly with explicit edges and the drain path is abandoned. |
| **P7** | Rerun P1 and P2 against the **v1.4.0** binary built from `llm-agents.nix`. | Behaviour matches the documented semantics the design relies on. | D19 takes the commit-pinned `packageOverride` path, or the evidence is re-measured at v1.4.0 and any divergent claim is corrected. |
| **P8** | **Session and control separation.** Prove a session provider or isolation mechanism under which an agent cannot reach the supervisor mutation endpoint or forge controller input, and separately that delivery state created as identity A is refused to identity B while the manifest handoff still publishes. | Agents are mechanically excluded from the mutation surface and from the controller's authority, and the handoff carries manifest, sha and approval digest sufficient to publish. | The production unattended workflow **does not ship**; the deployment stays prototype-only under D18 until a mechanism is proven. |

## Non-goals, and what this ADR does not authorize

- **No d2b product surface**: no flake output, `d2b.*` option, `nixos-modules/`
  content, manifest field, schema, wire message, broker op, CLI verb, Diataxis
  page, `README.md` mention or critical-subsystems row, and no framing of Gas
  City as a d2b capability anywhere including changelog prose.
- **No d2b execution substrate**: no runtime provider, sandbox, VM pool, guest
  artifact return, workspace transfer protocol, guest credential injection, or
  use of `d2bd`, the broker, guest-control or the Credential service.
- **No nested composite pack**, and no d2b formula name without the `d2b-`
  prefix.
- **No reliance on `compose.branch`, `compose.gate` or `compose.aspects`**, and
  no assumption that a gate `type` has a watcher.
- **No panel semantics weakened at any producer seam**, no round manifest added
  to xtask, and no change to `PanelRecord`, `PanelRequest`,
  `validate_record_set` or the pinned policy constants.
- **No producer-asserted binding accepted as proof**, and equally **no
  fail-open**: the Gas City panel path ships with derived binding or does not
  ship.
- **No authority in Gas City session state.** Approvals, dispatch records and
  publication manifests are never session-written; a bead close is not an
  approval; and no signing or approval key is readable by any agent.
- **No process-trust exemption** for the supervisor mutation surface, and no
  production unattended workflow while agents share a uid with a reachable
  mutation endpoint.
- **No caller-supplied body path** in the publication handoff, and no GitHub
  App fallback adopted while claiming credential isolation.
- **No unconditional force push**, and no lease-less update of an existing PR
  branch.
- **No user-space TCP source-port to PID lookup** as an authentication
  mechanism anywhere in this design.
- **No identity digests in ordinary logs, diagnostics or error text**; they
  belong only in protected audit records.
- **No unbounded descriptor passing**, and no claim that setting close-on-exec
  after creation is atomic.
- **No claim of an agent-worker uid** while the runtime is `tmux`, and no
  acceptance item written against a process that does not exist.
- **No confinement that blocks host loopback or the Dolt port.**
- **No fabricated systemd units for Gas City's orchestrator**, and no claim
  that shutdown ordering is free for a user-scope unit the system manager does
  not stop.
- **No Discord approval path in v1** without P5, and no treatment of an agent
  session as an approval origin.
- **No merge, auto-merge or merge queue.**
- **No blanket `.gc/` prohibition**, and no committed runtime evidence, state,
  transcript or attestation payload.
- **No frozen mechanism** for the items D20 lists.

## Acceptance

Accepting this ADR is not a claim that the mechanisms work; the prototype gates
above own that. The items below are the conditions the **first implementation**
must meet, and each is written so it cannot pass vacuously.

Every absence scan is subject to two standing requirements: it reports the
number of files, inputs or sites it examined and fails closed on an empty set;
and it ships planted violations it must reject.

- **M1 No product surface.** Mechanical: no Gas City reference in `flake.nix`,
  `nixos-modules/`, `packages/` outside the delivery gate,
  `docs/{reference,how-to,explanation}/`, `README.md`, or
  `critical-subsystems.md`, and no `d2b.*` option, manifest field or schema
  property mentioning it.

  **Counted coverage per surface, and one planted control per surface**, not a
  sample of four across all of them: a `flake.nix` output attribute; a Nix
  option or module in `nixos-modules/`; Rust product code in `packages/`
  outside the delivery gate; a `docs/reference/` page; a `docs/how-to/` page; a
  `docs/explanation/` page; `README.md`; a `critical-subsystems.md` row; and a
  manifest, bundle or schema property name and its description, counted
  separately where those are separate categories. Each surface reports its own
  examined count and fails closed at zero, so a relocated or renamed surface
  cannot pass by contributing nothing. A sampled control set leaves the
  unsampled surfaces unproven, which is the failure mode this scan exists to
  prevent.
  Reviewer judgement, stated as such: changelog mentions must describe
  contributor process and not a d2b capability, which no scanner can decide.
- **M2 d2b is never contacted, whether or not d2b is running.** Unchanged, and
  the strongest item in this record. With d2b running normally and at least one
  VM up: a configuration and formula scan finds no reference to `d2bd`, the
  `d2b` binary, `d2b-priv-broker`, `/run/d2b/*.sock`, guest-control or any d2b
  unit; an execution trace shows no `connect` under `/run/d2b/` and no `execve`
  of those binaries; and the workflow completes with `/run/d2b/` denied to the
  `gascity` identity and no d2b binary on its `PATH`. Counted coverage on each
  part, with planted controls for both the scan and the trace.
- **M3 The standalone surface is unaffected and self-sufficient.** With Gas
  City absent, `check-bindings.mjs`, the ADR index guard and `make check-tier0`
  pass; every agent and skill runs without the config repo, the `gc` binary or
  any Gas City service; and no file under `.github/{agents,skills}/` references
  Gas City, the config repo or the orchestration manifest. Not a byte freeze.
- **M4 Sibling imports resolve and nesting is rejected.** The city loads with
  the packs as siblings, every `gc.run_target` and `superpowers.*` target
  resolves, and a cooked `d2b-build` dispatches its first step. A planted
  nested-import configuration is **rejected**, and a planted duplicate agent
  name across two imports fails city loading. Also: every d2b formula name
  carries the `d2b-` prefix, the doctor check asserts the resolved winner for
  each, and a planted unprefixed `review` formula is caught.
- **M5 Composition lint.** A planted `compose.gate`, `compose.branch` and
  `compose.aspects` in the pack are each rejected by the doctor check, and a
  negative control confirms that a formula relying on one of them loses it at
  compile time, which is why the lint exists.
- **M6 Routes behave differently and converge.** Route A produces
  `specs/NNN-slug/` and stops at its human gates. Route B refuses a Proposed
  ADR, a missing one, and one whose bytes changed after admission, each with a
  typed error, and admits an Accepted one with its digest recorded and
  re-checked. Route C completes with **no** `spec.md` or `plan.md` anywhere,
  asserted by absence, and each of the five escalation conditions is planted in
  turn and observed to stop the run. Route D re-attaches by workflow root
  without duplicating beads and fails closed on an unresolvable root.
- **M7 The panel runs as a check loop and submits only unanimity.** A
  non-unanimous round one spawns a second attempt from the engine; a unanimous
  round two closes the step and writes a seal; `max_attempts` is enforced by
  the dispatcher. The gate receives exactly one final unanimous set, never an
  intermediate one, and round history is observable in beads rather than in
  xtask.
- **M8 Binding is derived, not asserted.** Admission re-derives `provider`,
  `model_version` and `reasoning_effort` from the dispatch record. Three
  **well-formed** envelopes are each rejected with their own typed semantic
  error, not a parse failure: one whose claimed binding contradicts the
  dispatch record; one whose dispatch record is absent; and one whose dispatch
  record is present but untrusted. A test that passes because the payload was
  malformed does not satisfy this item.
- **M9 Producer neutrality.** A Gas City round and a standalone round reach the
  same canonical gate. Each of these is submitted from **both** producers and
  rejected identically with the same typed reason, each envelope otherwise
  well-formed: wrong model, wrong effort, wrong provider, missing seat,
  duplicate seat, stale snapshot, and mismatched candidate or content id. The
  stored canonical records converge byte-identically or under a stated
  normalization for producer-specific fields.
- **M10 Content change invalidates a satisfied candidate.** The test drives a
  candidate to satisfied and **sealed** with a unanimous roster, mutates the
  reviewed content, and asserts the prior sign-off and seal **no longer satisfy
  eligibility**: the wave is not merge-eligible, the records do not carry over,
  and a fresh unanimous round against the new snapshot is required. Rejecting a
  late submission alone does not satisfy this item.
- **M11 Check scripts are unwritable by agents, and helper paths are anchored.**
  Every script referenced by a privileged or orchestrator-run `check` step
  resolves outside every agent-writable tree and is not writable by the
  `gascity` identity. A planted hostile edit from a task worktree is refused,
  and the edited script is observed never to execute.

  Every filesystem path the controller, publisher or audit sink owns is
  resolved with `openat2` under
  `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_SYMLINKS`, or proven
  equivalent semantics. Three planted controls are each refused: a symlinked
  leaf, a symlinked intermediate component, and a magic-link component. A
  resolver missing `RESOLVE_NO_SYMLINKS` passes the magic-link control and
  fails the symlinked-leaf control, which is why all three are required.
- **M12 Egress denies the right things and permits the control plane.** From
  the confined context: `gc bd update`, `gc bd close`, a check script and a
  supervisor call all succeed, and the Dolt port and `127.0.0.1:8372` are
  reachable. Simultaneously, planted d2b bridge and guest addresses, a LAN
  address, and a link-local address including `169.254.169.254` are each
  unreachable, while the configured model endpoint, substituter and resolver
  are reachable. Both halves are required.
- **M13 Publication requires approval and publishes an exact sha.** The
  publisher pushes `<full-sha>:refs/heads/<branch>` from an explicit working
  directory on the integration store and opens a PR with `--body-file`. It
  refuses, each with its own typed error and with **no branch pushed**: a
  finding present, a missing seat, a stale snapshot, an underived binding, and
  an absent or mismatched publication approval. No merge, auto-merge or merge
  queue is enabled under any configuration.
- **M14 The PR body carries the canonical panel result.** The rendered body
  contains `10/10 unanimous` from the attested set, the integration commit,
  `snapshot_sha256`, `candidate_id`, `content_id`, the round count, all ten
  seats with verdict and receipt locator, a validation summary by reference,
  the route input summary, the verification matrix summary, the simplification
  outcome, unresolved risks, and the merge-requires-human statement. It
  contains no transcript, credential, raw identifier or authenticated URL, and
  is within its size bound. A planted nine-of-ten body is rejected.
- **M15 The publisher never reads delivery state.** With the publisher running
  as its own identity, an attempt to read the delivery state root, the seal or
  any panel record is refused, and publication nonetheless succeeds from the
  controller-bound handoff of the exact sha, approval digest and bounded PR
  body over the authenticated transport of D9. No caller-supplied body path is
  accepted.
- **M16 The supervisor mutation surface is mechanically closed.** A process in
  an agent session attempting to answer a pending interaction directly against
  `127.0.0.1:8372` is refused by the mechanism selected through P3 and P8.
  Probing only the dashboard proxy, or documenting reliance on same-uid process
  trust, does not satisfy this item.
- **M17 Task import preserves dependencies at scale.** A 120-task `tasks.md`
  imports without `limit_exceeded`, a dependency-heavy control has every
  imported edge enforced at execution rather than run concurrently, re-import
  is idempotent by task id, and a malformed line fails with the unmet
  expectation named.
- **M18 Package provenance and PATH.** The running `gc` resolves to the store
  path the locked `llm-agents.nix` revision produces, no other `gc` is on the
  path of any identity that can invoke it, and the unit PATH contains `python3`
  while the module restates none of the other eight wrapper packages.

  Counted coverage over the resolved units, packages and pins, **failing closed
  when that corpus is empty** so a relocated or unresolved unit set cannot pass
  silently. Planted negative controls that must each be **refused**: a `packs.lock` whose recorded commit
  no longer matches the imported pack tree; a config revision pinning a
  different `llm-agents.nix` revision than the one that built the running
  binary; and a `gc` whose reported version or commit disagrees with the locked
  package. A three-way pin that cannot detect its own drift is not a pin.
- **M19 Repository hygiene.** `.gc/` is gitignored, worktrees resolve outside
  the d2b checkout where `GC_WORKTREES_DIR` is supported, and no runtime
  evidence, state, transcript or attestation payload is committed. Planted
  control: a committed `.gc/pr-pipeline/ship/*.md` is rejected.
- **M20 Lifecycle properties hold against the real supervisor.** The module
  manages Gas City's own supervisor unit at its real name and scope; a
  `control-dispatcher` is configured per graph-owning scope and its absence is
  observed to fail graph instantiation; ingress stops accepting before drain
  begins; agents get a bounded grace period; confinement outlives the processes
  it confines; and startup adopts before it cleans. Traces assert against the
  real unit, and each run plants a detectable control event so an empty trace
  cannot pass.

- **M21 An agent cannot forge authority.** Running as the `gascity` identity
  with a live session, each of these is attempted and refused: writing or
  amending a controller approval record; producing a dispatch record the
  verifier accepts; emitting or altering a publication manifest; reading the
  controller's signing or approval key; and connecting to the controller socket
  as a peer other than the expected orchestrator identity. Then the end-to-end
  case: a run that closes the publication gate bead directly with `gc bd close`
  and invokes the publisher is **refused**, because the controller has no
  `approve` recorded for that commit. Bead state alone advancing a gate fails
  this item.

  **Cross-operation matrix.** Every combination of the three endpoints and the
  three operation classes is exercised: nine cases, of which three are the
  authorised pairs (orchestrator with dispatch and status, operator with a
  closed decision, publisher with manifest consumption) and six are refused.
  The orchestrator endpoint must refuse an approval attempt because the
  operation is **absent from that endpoint**, not merely denied to that peer,
  and the test asserts the distinction.
- **M22 The supervisor mutation surface is closed to agents.** From an agent
  session, a direct call to `/v0/city/{city}/session/{id}/respond` on
  `127.0.0.1:8372` is **refused** by whichever mechanism P8 selected, while
  `gc bd update`, `gc bd close` and a check script all still succeed from the
  same context. Probing only the dashboard proxy, or recording that the
  deployment "relies on process trust", does not satisfy this item.
- **M23 Publication is idempotent, lease-safe and path-safe.** Four push cases,
  each asserted on the observed argument vector, and none of them a bare force:
  an **initial** push of a new branch with no lease; a **fast-forward retry**
  of the same publication, which converges; a **non-fast-forward update** after
  a rebase, carrying the manifest's `expected_previous_remote_sha` in
  `--force-with-lease`, which succeeds; and a **missing lease**, where the
  manifest carries no expected value, which is **refused outright** rather than
  forced.

  Two concurrency cases distinguish where the expected value came from: a
  remote commit landing **before** the manifest was produced is captured by it
  and the update succeeds; a remote commit landing **after** the manifest was
  produced makes the lease stale and the update is **refused**. A publisher
  that freshly reads remote state would pass the second case wrongly, so the
  test asserts the value used came from the manifest.

  PR create-or-edit is exercised across the retry: created once, body edited
  the second time, never duplicated.

  **Both refusals are actionable.** The stale-lease error names
  `d2b-gc publish status <run-id>` and, when run, that command shows the
  manifest's expected sha, the current remote sha and the intervening commits;
  the documented remedy is to reconcile through the integration route and rerun
  `d2b-gc publish <run-id>`. The missing-expected-state error names how to
  regenerate the manifest from the current approved candidate.

  Neither output contains a force instruction without a lease, asserted by
  scanning the rendered error text over a **counted, non-empty** corpus of
  rendered errors. A planted error string carrying an unleased force
  instruction is required and must be **rejected** by that scan; a scanner that
  has never rejected anything has not been shown to detect anything.

  **Crash between push and pull request recovers without forcing.** A crash is
  injected after the push succeeds and before the pull request is created; the
  retry reads the remote, finds it already at `<full-sha>`, treats the push as
  complete and proceeds to create or edit the PR. Separately, a remote sitting
  at a **third** sha, neither the target nor the manifest's expected value, is
  **refused** with the stale-lease remedy rather than adopted as a new expected
  value.

  The body arrives as bounded message bytes over the socket or stdin, so a
  planted attempt to swap a file at a caller-supplied path has nothing to
  swap. If the implementation passes a descriptor instead, exactly one is
  passed, the sender closes its own copy immediately after `sendmsg` returns
  **whether it succeeded or failed**, the receiver takes it with
  `MSG_CMSG_CLOEXEC` and closes it, and a planted attempt to send two or to
  send an unexpected descriptor is refused **with every received descriptor
  closed**, verified across all control messages in the set rather than only
  the first.

  The repeated-transfer loop runs a hundred iterations mixing four injected
  failure classes: `sendmsg` failures on the sender side, and on the receiver
  side an unexpected descriptor, a duplicate or multiple-descriptor message,
  and a **valid expected descriptor arriving with a malformed payload**. That
  last case is required specifically because it is the one an
  order-of-validation mistake leaks: the descriptor is legitimate, so a
  descriptor-set check passes it and the payload check returns early past it.
  After the loop the open-descriptor count of **both** sender and receiver is
  unchanged from its starting value. A success-only loop, or one that injects
  only sender failures, does not satisfy this item, because a refusing receiver
  that forgets to drain leaks exactly the descriptors an attacker chose to
  send.
  Temporary and handoff state is absent after both a successful and a failed
  run, and a crash mid-publication leaves only state a bounded recovery
  removes.
- **M24 Exactly one publication audit record per attempt id.** Four attempts,
  each with its own attempt id: a successful publication, one refused for a
  missing approval, one refused for a stale snapshot and one refused for an
  underived binding. The assertion is **per attempt id**, not an aggregate
  count: each id maps to exactly one record. Planted controls supply an attempt
  id with **zero** records and one with **two**, and both are rejected, because
  an aggregate of four would pass a zero-and-two distribution while proving
  nothing. Each record is bounded and redacted, written through the append-only
  path, and none is writable by the `gascity` identity.
- **M25 Retention is enforced by something that runs, on both bounds.** The
  named enforcer, a `gascity.nix` timer or a pack-owned cleanup command, is
  observed to run on schedule. Two planted corpora are required, not one: round
  inputs **over the 30-day age bound**, and round inputs **over the 2 GiB size
  bound** while still inside the age window. Both are observed deleted, and in
  both cases the content addresses, evidence references and per-seat
  attestations survive through the audit floor. A deployment with bounds
  configured and no enforcer scheduled fails this item, and so does one that
  trims only by age.
- **M26 Redaction is scanned across every category, with coverage and
  controls.** No durable record, log line, error message or operator output
  contains any of: a credential or token; a raw Discord id, user id or run
  handle; an opaque session or run secret; a URL carrying authentication; a
  store or host filesystem path; an argument vector, environment block or
  working directory; a socket path, unit name or PID; raw terminal bytes; a
  shell or session name; raw command output; a span attribute or metric label
  carrying any of the above; or a `Debug` rendering that exposes them. Audit
  records carry only approved fixed digests and closed enum values, and
  protected observable surfaces carry explicit redacting `Debug`
  implementations.

  The scan reports the number of records, log lines and error strings examined
  and **fails closed on an empty corpus**. One planted control per category
  above is required, and each must be rejected; a scan that ships fewer
  controls than categories has not been shown to detect the categories it does
  not test. Separately, an ordinary rejection diagnostic is asserted to carry
  its closed class and **no digest of any kind**.

  **Metric label cardinality is checked as well as content.** Every emitted
  label value is drawn from a closed enumeration, and a planted metric carrying
  an unbounded label value, a run id, is **rejected** even though that value
  discloses nothing sensitive. A scan that only looks for secrets passes this
  control and misses the cardinality failure entirely.

  **Audit records carry no text field at all.** The scan asserts every audit
  field is a fixed digest, a closed enum value, or a bounded numeric or
  timestamp, and a planted record carrying any free-form string is rejected.

  **The `Debug` protection is proven by three separate controls, because any
  one of them alone is vacuous.** A hostile-banner scan on its own proves
  nothing if the parser already discarded the bytes: the rendering is clean
  because there was nothing to leak, not because `Debug` redacted anything.

  1. **Parser control.** A hostile or unparseable version banner, carrying a
     secret-shaped token and control bytes, is rejected or normalized, and the
     resulting value is asserted to retain **none** of the hostile bytes.
  2. **Policy control.** The panel receipt error enum is asserted, through the
     repository's existing policy-test mechanism over the source or API
     surface, to carry an **explicit** `Debug` implementation and **not** a
     derived one. This is the control that survives a future field being added
     to a variant nobody re-scanned.
  3. **Rendering control.** The `Debug` and `Display` renderings of every
     variant are scanned and must contain no protected field, no invocation or
     argv, and no filesystem path.

  All three are required; passing only the third is the vacuity this split
  exists to prevent.

  **Correlation aliases resolve, and raw ids never appear.** Log lines and
  error text carry the bounded alias and the `d2b-gc correlate <alias>`
  command, never a raw run id or branch name; a planted log line carrying a raw
  run id is rejected. Running that command against a valid alias is observed to
  return the authorized run and branch mapping from protected controller state,
  and running it against an alias with no mapping fails closed rather than
  guessing.
- **M27 The supervisor gets its grace period, with a real session running.**
  A live agent session with an active pane is started **before** the stop, so
  the item cannot pass against an empty supervisor. On stop, that pane is
  observed **not** killed immediately with the supervisor's cgroup; the
  supervisor receives the configured interval to **drain**; and systemd
  escalates only after the bound expires. Adoption is tested as a separate
  phase on the following **start**, where the supervisor is observed to adopt
  and reconcile the surviving sessions. Signal delivery and timing are observed
  rather than inferred from unit file text, and a run in which no session was
  active is a failed run.
- **M28 Confinement outlives every agent process.** With an agent deliberately
  ignoring its stop request for the whole grace period, the confinement
  selected by P3 and P8 is observed still in force throughout and torn down
  only after that process exits, ordered against the process exit rather than a
  timer. A teardown observed before the last agent exit fails this item.
- **M29 Both evidence variants converge on one typed verifier.** Admission
  accepts trusted binding evidence as bounded bytes on stdin or over the
  socket, with no host path hardcoded, and tests supply fixtures for both
  variants: `GasCityControllerDispatch` and `StandaloneHarnessReceipt`. A
  standalone round is driven to admission **with no controller present**,
  proving the skill remains functional alone, and its receipt locator is
  observed to have been captured **automatically by the adapter** rather than
  entered by a person.

  **The cutover gate fires before dispatch.** With no supported harness
  resolver present, the standalone panel is observed to **refuse before any
  seat is dispatched**, not at admission afterwards, and zero seat sessions are
  created. Its typed error is asserted to name the required
  `StandaloneHarnessReceipt` mechanism, the `make panel-preflight` command, the
  current and supported harness version or the missing resolver, and the
  bounded alias. `make panel-preflight` is observed to fail for the same reason
  when run directly, so a contributor meets the error before starting a round
  rather than during one.

  For the standalone variant, resolution runs through an **injected**
  `HarnessReceiptResolver`, so every case below is driven by a mock rather than
  by a real broken harness. A locator that **resolves** is accepted and its
  resolved binding is what the verifier uses. Each failure is asserted to be
  its own **distinct** variant on a well-formed envelope, not a parse failure
  and not a shared invalid-state error:

  - no resolver configured returns `HarnessResolverMissing`;
  - an unsupported harness returns
    `HarnessVersionUnsupported { current, supported }` carrying both values;
  - a locator that does not resolve returns `HarnessReceiptUnresolvable`;
  - a resolved binding contradicting the record returns
    `HarnessReceiptBindingMismatch`;
  - a submission carrying model and effort strings instead of a locator returns
    `SelfAssertedBindingRejected`.

  **Each variant's remedy sequence is asserted exactly**, for all five
  including `SelfAssertedBindingRejected`: `make panel-preflight` and
  `make panel-resume PANEL_REF=<alias>` in every case; the pinned Copilot CLI
  upgrade instruction additionally for `HarnessResolverMissing` and
  `HarnessVersionUnsupported`; and `make panel-migrate` as the first step for
  `SelfAssertedBindingRejected`. The alias in each message is asserted to match
  the bounded grammar, and `make panel-resume PANEL_REF=<alias>` is run against
  it and observed to replay the stored invocation **without printing it**.

  **No rendered message contains an invocation, argv, or a path**, asserted by
  scanning every one of the five rendered messages. A test that accepts any
  refusal, or any remedy string, for any of these five cases does not satisfy
  this item.

  `make panel-migrate` is separately exercised: it brings the skill and adapter
  to the pinned revision on a clean tree, and it **refuses** on a dirty tree
  and on a conflicting update, in each case naming which condition it hit and
  leaving the tree untouched.

  The same shape is exercised for the controller variant: evidence absent,
  evidence untrusted, evidence contradicting the record, and a record carrying
  only its own producer strings, all refused.

- **M30 Every decision routes, and no bead is stranded.** All four decisions
  are exercised end to end against a real gate. In each case the controller
  records the decision **and** closes the gate bead, and the decision-router
  step reads the protected record rather than the bead: `approve` continues to
  the next stage; `revise` invalidates the current artifact approval and
  returns to the producing stage or its fix loop; `rescope` parks or terminates
  the run and starts a successor linked to it, requiring its own approvals;
  `abort` cancels and closes the remaining workflow. After each, no gate bead
  is left open with nothing to close it, and the router's reported status names
  the stage it routed to and the command to resume or inspect.

  **Rescope is idempotent.** The successor's identity is derived from the
  source run id and the protected decision record's id or digest, so a repeated
  rescope attaches to the existing successor. A crash injected **after the
  successor is created and before the route completes** is followed by a retry,
  which returns the same successor rather than creating a second.

  **The record governs, not the bead.** Two divergence controls: a gate bead
  closed directly with **no** protected decision recorded causes the router to
  **refuse or park** rather than advance; and a bead whose state disagrees with
  the record is resolved in favour of the **record**, with the mismatch
  diagnosed. Together these prove the router does not read the bead as
  authority, which an advance-on-close implementation would fail.

- **M31 The audit sink is root-owned and append-only.** Submitting a bounded
  typed record from the controller and from the publisher succeeds; submitting
  from the `gascity` identity is refused. Update, delete and truncate have no
  API to call, and attempts to rewrite or truncate the store from every
  non-sink identity fail. An acknowledged submission survives an immediate
  power-loss simulation, demonstrating the synchronous flush.

  **Retention is proven on planted corpora, not asserted.** Audit records are
  planted beyond the age bound, beyond the size bound, and beyond the default
  retention, and rotation is observed to remove **only** eligible sealed
  records while preserving the content-address and attestation floor. The scan
  counts the records it examined and fails closed on an empty corpus. A
  deployment whose enforcer is missing, or whose enforcer runs and removes
  nothing eligible, fails this item.

  Separately, the publisher is observed to verify the **controller's protected
  approval** and to refuse a publication whose approval exists only as an audit
  line.

## Consequences

**d2b gains nothing, which is the point, and Gas City gains no guarantees
either.** A consumer's experience is unchanged. The cost is that Gas City
carries no compatibility promise, no deprecation policy and no support
obligation, and a contributor who builds a habit on it depends on something the
project has not promised to keep.

**Extending upstream instead of reimplementing it trades control for
maintenance.** `build-base` and friends give us stages, artifacts, postures and
a review-fix loop we do not have to write, and every upstream fix arrives free.
In exchange our methodology formulas are hostages to their extension seams: a
renamed var or a restructured stage in a future release breaks `d2b-build`, and
because import `version` is parsed and not enforced, nothing warns us. The pin
is a commit and upgrading is a real task with real breakage.

**The security boundary in v1 is three identities and one narrow socket, and
the record says what each buys.** Agents share the `gascity` identity, so
nothing inside a session is trustworthy. What is mechanical is that the
controller owns dispatch, approvals and the publication manifest under its own
uid behind a peer-authenticated socket, and that the publishing PAT lives with
the publisher. An earlier revision claimed five exclusions against a process
that did not exist; the expert simplification then removed the boundary
entirely and let an agent forge everything the gate consumes. Three identities
is the smallest arrangement that is both implementable and not forgeable.

**And it is still not a production posture until P8.** While agents share a uid
with a reachable supervisor mutation endpoint, an agent can answer its own
pending interactions. D18 records that as prototype-only rather than papering
over it, which means the honest status of the unattended workflow today is
blocked on a prototype, not shipped with a caveat.

**Discord is less capable in v1 than the design wanted.** Notification, status
and Q&A only, with approvals taken locally. That costs the operator the
convenience of approving from a phone and removes an inbound public HTTPS
surface and an agent-session-as-approval-origin hazard. If P5 passes, the
capability returns under the same binding rules.

**Nine prototypes is a real schedule cost, and one of them may be fatal.** P0
in particular could establish that check scripts resolve inside an
agent-writable tree, which would make the current formula idiom a privilege
escalation and force the module to own every check path before anything ships.
Finding that before implementation is the point; finding it after would be much
worse.

**The evidence gap is now explicit and still open.** Most behavioural claims
here come from `main`, the deployed binary is v1.4.0 and 213 commits behind,
and the package cannot report its own upstream commit. P7 closes this. Until it
does, any acceptance failure has two candidate causes and the first thing to
check is the version, not the pack.

**Shortening the record loses precision deliberately.** The previous revision
specified an on-disk audit layout, an fsync sequence, quarantine sharding, five
principals and a five-unit matrix. Several were wrong against the running
system and all were hard to reverse. Moving them to specs means the ADR says
less and what it says is more likely to survive contact.

## Alternatives considered

**Make `d2b-engineering` a composite pack that imports the upstream packs.**
Rejected on measured grounds. City-level import stamping overrides nested
bindings by design, so every `gc.run-operator` and `superpowers.implementer`
target becomes `d2b.<name>`; formulas still compile because run targets are not
validated at compile time, and the run stalls at first dispatch. Duplicate
agent names across two imports would additionally fail city loading. Sibling
imports plus a catalog entry deliver the same operator experience and break
nothing.

**Hand-roll the lifecycle rather than extend `build-base`.** Rejected. It is
what the previous revision did, and it forks a supported extension contract
that already provides the stages, the artifact schemas, the review-fix loop,
the publication variables and the `gc.publisher` role. The fork's cost is
permanent and invisible until an upstream fix does not arrive.

**Block unattended panel orchestration until cryptographically trusted receipts
exist.** Rejected as unsatisfiable. It contradicted the requirement that Gas
City orchestrate the panel in v1: every run would park, satisfying one decision
by violating the other, and two acceptance items asserted opposite outcomes for
the same build. Deriving the binding from the orchestrator's own dispatch
record is strictly better than a self-asserted string and is buildable.

**Keep five principals and a custom append-only audit store.** Rejected.
Delivery state is uid-owned `0700` and refuses to live inside a Git tree, so a
separate publisher uid cannot read the seal it must render a PR body from. One
identity plus an explicit handoff satisfies the property; the store's period
and quarantine layout was mechanism this record should not have frozen.

**Confine agents in a network namespace with no host loopback.** Rejected. The
supervisor is loopback HTTP and beads run over a Dolt TCP port, and agent steps
are instructed to run `gc bd update`. The namespace would stop every step from
recording its result while an internet-egress test still passed. Owner-uid
egress filtering with loopback allowed confines what matters.

**Ship module-owned system units for a Gas City orchestrator.** Rejected.
There is no such binary or unit; `gc` owns a systemd **user** unit and restarts
it on drift, so a system-scope unit races it and a `BindsTo=` chain never
observes the real process dying.

**Use the `github` pack's App path for publication in v1.** Not chosen,
pending P4. It requires an App id and private key in a `0640` file shared with
the webhook and admin services, which cannot be both publisher-exclusive and
functional for those services, and its push command inherits cwd. A
repo-scoped PAT with an explicit working directory is smaller and matches one
operator.

**Use the `discord` pack as the approval controller.** Rejected for v1.
Interactions need inbound public HTTPS and the gateway delivers into agent
sessions, which are precisely the principal forbidden from originating
approvals. Local gate closure keeps the artifact-binding property, which is the
part that matters.

**Use `speckit.implement` as the executor.** Rejected. It would put a second
executor beside Gas City with no durable state, no bead graph and no
relationship to the panel or the publisher. Spec Kit declares a handoff at
exactly that point.

**Send bug fixes through the full feature phase.** Rejected. It manufactures a
specification nobody wanted and teaches the operator that gates are ceremony.
Route C enters at decompose and escalates on a closed list.

**Treat Compound Engineering as the binding panel.** Rejected on measured
grounds: 17 selector-gated lanes, no per-lane model or effort record, no
unanimity, no seat identity, and a synthesiser that collapses lanes into one
verdict. It cannot emit an attestation this gate accepts.

**Let `pr-pipeline` imply publication, or omit the panel result from the PR.**
Both rejected. `mol-pr-ship` ends at a readiness report and pushes nothing, and
the PR is the one place a human decides to merge, so the review result must be
legible there without going to look for it.

**Let Gas City merge.** Rejected. `v3` is protected and the merge is the point
of no return; every other control here bounds what reaches it rather than
removing it.
