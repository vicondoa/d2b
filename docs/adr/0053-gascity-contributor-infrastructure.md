# ADR 0053: Gas City as contributor infrastructure, not a d2b capability

- Status: Accepted
- Date: 2026-08-02
- Amended: 2026-08-04. The panel roster changes from a closed ten-role set to
  a **selected subset of a twelve-role pool**: seven mandatory seats plus
  every optional seat a deterministic trigger table selects, under a
  **surface-dependent floor** of ten seats on a code or operative-configuration
  candidate and eight on a documentation-only one. Three seats are added to the
  pool, `simplicity`, `agentic` and `reliability`, and one is **removed**,
  `rust`. `software` becomes the mandatory multi-language reviewer: it is
  reordered to hunt correctness before convention, and it carries an explicit
  **standards profile per language**, Rust, Python, Bash and POSIX shell, and
  Nix, activated mechanically from the changed paths and **bound by the
  controller** in the dispatch record rather than chosen by the seat. The depth
  the removed
  seat held, unsafe and FFI soundness, public API design, Cargo SemVer
  classification and workspace dependency direction, moves into `software`'s
  Rust profile. `security` gains an explicit adversarial penetration-tester
  mandate; `product` gains scope and gap analysis, external contract fidelity,
  and a controller-assigned Gas City profile for reviews of this record; `docs`
  gains intra-document coherence. The record gains one producer-written field,
  `relevant`, with effective relevance derived by the controller so a session
  cannot opt a reviewer out.
  **D21 is added** and carries the whole contract, with **M33 through M38**
  added as its acceptance items.
  **D7, D8, D9, D10, D20, P2, the non-goals list, M7, M10 and M14 are amended
  in place**, the "What we leverage upstream" rows for
  `packages/xtask/src/delivery/model.rs` and `panel.rs` are corrected, and
  Consequences and Alternatives gain the costs and the rejected options this
  change created. Three earlier statements are **withdrawn**: that `PanelRecord`,
  `PanelRequest` and `validate_record_set` are preserved byte-identical, that
  the roster is a closed ten-role set, and, from an earlier revision of this
  same amendment, that `rust` remains an optional depth seat.
  Every other decision in this record
  stands unchanged, and nothing here is superseded. None of this is implemented
  yet; the committed code still carries the ten-role roster, `PanelRole::Rust`
  included, re-measured on the
  amendment date. Supporting source and prompt-construction material lives in
  [`specs/0053-panel-prompt-sources.md`](specs/0053-panel-prompt-sources.md).
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

**Re-measured 2026-08-04, and still true of committed code.** `PANEL_ROLES` is
a closed ten-element array; `PanelRole` has ten variants under
`rename_all = "snake_case"`; `PanelRequest::validate` rejects any request whose
`roles` is not exactly `PANEL_ROLES` in order; `validate_record_set` requires
exactly ten files; `PanelAttestation::validate` rejects anything short of
unanimity with the literal message "ten of ten signoffs are required"; and
`DELIVERY_SCHEMA_VERSION` is a single shared constant, currently `2`, that
every delivery artifact is checked against by equality. **D21 changes all of
this and none of it is implemented yet.** The paragraphs above are the measured
baseline the amendment is written against, not a description of the target
state.

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
| d2b panel policy | [`packages/xtask/src/delivery/model.rs`](https://github.com/vicondoa/d2b/blob/b5881b2a6a42cd6e4db89c662c9df853f6a98d55/packages/xtask/src/delivery/model.rs) | The pinned provider, model and effort policy, unchanged by this record. `PANEL_ROLES` **is** changed: D21 replaces the closed ten-role constant with a twelve-role pool plus a mandatory subset. |
| d2b panel gate | [`packages/xtask/src/delivery/panel.rs`](https://github.com/vicondoa/d2b/blob/b5881b2a6a42cd6e4db89c662c9df853f6a98d55/packages/xtask/src/delivery/panel.rs) | `PanelRecord`, `PanelRequest` and `validate_record_set`. D8's byte-identity claim over these three is **withdrawn by D21**, which adds one record field and makes the roster check roster-driven, with the admitted seat count set by the candidate surface class. The `deny_unknown_fields` discipline, the distinctness checks and the unanimity predicate are unchanged. |
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
panel that runs its whole roster and then discovers it cannot attest has wasted
the expensive part and produced nothing.

Existing users get a cutover, not a deprecation window. This is contributor
tooling with one operator, so no window is required; what is required is that
the remedy is explicit. The preflight gains one operator spelling, `make panel-preflight`, running the
existing binding checks and the harness receipt resolver and version checks
together, with `scripts/copilot/check-bindings.mjs` as what the target invokes.
Two operator-facing spellings for one preflight is how a contributor ends up
running only half of it. The target is introduced by this implementation and
does not exist today; the current operator command stays the node script until
it does. The preflight failure names the unsupported or missing resolver and
the pinned Copilot CLI version to upgrade to.

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
end to end: it dispatches the selected roster, drives the rounds, supplies each
seat its snapshot, diff and validation context, collects verdicts, and blocks
progression until every seat on the roster has signed off. It does not own
semantics, which are unchanged except where D21 amends them: the roster is
controller-selected and controller-owned rather than session-chosen,
independent seats, pinned provider, model and effort policy, validation
evidence supplied rather than re-run, delta review after round one, content
change invalidating prior sign-off, `signoff` true iff `recommendations` is
empty, unanimity over the selected roster, records bound to the same
`candidate_id`, `content_id` and `snapshot_sha256`, and no lane attesting its
own work.

**Amended 2026-08-04.** The phrase "closed ten-role roster" in the sentence
above is **withdrawn**. D21 replaces it with a selected subset of a
twelve-role pool under a surface-dependent floor, ten seats on a code or
operative-configuration candidate and eight on a documentation-only one, with
seven seats always present. Nothing else in
D7 changes: the roster is still closed at dispatch time, still bound to the
candidate, and still not a thing a session may choose.

**Mechanically the panel is a `check` loop**, because `check` is the only
engine loop and cannot be combined with `gate`, `loop`, `expand`, `assignee` or
`retry`. `d2b-panel` is one step whose `[steps.check]` carries `max_attempts`
and whose exec mode invokes the admission script; every seat on the selected
roster is dispatched beneath it. Fix rounds use `d2b-panel-fix` extending
`fix-loop-base`. Round history lives in beads. P2 proves this shape or routes
to an external driver invoked from a single step.

**Binding assurance is derived, not asserted.** An earlier revision made
unattended orchestration conditional on cryptographically trusted receipts,
which contradicted the requirement that Gas City orchestrate the panel in the
first version: every run would park and the deployment would satisfy one
decision by violating the other. That rule is withdrawn. Instead:

- Every seat on the selected roster is dispatched through **one provider
  profile owned by the config repo**, so the intended binding is configuration
  rather than a claim.
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

1. **Dispatch.** It selects the roster per D21, launches or authorizes panel
   dispatch under a fixed provider, model and effort profile that it owns, and
   emits the trusted dispatch record carrying both the binding and the roster.
   If the seats must themselves run as Gas City sessions, the trusted binding
   and the trusted roster still come from the controller's own dispatch, never
   from session-written state. P2 must prove that; if it cannot, the Gas City
   panel does not ship.
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

> **Amended 2026-08-04.** The byte-identity sentence above is **withdrawn in
> part**. D21 adds one producer-written field to `PanelRecord`, makes the
> roster checks in `PanelRequest::validate`, `validate_record_set` and
> `PanelAttestation::validate` roster-driven rather than fixed at ten, with the
> admitted cardinality set by the candidate's surface class, and bumps
> `DELIVERY_SCHEMA_VERSION`. What survives unchanged is everything D8 was
> actually protecting: `deny_unknown_fields` on both types, every field
> mandatory, the pinned provider, model and effort constants, the distinctness
> checks over `run_id`, `receipt_locator` and `output_sha256`, the
> `signoff` iff empty-`recommendations` predicate, the refusal of any set
> containing a finding, and the rule that no producer string is ever evidence.
> The rest of D8 below, including both trusted-evidence variants, the typed
> verifier, the error taxonomy and the migration wrapper, is unaffected.

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
failure costs a preflight rather than a full roster of reviews. Partially
running the panel and failing at admission is specifically what this gate
exists to prevent.

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
| `HarnessVersionUnparseable` | The harness produced an unparseable version banner; upgrade to the pinned version |
| `HarnessReceiptUnresolvable` | The locator did not resolve; re-run the round so the harness issues a fresh receipt |
| `HarnessReceiptBindingMismatch` | The resolved binding contradicts the record; the panel must be re-run under the pinned binding |
| `SelfAssertedBindingRejected` | A record supplied binding strings instead of a locator; update the adapter to capture the receipt |

**The preflight becomes one repository command, `make panel-preflight`.** It
runs the existing binding checks together with the harness receipt resolver and
version preflight, and it is what must pass **before any seat is dispatched**.
Contributors get one command to remember rather than a list, and every error
variant below names it.

That target **does not exist yet**; it is proposed by this record and lands
with the implementation. Until then the operator command remains
`node scripts/copilot/check-bindings.mjs`, which is what
`docs/contributing/copilot-agents.md` documents today. Contributor docs
describe what works now; this record describes what replaces it.

Every variant carries a **structured remedy**, and that remedy is **derived,
never stored**. The error holds no remedy field at all. Instead a typed method
computes it:

```
fn remedies(&self, producer: ProducerContext) -> RemedyPlan
```

`ProducerContext` is closed, currently `Standalone` and
`GasCity { safe_stage: SafeStageId }`. `RemedyPlan` is an immutable typed
sequence that no caller can populate: it is produced only by this function,
which matches on the variant and the producer and returns the fixed ordered
actions **by construction**. `RemedyAction` is drawn from a closed set,
currently `RunPanelPreflight`, `RunPanelMigrate`, `UpgradePinnedHarness`,
`RerunOriginalPanelCommand` and `RetryGasCityPanelStage { stage: SafeStageId }`,
and `Display` renders the computed plan into the fixed command text.

A stored list was the earlier shape and it is withdrawn, because a stored list
is a field someone can build wrongly. It permits a Gas City error carrying
`RunPanelMigrate`, or a correct set in the wrong order, to exist in memory and
be rendered before any test notices, and the mitigation was to write tests
asserting those states are rejected. Making the order a total function of
variant and producer deletes the invalid states instead of detecting them: the
wrong plan is not a bug the type system tolerates and a test must catch, it is
a value that cannot be constructed. Callers and tests match on the returned
actions; nothing parses a message to decide what to do, and no action carries
argv or a free-form string.

**No error prints the panel invocation or its arguments, and none stores it
either.** An earlier revision required the exact active
`/d2b-panel-round <mode> ...` line in the error text; a later one replaced that
with an alias and a protected invocation mapping so the command could be
replayed. Both are withdrawn. Printing argv reintroduces the paths and
free-form operator input D17 excludes from every other surface, and storing it
buys a resume that a contributor already has: the standalone panel is started
from an interactive shell, so the original command is in shell history. A
protected store of invocations is retention burden and a new secret-adjacent
surface bought to solve a problem the terminal already solved.

The two producers therefore recover differently, because their situations
differ:

- **Standalone**: `RunPanelPreflight`, then `RerunOriginalPanelCommand`, whose
  rendered text tells the contributor to rerun the original panel command from
  their shell history. It does **not** echo that command.
- **Gas City**: the error names the formula stage and the retry route, which
  Gas City already holds durably in beads. No alias and no mapping are needed
  because the orchestrator knows where the run is.

**The core error is producer-neutral; the producer is an argument, not a
stored field.** The verifier does not know which producer it is serving and
should not have to: an error that always carried `RunPanelMigrate` would tell a
Gas City run to migrate a standalone skill it does not use. So the error stores
nothing producer-specific, and `remedies` takes the `ProducerContext` from the
caller that has it. Order is semantic, not cosmetic: a contributor told to run
the preflight before migrating will watch the preflight fail for the reason the
migration exists to remove.

The table below is what `remedies` returns. The core rows are the
producer-neutral spine, kept as a separate list because that is how the
decision is reviewed and how a new variant is reasoned about, but they are not
a value the error holds.

Core spine by variant:

- `HarnessResolverMissing`, `HarnessVersionUnsupported { current, supported }`
  and `HarnessVersionUnparseable`: `UpgradePinnedHarness`, then
  `RunPanelPreflight`.
- `HarnessReceiptUnresolvable`, `HarnessReceiptBindingMismatch` and
  `SelfAssertedBindingRejected`: `RunPanelPreflight`.

Returned plans, fixed per producer and variant:

- **Standalone**, `SelfAssertedBindingRejected`: `RunPanelMigrate`, then
  `RunPanelPreflight`, then `RerunOriginalPanelCommand`. The migration goes
  **first**, before the core spine, because the legacy manually-entered values
  live in the checked-out skill and the preflight cannot pass until they are
  gone.
- **Standalone**, every other variant: the core spine in its order, then
  `RerunOriginalPanelCommand` last.
- **Gas City**, every variant: the core spine in its order, then
  `RetryGasCityPanelStage { stage }` last, carrying the `SafeStageId` from the
  `ProducerContext`, and **never** `RunPanelMigrate`. `SafeStageId` is a
  bounded newtype over a closed stage identifier, not prose and not a formula
  path, so the action stays free of free-form strings like every other.

**`make panel-migrate` is a wrapper that fails closed**, not a documented pair
of git commands. **It moves the contributor's branch forward onto current
protected `v3`**, which is where the supported skill and adapter live. It does
**not** rebase onto the pinned supported revision. A pin is a historical commit
and `git rebase <pinned-sha>` onto one moves the branch **backwards**, throwing
away every protected commit merged since and producing a branch that no longer
contains the work it was based on. The pinned revision names the migration that
must be present, not the place to land.

So the pin is used as a **precondition, not a target**. The wrapper's preflight
fetches the canonical remote and verifies the required panel migration commit
is **reachable from** `upstream/v3`. If it is not, the wrapper refuses with a
typed error saying the supported migration is not published yet, which is the
honest diagnosis: the contributor cannot migrate to something that has not
landed, and detaching to the pinned SHA to get the files would be a worse
outcome than waiting.

**`origin` is the contributor's remote and the wrapper never touches it.**
Canonical `vicondoa/d2b` is reached through a remote named **`upstream`**, URL
exactly `https://github.com/vicondoa/d2b.git`, and the migration target is
`upstream/v3`.

An earlier revision had this backwards: it required `origin` to be canonical
and offered to rename a contributor's fork out of the way. That is withdrawn.
`origin` is normally the fork a contributor pushes to, so renaming it breaks
`git push`, any configured upstream tracking, and every habit and script built
on it, in service of a read-only fetch the wrapper could have done under a
different name. A migration tool that rearranges someone's push remote to
perform a rebase has taken far more than it needed. The fork/upstream layout is
also what a forked-repository workflow already looks like, so for most
contributors there is nothing to repair at all.

The wrapper reads `upstream` and never writes it. It performs no `git remote
set-url`, no `git remote rename`, and no automatic remote mutation of any kind;
where a remote must change, the contributor runs the command.

The wrapper **detects conflicts before it starts**, refusing with the tree
untouched rather than leaving a half-finished rebase. Its refusals name exact
commands and mutate nothing:

- **Dirty tree** (`DirtyTree`): run `git status --short`; then either commit
  the changes, or `git stash push -u -m panel-migrate`; then rerun
  `make panel-migrate`; and after it succeeds, `git stash pop` if that path was
  taken.
- **Conflicting update** (`ConflictingUpdate`): the wrapper **prints the paths
  it predicts will conflict**. It already computed that list to decide to
  refuse, so telling the contributor to go run `git status` to rediscover it is
  asking them to redo work the tool has already done, and `git status` on an
  untouched tree does not show a conflict that has not happened yet.

  Those paths are **advisory, for planning only**. A rebase replays commits one
  at a time and stops at whichever subset conflicts at that commit, so the
  predicted set is the union across the whole replay and is never the working
  set at any single stop. The wrapper therefore prints no bulk `git add` over
  the predicted list: a contributor who pastes that command stages paths that
  are not unmerged at this stop, including files the replay has not reached,
  and turns a conflict resolution into an unrelated content change that
  `git rebase --continue` then commits.

  The printed sequence is:

  ```
  git fetch upstream
  git rebase upstream/v3
  ```

  Then, **at each rebase stop**: `git status --short`; resolve only the files
  that are currently unmerged; `git add <resolved-paths-for-this-stop>`;
  `git rebase --continue`. To abandon the migration at any stop:
  `git rebase --abort`. After the rebase completes: `make panel-migrate`.

  `git fetch upstream` rather than `git fetch upstream v3`, because the latter
  updates `FETCH_HEAD` without reliably updating `refs/remotes/upstream/v3` on
  every supported Git configuration, and the next line resolves `upstream/v3`.
  An explicit refspec would work equally well; plain `git fetch upstream` is
  chosen because it behaves the same everywhere and needs no explanation.

  No refusal ever prints a rebase onto a pinned or otherwise historical SHA.
  Because detection precedes this explicit operator action, the wrapper itself
  leaves no in-progress rebase behind.
- **Upstream remote missing** (`UpstreamRemoteMissing`): no remote named
  `upstream` exists. This is the ordinary first-run case for a contributor who
  cloned their fork, and the repair adds a remote rather than moving one:

  ```
  git remote add upstream <canonical-url>
  git fetch upstream
  make panel-migrate
  ```

  `origin` is not mentioned, because nothing about it needs to change. This
  path prints **no rebase**: the target does not resolve yet, so a rebase
  command here would be a sequence the contributor cannot complete.

  **Exactly two canonical URLs are accepted, one per transport**:

  ```
  https://github.com/vicondoa/d2b.git
  git@github.com:vicondoa/d2b.git
  ```

  The SSH form is not a credential-bearing URL. `git@github.com:` is GitHub's
  fixed service account in the standard scp-like syntax, identical for every
  user, and the secret it authenticates with is a key on disk that the URL does
  not contain. Rejecting it as "userinfo" would confuse a constant with a
  credential and force SSH-only contributors onto a transport they may have
  deliberately not configured.

  Everything else stays rejected: any other userinfo, any other scp-like host
  or user, tokens and `x-access-token` forms, any other repository or owner,
  query strings and fragments, and `ssh://` URL forms. Two literals is the
  entire allowed set. An `ssh://git@github.com/vicondoa/d2b.git` is equivalent
  in effect but is a third spelling to validate and a third to keep in step, so
  it is excluded until someone needs it.

  **The wrapper picks the transport deterministically from `origin`.** If
  `origin`'s URL is a GitHub URL, `upstream` gets the matching transport: an
  `https://github.com/...` origin selects the HTTPS canonical URL, a
  `git@github.com:...` origin selects the SSH one. In every other case,
  including no `origin` at all, a non-GitHub host, or a URL it cannot parse, it
  selects **HTTPS**.

  HTTPS is the default because this remote is fetched and never pushed, and
  anonymous HTTPS fetch of a public repository works with no configuration,
  while SSH fails for anyone without a key registered. Matching `origin` first
  matters because a contributor on SSH usually has HTTPS credentials unset
  entirely, and handing them an HTTPS remote produces a prompt rather than a
  clone. The rule is a total function of one observable value, so the wrapper
  never asks a question and two runs in the same tree render the same command.
- **Upstream remote mismatch** (`UpstreamRemoteMismatch`): a remote named
  `upstream` exists with a URL equal to neither canonical URL. Either accepted
  form is canonical and proceeds normally; only a third value refuses. The
  wrapper performs **no `git remote set-url` and no rename**, and emits no
  mutating command. Hijacking a remote a contributor configured deliberately,
  perhaps a mirror or a second project, would be the tool silently deciding it
  owns a name it merely wanted.

  The refusal prints `git remote get-url upstream` so the contributor can see
  what is actually configured, prints **both** accepted canonical URLs so they
  know what would satisfy the check, and asks them to choose a remote
  arrangement themselves before rerunning `make panel-migrate`. It does **not**
  print the configured non-canonical URL: that value is contributor
  configuration this record has no claim on, and it is exactly the kind of
  string that turns out to carry a token. The contributor reads it with the
  printed command, in their own terminal, rather than receiving it back through
  an error surface. The record does not prescribe which arrangement to adopt,
  because the right answer depends on what that remote is for and only the
  contributor knows.
- **Canonical target missing** (`CanonicalTargetMissing`): `upstream` is
  present with an accepted canonical URL, the fetch succeeded, and there is still no
  `upstream/v3`. No change of remote layout repairs a branch that is not there,
  so this refusal carries **no git command at all**. It states that the
  canonical protected branch is unavailable or not yet published, and that the
  remedy is outside the contributor's tree: wait for it to be published, or
  contact the repository owner. This is deliberately not a generic network or
  access diagnosis. The fetch worked; what is absent is the branch.

A genuine network or access failure still fails at `git fetch upstream` and is
reported by Git itself, which already says it better than a wrapper would. What
this record requires is that every **typed** diagnosis above names its specific
condition, never a generic access message that covers several and explains
none.

A future mode that starts a rebase before detecting conflicts would be a
separate design with the same per-stop remedies. The v1 preflight mode refuses
with the tree untouched.

Naming `git rebase upstream/v3` is safe precisely because the wrapper has
already determined the conflict and mutated nothing: the contributor is
starting the rebase deliberately, with the predicted paths in front of them,
rather than discovering mid-operation that a tool left them in a state they did
not ask for.

**Redaction is type-level, the type set is closed, and the outer `Debug` is
derived on purpose.** The governed surface is a **declared closed set of
types**, not one type and whatever hangs off it: `PanelReceiptError`,
`RemedyPlan`, `ProducerContext`, `RemedyAction`, and any other independently
governed payload type added later. Each is governed on its own account,
because `remedies` computes a plan rather than storing one, so `RemedyPlan`
and its actions are never reachable by following fields from the error.
Governance here is a list someone maintains, not a graph walk.

**Every** field of every governed type, and every field of every type nested
inside one, must come from a **closed approved set** of safe types: redacting
newtypes, closed enums, bounded numeric, version or stage newtypes. No raw
`String`, no `OsString`, no `Path` or `PathBuf`, no arbitrary map or vector of
text, and no type absent from the approved set may appear anywhere in any of
those trees.

The policy is deliberately **not** scoped to fields someone marked protected.
"Protected" is a judgement made by whoever added the field, and the field that
leaks is the one nobody thought to label: a `context: String` added to help
debugging, a `path` kept for a better message. An allowlist inverts the
default, so an unrecognised type is a failure rather than an omission, and a
contributor adding a field either reuses an approved type or argues the new one
onto the list in review.

Only after an **exhaustive field census** passes does the enum **derive**
`Debug`. An earlier revision required an explicit hand-written `Debug` as a
second layer. That is withdrawn, because with a closed safe type set it is not
a second layer but a liability in the other direction: a hand-written formatter
is a list someone must remember to extend, so a field added later is silently
**omitted** from the rendering, and the diagnostic a contributor needs during a
failing panel is missing while nothing fails to warn them. A derived `Debug`
over fields that cannot print unsafely is both complete and safe, and it stays
correct when the enum grows. Safety comes from the census; completeness comes
from the derive.

`Display` remains hand-written and redacting. It is not a dump of fields but a
deliberate operator-facing message, so it is written rather than derived: a
closed reason, the fixed remedy commands rendered from the `RemedyPlan` that
`remedies` computed, and bounded safe version newtypes where the variant has
them.

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

**This survives the 2026-08-04 amendment.** D21's roster artifact and
continuity ledger are **controller-side**, not gate inputs: the gate still sees
exactly one final unanimous set and still has no round ordinal. What it gains
is a roster, a surface class and a `held` set carried on the trusted dispatch
record it already consumes, which is one more field group on an existing input
rather than a new artifact class in xtask.

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

**Publication remotes are the publisher's, not the contributor's.** The
publisher pushes from `<handoff-repo>`, a clone it controls under the handoff
identity, whose `origin` is the canonical repository it was cloned from. That
`origin` has nothing to do with the remotes in a contributor's working tree,
which is why `make panel-migrate` leaving `origin` alone costs publication
nothing: the two never share a repository. A publisher that resolved a remote
name out of a contributor's checkout would be taking a push target from an
untrusted layout, so it does not; the handoff clone's remote is established
when the clone is created and is not read from anywhere else.

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
carries the panel result as `<n>/<n> unanimous`, where `n` is the size of the
attested roster, taken from the attested record set, the integration commit,
`snapshot_sha256`, `candidate_id`, `content_id`, the round count, the surface
class the roster was sized from, a per-seat table of role, verdict and receipt
locator that renders a `relevant: false` pass **distinctly** from a substantive
sign-off, the selection reason for every seat, a validation evidence summary by
reference or digest, the route input summary, the verification matrix summary,
the simplification outcome, unresolved risks, and an explicit statement that
merge requires human action.
It carries no transcripts, credentials, raw identifiers or authenticated URLs,
and it is bounded in bytes. PR creation is impossible while any finding stands,
a roster seat is missing, the roster violates D21's composition rules, the
snapshot is stale, the binding is underived, or the publication approval is
absent or bound to different bytes.

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
only, and no protected observable surface holds a field that is not a redacting
newtype, so its `Debug` rendering is safe by construction whether derived or
written.

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

This alias exists for **background log correlation only**, and its mapping is
bounded and expires under the same retention as the records that reference it.
It is deliberately not a resume handle: D8 stores no panel invocation and
offers no replay, so nothing here retains a command to be replayed and there is
no second, longer-lived mapping to keep.

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
invocation; the Discord approval adapter, if P5 permits one; and, added by the
2026-08-04 amendment, the wire shape of the change-surface artifact, the panel
roster artifact and the continuity ledger that D21 requires. An ADR that
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

**D21. The panel roster is a selected subset of a twelve-role pool under a
surface-dependent floor, and selection is controller-owned.** *Added by the
2026-08-04 amendment. It amends D7 and D8 and is the single place the panel
composition contract lives.*

A fixed roster of ten was wrong in both directions at once. On an ADR-text
change it ran `kernel`, `networking` and `nixos` against prose those seats have
nothing to say about, at three sessions of cost and three chances for a
peripheral nit to force another round. On a code change it was simultaneously
too thin: nobody owned deletion, nobody owned the agent and prompt surface that
now decides how most work gets done here, and nobody owned resource ownership,
cleanup on crash paths, restart adoption and on-disk state migration, which is
the defect class this repository's own decision set is mostly about (ADR 0011,
0027, 0034, 0040, 0049). Widening the pool and selecting from it fixes both
ends, and it is the smaller change: the seats already exist as a concept, the
record already keys on role, and the gate already refuses anything that is not
unanimous.

**The pool is closed at twelve.**

| Class | Seats |
| --- | --- |
| **Mandatory**, on every panel, never removable | `software`, `test`, `product`, `docs`, `security`, `observability`, `simplicity` |
| **Optional**, selected by trigger | `reliability`, `agentic`, `nixos`, `networking`, `kernel` |

`simplicity`, `agentic` and `reliability` are new. **`rust` is removed from the
pool**, and its territory moves into `software`, which becomes the mandatory
multi-language reviewer carrying an explicit standards profile per language.
`software`, `security`, `product` and `docs` change scope. `nixos`,
`networking` and `kernel` are unchanged in scope and move from mandatory to
optional. The committed collection point for the sources, prompt requirements
and anti-patterns of all twelve, plus the `software` language profiles and the
Gas City `product` profile, is
[`specs/0053-panel-prompt-sources.md`](specs/0053-panel-prompt-sources.md).
Every seat prompt cites it; it is decision-support material and is not
dispatched to anything.

**Every seat name is a single lowercase word.** That is a constraint, not an
accident, and the reason is in the implementation hazard below. `simplify`
would have been a verb naming an action on a panel where every finding costs a
full round, so the seat is named `simplicity` after the property it defends.
`agentic-coding` would have been the first two-word seat and the first place
three independent spellings of a role could silently disagree, so the seat is
named `agentic`, which is also the more accurate name: it reviews agent
configuration and orchestration contracts, not a coding practice.

**Composition invariants.** For any panel on any candidate:

1. `roster` is a subset of the pool.
2. Every mandatory seat is in `roster`. There is no candidate class, no
   escape hatch and no operator flag that removes one.
3. **The floor is surface-dependent**, evaluated from the classifier below:
   - `class = code-operative`: `|roster intersect optional| >= 3`, so at least
     three of the five optional seats.
   - `class = docs-only`: `|roster intersect optional| >= 1`.
4. `|roster| >= 10` when `class = code-operative`, and `|roster| >= 8` when
   `class = docs-only`. This follows from 2 and 3 and is checked separately
   anyway, because a check that holds only by derivation is one refactor away
   from not holding.
5. `roster` is larger than its floor whenever the trigger table selects more
   **distinct** optional seats than the floor requires. Selection is not a
   quota: it takes every optional seat whose trigger matches, not the first
   three. Two rules that select the same seat select one seat.
6. Exactly one record per roster seat, and no record for a seat outside
   `roster`.

**Why the floor is conditional rather than a constant.** Ten seats on a code
change is the size this repository wants, and the arithmetic makes that
concrete: seven mandatory plus a floor of ten means at least three of the five
optional seats are always dispatched on code. Ten seats on an ADR is the
original defect, restated. A single unconditional ten would fill a
documentation-only roster from the head of the fill order regardless of what
the candidate contains, seating `reliability`, `agentic` and `nixos` against
prose none of them can read. That is exactly the thing D21 exists to stop, so
the constant is not worth its simplicity here. Cost is not linear in seats
either: because `signoff` is true if and only if `recommendations` is empty,
one non-empty record re-runs the whole roster, so seat count multiplies into
round count rather than adding to it.

**The surface classifier is a deny-by-default allowlist, and it fails closed.**
`class(candidate)` is `docs-only` if and only if **every** path in the change
surface satisfies `docs_only(p)`; otherwise it is `code-operative`. All five
conditions must hold for `docs_only(p)` to be true:

1. `p` ends in `.md`. Every other extension, and every extensionless path, is
   code-operative. A `.json`, `.nix`, `.rs`, `.sh`, `.py`, `.toml`, `.bzl` or
   `.mjs` path is code-operative whatever directory it sits in.
2. `p` is under `docs/`, `specs/` or `changelog.d/`, or `p` is one of the five
   tracked root Markdown files `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`,
   `SECURITY.md`, `TODO.md`. Everything else is code-operative by construction,
   which deliberately includes `tests/`, `proofs/`, `labs/`, `examples/`,
   `templates/` and `.github/` even where the file is Markdown. `docs/` here
   includes `docs/adr/` and `docs/adr/specs/`, which is what makes this record
   and its supporting specification a documentation-only candidate.
3. `p` is not `AGENTS.md` and does not end in `/AGENTS.md`. Those are operative
   instruction files that agents load at runtime; the extension is Markdown and
   the function is configuration.
4. `p` is not under `.github/`. Redundant against 2 and stated anyway, because
   this is the exact trap the rule exists for: operative prompts that look like
   documentation. Measured 2026-08-04, `.github/` holds `agents/`, `skills/`,
   `workflows/` and a pull-request template; `agents/*.agent.md` and
   `skills/*/SKILL.md` are the Markdown that is really configuration, and
   `prompts/`, `instructions/` and `copilot-instructions.md` do not exist here
   yet but are the vendor's standard locations and would be the same trap.
5. `p` is not a generated contract. The generated set is carried in the
   versioned table as a literal path list, because the gate has no repository
   access and cannot read the gate script that owns it. Measured 2026-08-04,
   the `drift_paths` array in `tests/unit/gates/drift-check.sh` is
   `docs/reference/schemas/`, `docs/reference/error-codes.md`,
   `docs/reference/daemon-api.md`, `docs/manpages/`, `docs/completions/`,
   `docs/reference/cli-output/`, `docs/specs/ADR-046-spec-set.json`,
   `docs/specs/ADR-046-work-items.json`,
   `docs/specs/ADR-046-implementation-graph.json`,
   `docs/specs/ADR-046-implementation-graph.md`, `nixos-modules/generated/`,
   `packages/d2b-contracts/src/generated`, `packages/d2b-guestd/src/generated`
   and `packages/d2b-resource-api/src/generated`. A drift test asserts the
   table's copy equals that array, so the duplication is checked rather than
   trusted.

Three refusals complete it, all in the fail-closed direction: an **empty**
change surface classifies `code-operative`; a **rename** is classified from
both sides and is `code-operative` if either side is; and anything the
classifier cannot decide, including an unrecognised path shape in a future
tree layout, classifies `code-operative`. There is no "probably docs" outcome.
The cost of being wrong toward code is three extra seats; the cost of being
wrong toward docs is a code change reviewed by an eight-seat panel with no
specialist on it.

**Selection is two steps, and the split is what makes the second one
gate-checkable.** First `match(change_surface) -> matched_rules`, evaluated by
the controller against the versioned table below. Then
`select(matched_rules, class, held) -> roster`, a pure set computation:

```
roster = mandatory
       union { seat(rule) : rule in matched_rules }
       union held
       union quorum_fill(class)
```

where `matched_rules` is the set of **seat rule** identifiers from the table
below that match the candidate's change surface, `held` is the sticky set
defined further down, and `quorum_fill` takes seats in the fixed order
`[reliability, agentic, nixos, networking, kernel]`, skipping any already
selected, until invariant 3 holds for `class`. The result is a **set**, so a
seat selected three ways appears once and carries the highest-precedence reason
in the order `mandatory > held > triggered > quorum_fill`.

**The fill order is ranked by breadth, not by importance.** `reliability` leads
because it is the only optional seat with something to say about nearly every
candidate: any change that opens a resource, spawns a child, writes state or
bumps a schema constant is in its territory. `agentic` is second because the
agent and prompt surface is the most frequently touched non-code surface in
this tree. `nixos` is third because Nix is the second language of this tree and
`nixos` is the only language territory left in the pool once Rust moved into
`software`. `networking` and `kernel` are last because they are the two
narrowest territories and the two most likely to have nothing to say.

**Removing `rust` from the pool makes the fill tighter, and that cost is
named.** Three of five optional seats on code is a higher forced fraction than
three of six was, and the seat that vacated the third fill slot is the one that
had something to say about the language most of this tree is written in.
The concrete case is a change confined to `packages/*/src/**.rs` that matches
no `reliability` path or token: no optional seat rule matches at all, so the
roster reaches ten by seating `nixos` on a diff with no Nix in it. Note what
this is **not**: it is not a saving. The floor is unchanged, so on a candidate
at its floor the roster is the same ten it always was, with a different and
worse third optional seat. The saving appears only on candidates that already
triggered more than three optionals. That
is a real irrelevance and it is accepted for two reasons. The Rust review did
not disappear and did not shrink: it moved into `software`, which is mandatory,
is never rotatable out of a later round the way an optional seat is, and whose
Rust profile activates on exactly the paths `rust-sources` used to match. And
the cost of the wrong fill is one session and a truthful
`relevant: false`, which carries `signoff: true` and therefore cannot force a
round, while the roster artifact records `quorum_fill` as the reason so the
pattern is visible to whoever reads the PR body. If that pattern turns out to
be common, the remedy is a table change, which is a reviewed commit, not a
floor exemption.

**The trigger table is a versioned constant in `packages/xtask`, not a
heuristic.** It starts at **version 1** with the settled pool: no selector for
mandatory `simplicity`, no selector for the removed `rust` seat, deterministic
`reliability` and `agentic` rules, and the four `software-*-profile` rules.
Leaving a dead row in a constant the gate recomputes is a trap: a later reader
would assume the selector can change an outcome, while a rule naming a seat
outside the pool has no resolvable target at all.

Path rules match repository-relative paths on either side of a rename, and
apply to every path in the change surface. **Content rules match added lines
only, case-sensitively, against a fixed token list, and are evaluated only on
paths that individually fail `docs_only(p)`.** A token in prose is a citation,
not a call: this record mentions `pidfd`, `deny_unknown_fields` and
`DELIVERY_SCHEMA_VERSION` repeatedly without any of them being a use. The last
five rows are **profile rules**, not seat rules: they select nobody and bind a
profile onto a seat that is mandatory anyway.

| Rule id | Selects | Matches |
| --- | --- | --- |
| `nix-sources` | `nixos` | `**/*.nix`, `flake.lock`, `nixos-modules/**`, `nix/**`, `pkgs/**`, `templates/**`, `examples/**` |
| `net-paths` | `networking` | `nixos-modules/network*.nix`, `nixos-modules/net.nix`, and any path whose basename contains `firewall`, `nftables`, `bridge`, `vsock`, `dhcp` or `dns` |
| `net-tokens` | `networking` | added line contains any of `nftables`, `iptables`, `AF_VSOCK`, `systemd.network`, `169.254.`, `bind(`, `listen(`, `resolv` |
| `kernel-paths` | `kernel` | `nixos-modules/minijail*`, `packages/d2b-priv-broker/**`, `packages/d2b-guest-shell-runner/**` |
| `kernel-tokens` | `kernel` | added line contains any of `pidfd`, `cgroup`, `clone3`, `unshare(`, `setns`, `seccomp`, `ioctl`, `openat2`, `RESOLVE_`, `MS_`, `/proc/`, `/sys/fs/cgroup` |
| `reliability-paths` | `reliability` | `packages/xtask/src/delivery/**`, `packages/d2bd/src/**`, `packages/d2b-priv-broker/src/**`, `packages/d2b-resource-store*/**`, `nixos-modules/store.nix`, and any path under `packages/*/src/**` whose basename contains `storage`, `state`, `lifecycle`, `session`, `shutdown`, `restart`, `pool`, `adopt`, `lock`, `lease`, `sync`, `reconcile`, `supervisor` or `cleanup` |
| `reliability-tokens` | `reliability` | added line contains any of `Drop for`, `tokio::spawn`, `thread::spawn`, `JoinHandle`, `Mutex<`, `RwLock<`, `Atomic`, `catch_unwind`, `rename(`, `fsync`, `O_TMPFILE`, `SCHEMA_VERSION`, `deny_unknown_fields`, `EBUSY` |
| `agentic-paths` | `agentic` | `.github/agents/**`, `.github/prompts/**`, `.github/instructions/**`, `.github/skills/**`, `.github/copilot-instructions.md`, `scripts/copilot/**`, `.gc/**`, `**/AGENTS.md`, `**/*.formula.toml`, `**/pack.toml`, `**/prompt.template.md`, `docs/contributing/copilot-agents.md`, `docs/contributing/panel-review.md`, `docs/adr/0053-gascity-contributor-infrastructure.md`, `docs/adr/specs/0053-panel-prompt-sources.md` |
| `software-rust-profile` | nobody; adds `rust` to `software.profiles` | `**/*.rs`, `**/Cargo.toml`, `Cargo.lock`, `rust-toolchain*.toml`, `**/BUILD.bazel`, `**/*.bzl`, `MODULE.bazel*` |
| `software-python-profile` | nobody; adds `python` to `software.profiles` | `**/*.py`, `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements*.txt` |
| `software-shell-profile` | nobody; adds `shell` to `software.profiles` | `**/*.sh`, `**/*.bash`, any extensionless path whose parent directory is named `bin` or `tools`, or added line starting with any of `#!/bin/sh`, `#!/usr/bin/env sh`, `#!/bin/bash`, `#!/usr/bin/env bash`, `#!/bin/dash`, `# shellcheck shell=` |
| `software-nix-profile` | nobody; adds `nix` to `software.profiles` | `**/*.nix`, `flake.lock` |
| `adr-0053-product-profile` | nobody; binds `product.profile = gascity` | `docs/adr/0053-*.md`, `docs/adr/specs/0053-*` |

**`software`'s language profiles are controller-derived, exactly like
`product`'s.** The four `software-*-profile` rules bind a set,
`software.profiles`, in the dispatch record; the gate refuses a `software`
record produced under a different set for that candidate. This reuses the
mechanism `adr-0053-product-profile` already needs rather than inventing a
second one, and it converts "did the seat apply the Rust standards?" from a
prompt hope into a dispatch fact a human can read in the PR body. Six
properties are decided here:

- **Every applicable profile activates.** A diff touching `.rs`, `.sh` and
  `.nix` binds all three. There is no primary-language election, because the
  defect a mixed diff hides is exactly the one at the language boundary.
- **The empty set is legal and is not an abstention.** A change confined to
  `scripts/copilot/check-bindings.mjs`, to a `Makefile`, or to a `.json`
  schema binds no profile, and `software` still runs its shared
  correctness-first and local-convention sections over it. Measured
  2026-08-04, `.mjs` is the one real source type in this tree with no profile;
  a fifth profile is the extension point if JavaScript grows here, and until
  then an unrecognised source type is a reason to review without a standards
  citation, never a reason to abstain.
- **Profiles are scope, not selection reason.** Telling a seat which languages
  its delta contains is a statement about the delta. Telling it why it is on
  the roster is not, and stays controller-side under the rule further down.
- **`software-nix-profile` is deliberately narrower than `nix-sources`.** The
  seat rule fires on the whole Nix surface including directories that carry no
  `.nix` file in the delta, because that is where module wiring lives; the
  profile fires on Nix source, because that is what carries Nix code quality.
  The overlap is intended and the ownership split is stated below.
- **`software-shell-profile` is the one rule with both a path clause and a
  content clause**, because this tree commits four executable shell scripts
  with no extension and a suffix rule would miss them. Its parent-directory
  clause catches the two under `tests/tools/` and the two under a `bin/`
  directory measured 2026-08-04; its shebang clause catches anything else,
  under the same restriction every content rule carries, that it is evaluated
  only on paths which individually fail `docs_only(p)`. A shebang quoted in
  prose therefore never binds a profile.
- **Bound-exceeded binds every profile, not none.** When an oversized change
  surface makes selection return the entire pool, profile binding returns the
  entire profile set by the same reasoning: an over-cited standard costs a
  paragraph, an un-applied one costs the review.

Worked example, and the one that matters first: a change confined to
`docs/adr/0053-gascity-contributor-infrastructure.md`, `docs/adr/README.md` and
`docs/adr/specs/0053-panel-prompt-sources.md`. Every path is `.md`, under
`docs/`, is not an `AGENTS.md`, is not under `.github/` and is not in the
generated set, so `class = docs-only` and the floor is eight with one optional
seat. `agentic-paths` matches two of the three paths and selects `agentic`;
`adr-0053-product-profile` matches and binds the profile; no
`software-*-profile` rule matches, so `software.profiles` is empty and the seat
runs its shared sections only; no content rule is evaluated at all, because no
path in this surface is code-operative. One optional seat satisfies the floor,
so `quorum_fill` adds nothing. The roster is the seven mandatory seats plus
`agentic`, **exactly eight**, and `product` runs under the Gas City profile.
**That is the roster for the panel review of this amendment.**

**Over-selection is the fail-closed direction.** The change surface is a new
bounded artifact, derived by the delivery tooling from the candidate's
`base_oid..head_oid` across the repository set and digested into the dispatch
record, so nothing downstream needs a Git tree to reason about it. If a
candidate exceeds its bound, selection returns the **entire pool**. We can
survive reviewing something twice; we cannot survive silently not reviewing it.
The same rule applies to an unrecognised rule identifier or a trigger-table
version the reader does not implement: refuse, or select everything, never
select less.

**The `product` profile is controller-assigned, never session-declared.** When
`adr-0053-product-profile` matches, the controller sets the `product` seat's
profile to `gascity` in the dispatch record and the gate refuses a `product`
record produced under any other profile for that candidate. Under that profile
the seat must check upstream behaviour claims against commit-pinned normative
Gas City specifications and source, not against guides, and must respect the
normativity ladder Gas City's own reference index declares. This exists because
this record is almost entirely claims about somebody else's software, and a
product review that reads the guides agrees with everything the record already
says.

**Seat scope, stated here because the roster is only as good as its ownership
map.** The prompt-level detail is in the supporting specification; what belongs
in the decision is the boundary between seats, because that is what stops a
wider pool from producing duplicate blocking findings.

- **`software` reviews correctness first, and it is explicitly
  language-profiled.** Its prompt is ordered: mentally execute the changed
  control flow and hunt boundary and off-by-one errors, absence and null
  propagation where the language has it, race and
  time-of-check-to-time-of-use, invalid state transitions, and broken error
  propagation; then structure, readability and error handling; then local
  coding, naming, file and directory conventions; then measured performance.
  Those four sections are **shared and always run**, whatever the delta
  contains. On top of them the prompt carries one **standards profile per
  language** - Rust, Python, Bash and POSIX shell, and Nix - each naming its
  own normative sources, and the profiles that run are exactly the ones the
  `software-*-profile` rules bound for this candidate. It follows
  repository-local conventions first and cites the exact local or external rule
  behind every convention finding. Formatting-only findings stay non-blocking.
  A `software` record whose findings are all convention-level while a logic
  defect sits in the delta has not done the work, and the order exists to make
  that a stated failure rather than a matter of taste.
- **The Rust profile carries the depth where being wrong is silent**: unsafe
  and FFI soundness, public API design, the Cargo SemVer classification of a
  signature change, workspace dependency direction, error-source chains, Rust
  naming and idiom, and Rust-specific performance. Those duties were an
  optional seat before this amendment. Coverage of Rust candidates is
  unchanged, because the rule that selected that seat matched the same paths
  the profile now activates on. What changes is that the depth arrives inside a
  seat that has already read the whole delta, and that it can no longer be
  rotated out of a later round, because a mandatory seat never can. **It does
  not generally save a session**, and the arithmetic is worth stating because
  it is the opposite of what "one fewer seat" suggests: on a candidate whose
  triggers already reach the floor, `quorum_fill` replaces the vacated seat
  with the next one in the order and the roster is the same size, which is the
  irrelevance cost named above. A session is saved only on candidates that
  triggered more than three optional seats, where the roster was above its
  floor to begin with. **Performance belongs to
  `software`** whole, so the most measurement-hungry topic in the pool has one
  owner and no seat boundary runs through it.
- **`nixos` is the only language specialist left, and the split with
  `software`'s Nix profile is explicit.** `software` owns general Nix code
  quality: readability, naming, idiom, `with`-scope and `let` hygiene, dead
  bindings, and the formatter boundary. `nixos` owns the module system: option
  declarations and types, `mkDefault` versus `mkForce`, merge semantics,
  assertions, activation ordering, and NixOS-specific correctness including
  ADR 0015's three-root-unit rule. A finding about how a Nix expression reads
  is `software`'s; a finding about what the module system will do with it is
  `nixos`'s. Where a candidate contains Nix, both are usually present, and this
  is the one place in the pool where two seats read the same file on purpose.
- **`product` gains scope and gap analysis and external contract fidelity.** It
  enumerates the decision and acceptance items of the artifact under review and
  states, per item, covered or not covered by this delta, plus anything in the
  delta that no item asked for. It owns CLI surface and exit codes, wire and
  artifact schema compatibility, version discipline, and the operator migration
  and upgrade path, and any change to a serialized type or a schema constant
  gets an explicit compatibility statement: break, additive, or version bump.
  It also owns cross-decision consistency and supersession. Under the Gas City
  profile it owns the **truth and normativity** of this record's claims about
  upstream; `agentic` owns the **mechanics** of what d2b itself authors.
- **`docs` gains intra-document coherence** on top of Diataxis placement,
  changelog fragments, schema-to-prose drift, ADR index coverage, and the
  process-marker and dash rules: contradictions between sections, terminology
  drift, forward references to undefined things, statements two careful readers
  would read differently, and cross-links that do not resolve. For a repository
  whose primary artifact is a self-amending record of this length, that is the
  highest-value reading available and it costs nothing extra.
- **`simplicity` is mandatory and carries two lenses**, because a mandatory
  seat with a code-only charter is a no-op on the most common candidate class
  here. The **code lens** is unchanged: the simplest maintainable
  implementation that meets the stated requirements, reuse of a mature
  supported library over a hand-rolled one, no reinvented wheels, no needless
  indirection, and deletion where deletion lowers risk. The **artifact lens**
  applies to an ADR, a specification or a plan: is the decision surface
  minimal, does the record reinvent behaviour upstream already provides, is a
  previously rejected alternative being reintroduced without new evidence, and
  is a contract stated once rather than duplicated between prose and schema.
  Its rejections stand and are rejections, not preferences: code golf, lost
  validation or error handling or tests or observability sold as
  simplification, dependency sprawl, complexity laundering behind a dependency
  or macro or configuration surface, unsupported or unmaintained dependencies,
  and abstraction churn.
- **`reliability` is new and its boundary is explicit**, because it borders
  three seats that already exist. It owns resource ownership and cleanup on
  error and crash paths, restart, adoption and idempotency, ordering and
  concurrency across components, partial-failure and degraded-state behaviour,
  and on-disk state and schema migration. `kernel` owns syscall and
  kernel-interface semantics and version floors; `software` owns in-function
  correctness and error propagation; `test` owns whether any of it is covered;
  `product` owns the operator-facing migration and compatibility experience.
  `reliability` owns the design property across components, which is the
  question none of those four asks.
- **`security` carries the adversarial mandate**: state the attacker model and
  the capability assumed, then reach something with it, and carry a concrete
  exploitation path on every blocking finding.

**The verdict gains exactly one producer-written field.** `PanelRecord` adds
`relevant: bool`. Nothing else about the record becomes producer-written,
because a producer-written field is a forgery surface and D7's whole posture is
that a producer string is never evidence. Round ordinal, selection reason, rule
identifier, seat profile, surface class and effective relevance are
**controller-derived** and live in the dispatch record, the roster artifact and
the continuity ledger.

Record-local rules, both rejections:

- The existing predicate is unchanged and applies to **every** record:
  `signoff` is true if and only if `recommendations` is empty.
- `relevant: false` requires `recommendations` empty. Combined with the above,
  a not-relevant record therefore always carries `signoff: true`. A
  not-relevant record with a recommendation is invalid.

`relevant: false` is a **pass, not an abstention**. Unanimity is therefore
unchanged as a predicate: every seat on the roster signed off. What changes is
only that the roster is variable, so the message stops saying ten of ten.

**The selection reason never reaches the seat.** The prompt carries the seat's
scope, the snapshot, the diff ranges and the validation evidence, and nothing
about why that seat is on this roster. `selection_reason`, including the
`quorum_fill` value, lives in the dispatch record, the roster artifact, the
seal and the PR body, where a human reads it. This is decided rather than left
open because both leaks are bad in opposite directions: a seat told it was
seated by quorum fill writes `relevant: false` reflexively and the floor buys
nothing, while a seat told it was triggered by a specific rule reviews the rule
instead of the diff. A seat decides its own relevance from the change surface
it was given, which is the only input that can produce an honest answer.

**Effective relevance is derived by the controller and latches.**

```
effective_relevant(seat) = OR over every round k so far of
                           ( record(k).relevant OR record(k).recommendations non-empty )
```

Once true it never becomes false for that candidate lineage. A later
`relevant: false` from a seat that has already latched is **recorded and
ignored** for relevance and continuity; it is not a rejection, because refusing
an honest "I have nothing further" would cost a round to punish a verdict that
changes nothing. The ledger stores both the claim and the derived value, so the
divergence is visible rather than lost. **A session cannot opt a reviewer out
by writing `relevant: false`**, which is the whole reason relevance is derived
rather than asserted.

**Reviewer continuity.** For a candidate lineage:

- `held = { seat : effective_relevant(seat) and the seat's most recent record
  did not carry signoff true }`. Every seat in `held` is on the next round's
  roster by construction of `select`.
- A seat that has never been effectively relevant may be **rotated out** in a
  later round, subject to the composition invariants. A seat that has been
  effectively relevant and has not yet signed off may not.
- A seat whose most recent record carried `signoff: true` leaves `held`, but
  the trigger table immediately re-selects it if any of its rules still match
  the new change surface. Release is therefore only real when the change
  surface has genuinely moved away from that seat. This is the mechanism, not a
  promise.
- Seat **identity** is pinned while a seat is in `held`: the same role bound to
  the same provider, model, effort and prompt digest. Swapping a reviewer that
  is holding a finding for a fresh one is how a finding gets laundered, so it
  is forbidden until that reviewer returns a true sign-off.
- New specialists are added freely as fixes change the surface. Roster growth
  between rounds needs no justification beyond the trigger table.
- Mandatory seats are never in the rotatable set at all.

**Every held reviewer reads its own prior findings before writing a new
verdict.** A seat whose earlier record on this candidate lineage carried
recommendations opens its next record's summary by taking each of those
recommendations in turn and judging it **resolved or not resolved against the
new delta**, before it issues any verdict. This is the cheapest round-count
reduction available: today a held seat can restate a fixed finding or drop an
unfixed one with equal ease, and neither is visible. It is a prompt
requirement, not a gate refusal, and this record says so plainly rather than
implying a control it does not have: `recommendations` is the only structured
channel the record has, and D21 deliberately did not add a second one to carry
per-finding resolution state.

**Continuity and content invalidation are reconciled, not traded off.** The
existing rule stands untouched: any content change invalidates every prior
sign-off, and later rounds review the delta plus full context. The two rules
operate on different objects. Invalidation is about **verdicts**, which never
carry across a content change and are re-earned every round against the final
`content_id`. Continuity is about **roster membership and seat identity**,
which do carry across, precisely so that a content change cannot quietly drop
the reviewer whose finding caused it. A seat that signed off in round two has
not signed off on round three's bytes; it produces a fresh record or the panel
does not pass.

**Two floors stop the mandatory seats from becoming a rubber stamp.** Both are
derived from controller-owned inputs and both are refusals:

- At least one mandatory seat must be effectively relevant. A panel on which
  nobody read anything is not evidence of anything.
- If the candidate classifies `code-operative`, `software` must be effectively
  relevant. A code change that the general software reviewer found irrelevant
  is a contradiction, and the honest reading is that the seat did not do the
  work. This reuses the classifier rather than inventing a second path test, so
  there is one definition of what counts as code and both the floor and the
  seat count move together.

**What the gate enforces, and what it does not.** This boundary is stated
plainly because the alternative is a claim that does not survive contact.

- **The gate enforces**, from artifact bytes alone with no repository access:
  every composition invariant, including the surface class carried on the
  trusted dispatch record and the floor that class selects; that `roster`
  equals `select(matched_rules, class, held)` recomputed from the trusted
  dispatch record's own fields; that the roster in the request equals the
  roster in the dispatch record; one record per roster seat and none outside
  it; the record-local predicates; unanimity; that `held` is a subset of
  `roster`; that the dispatch record's controller-derived effective-relevance
  set is consistent with the final round it is looking at, so a seat writing
  `relevant: true` cannot be absent from it; both relevance floors
  evaluated over that set; and that every seat profile the dispatch record
  binds, `product.profile` and each member of `software.profiles`, is a
  declared profile of a seat that is on the roster, with each such record
  produced under exactly that binding.
- **The gate does not enforce** that `matched_rules`, `class` or the bound
  profile set is a truthful
  description of the diff, because the delivery gate has no Git tree by
  construction: the state root is refused inside any enclosing Git working
  tree. `match`, `classify` and `profiles` are the controller's, reproducible
  offline from
  the change-surface artifact by a recompute command and auditable from the
  roster artifact. Nor does the gate re-derive effective relevance across
  earlier rounds, since it only ever sees the final set: the continuity
  ledger's integrity comes from the append-only sink of D17, which the
  `gascity` uid cannot rewrite, not from a second mechanism invented here.
- **The named residual**, since a generic risk section is worthless: a
  controller that under-reports `matched_rules`, that misclassifies a
  code-operative candidate as docs-only, that drops a seat from `held`, or that
  binds a thinner profile set than the delta warrants,
  produces a smaller roster or a shallower `software` review that the gate will
  accept because it is internally
  consistent. What catches it is that the roster artifact records the
  change-surface digest, the surface class, the table version and the bound
  profile set, the recompute
  command reproduces `classify`, `match`, `profiles` and `select` from the same
  bytes, the
  ledger is append-only outside the `gascity` uid, and the roster with its
  per-seat reasons, its class and its profiles appears in the PR body where a
  human decides
  to merge. Misclassification is the most legible of the four, because the
  PR body carries a class and a seat count that must agree; a thin profile set
  is the least legible, because nothing downstream knows what languages the
  delta contained without recomputing. What does not catch
  it is anything automatic, and this record does not pretend otherwise.

**One concrete implementation hazard, named because it is silent.** `PanelRole`
serializes under `rename_all = "snake_case"`, `record_file_name` is built from
a hand-written `as_str`, and `scripts/copilot/check-bindings.mjs` derives its
expected agent filename by converting the enum variant to kebab case. Today
every role is a single word so all three spellings agree by accident, and D21
keeps that property deliberately: `simplicity`, `agentic` and `reliability` are
one word each, so `Simplicity` serializes `simplicity`, `as_str` returns
`simplicity`, and the script expects `panel-simplicity.agent.md`. The first
two-word seat breaks this without any type error, producing a record written
under one spelling and looked up under another, which surfaces as a missing
seat rather than as a naming bug. A test therefore asserts all three spellings
are the same string **for every pool member**, and it lands now rather than
with the seat that would need it.

**Version discipline is a clean break.** `DELIVERY_SCHEMA_VERSION` is a single
shared constant checked by equality, so adding a record field bumps it for
every delivery artifact and no cross-version record is readable. That is the
correct outcome here and matches D3's posture: this is contributor tooling with
one operator, so in-flight candidates re-run rather than acquiring a
compatibility path nobody will maintain. There is no v2 acceptance path.

**Three new seats and one removal mean three added seat prompts and one
deleted one, and the binding check already knows.** Measured 2026-08-04:
`scripts/copilot/check-bindings.mjs` parses `PANEL_ROLES` out of `model.rs`
with a regex that **fails closed** if it cannot find the array, derives the
expected agent file set from it in both directions, and requires the
`## The bar for a finding` block to be byte-identical across every
`panel-*.agent.md`. So the file set is already derived rather than hardcoded,
and adding three pool members automatically requires
`panel-simplicity.agent.md`, `panel-agentic.agent.md` and
`panel-reliability.agent.md` to exist carrying the same bar bytes. Because the
check is bidirectional, dropping `rust` from the pool automatically requires
`.github/agents/panel-rust.agent.md` to be **deleted in the same change**; the
Rust standards it carries are rewritten into the `software` prompt's Rust
profile, not orphaned. Ten committed agent files minus one plus three is
twelve, which is the pool size, and the check is what proves that rather than
this sentence. Four things must move with the constant, and each is a real
failure if it does not:

- The script's parse target. Renaming or reshaping the roster constant makes
  the regex miss and the check fail closed, which is the right failure but is
  still a failure until the script is updated in the same change.
- The bidirectional check must read the **pool**, not the mandatory set. An
  optional seat still needs a committed prompt; selection decides who runs, not
  who exists.
- The bar-mismatch error text, which says "All ten seats". That string is the
  one place the count is genuinely hardcoded.
- The `PanelRole::Rust` variant itself. Removing a variant from a serialized
  enum makes every record and fixture carrying `"role": "rust"` unreadable,
  which is the correct outcome under the clean-break rule above and is a
  failure the moment a stale fixture survives the change. The removal lands
  with the `DELIVERY_SCHEMA_VERSION` bump D21 already forces, not separately.

**None of this is implemented.** At the time of this amendment the committed
code has the closed ten-role roster measured above, which still contains
`PanelRole::Rust`, ten committed panel agents including
`panel-rust.agent.md`, and a binding check whose bar-mismatch message names ten
seats. D21 decides the
target; the implementation lands under the prototype and acceptance gates
below, and the contributor documentation that describes the panel to humans is
updated in the same change that ships the code, per the repository's rule that
contributor docs describe what works now.

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
| **P2** | `d2b-panel` as a `check` loop whose exec script shells `xtask ... panel-attest --records-stdin --dispatch-record <fd>` against a fixture wave; and the controller-emitted dispatch record the verifier re-derives binding **and roster** from, with the seats running as Gas City sessions. Run it three times: once on a documentation-only fixture whose change surface triggers exactly one optional seat, once on a code-operative fixture that triggers four, and once on a code-operative fixture that triggers none; and drive a two-round case in which a seat returns a finding in round one. | Round one non-unanimous spawns attempt two from the engine; round two unanimous closes the step and writes a seal; `max_attempts` is enforced by the dispatcher; binding **and roster** are re-derived from the **controller's** dispatch record, and a session-written claim can change neither. The documentation-only fixture dispatches exactly eight seats, the four-trigger code fixture dispatches eleven, and the zero-trigger code fixture dispatches ten by taking `reliability`, `agentic` and `nixos` from the fill order, all three matching `classify` and `select` recomputed offline. The seat that returned a finding in round one is present in round two under the same seat identity, and a session-written `relevant: false` from it does not remove it. | The panel becomes an external driver invoked from one step. If the controller cannot own dispatch **or roster selection** for session-run seats, the Gas City panel **does not ship** and the standalone producer remains the only supported one. No fail-open. |
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
  to xtask, and no change to the pinned provider, model and effort constants.
  **Amended 2026-08-04:** the former blanket "no change to `PanelRecord`,
  `PanelRequest`, `validate_record_set`" is narrowed by D21 to the properties
  it was protecting, listed in D8's amendment note. Widening the roster is not
  weakening a seam; every added seat is another seat that must sign off.
- **No roster chosen by a model, a session, or a prompt.** Selection is a total
  function of the change surface evaluated by the controller against a
  versioned constant table, and a session-written field never changes it.
- **No panel below its class floor**, ten seats on a code-operative candidate
  and eight on a documentation-only one, and no candidate class, escape hatch
  or flag that removes a mandatory seat. **No surface class inferred
  optimistically**: anything the classifier cannot decide is code-operative.
- **No selection reason disclosed to a seat.** The prompt receives scope and
  evidence; `quorum_fill` and every other selection reason stay controller-side
  and are read by a human in the PR body, never by the reviewer they describe.
- **No reviewer self-release.** A seat that has been effectively relevant
  cannot leave the roster by declaring itself irrelevant, and cannot be
  swapped for a different reviewer identity until it returns a true sign-off.
- **No second blocking channel.** `recommendations` remains the only field a
  finding can enter. Prior-finding resolution, residual risk and cross-seat
  observations live in the seat's summary, and this record does not pretend a
  prompt requirement is a gate.
- **No link-check gate** for the source set in
  [`specs/0053-panel-prompt-sources.md`](specs/0053-panel-prompt-sources.md),
  and **no twelve separate committed prompt-source files**. One collection
  point, retrieval dates, and explicit moving-source markers.
- **No prompt built from a leaked, extracted, unattributed or
  licence-incompatible source.** Structures and checklists are extracted from
  sources whose licence and provenance are recorded; prose is not copied; and a
  premade prompt's numeric thresholds are never adopted as if they were this
  repository's.
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
  xtask. Unanimity is over the **selected roster**, and the run is exercised on
  at least two candidates of different roster size, one of eight and one of at
  least ten, so a hardcoded cardinality cannot pass this item.
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

  **Continuity survives the invalidation.** The same test asserts that a seat
  which was effectively relevant before the mutation is still on the roster
  after it, under the same seat identity, and that its invalidated sign-off is
  not reused as evidence for the new content. An implementation that satisfies
  the invalidation half by discarding the roster along with the verdicts fails
  this item, because that is precisely the way a finding disappears.
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
  contains `<n>/<n> unanimous` from the attested set, where `n` is the attested
  roster size, the integration commit, `snapshot_sha256`, `candidate_id`,
  `content_id`, the round count, the surface class, **every roster seat** with
  verdict, selection reason and receipt locator, a validation summary by
  reference, the route input summary, the verification matrix summary, the
  simplification outcome, unresolved risks, and the merge-requires-human
  statement. A seat that passed with `relevant: false` renders **distinctly**
  from a substantive sign-off, asserted against a fixture containing both. It
  contains no transcript, credential, raw identifier or authenticated URL, and
  is within its size bound.

  Five planted bodies are each **rejected**: one whose numerator is below its
  denominator; one whose denominator is below the floor its stated class
  requires; one whose seat table omits a roster seat; one whose roster violates
  a composition invariant, at minimum by dropping a mandatory seat; and one
  whose stated class is `docs-only` while its seat count is ten, since a body
  that reports both is reporting a roster nobody could have selected. The test
  is run over rosters of eight, ten and more than ten, so a body renderer with
  `10` compiled into it fails rather than passing on the one size it was
  written for.
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
  records carry only approved fixed digests and closed enum values, and no
  protected observable surface holds a field that is not a redacting newtype,
  so its `Debug` rendering is safe by construction whether derived or
  written.

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
     secret-shaped token and control bytes, is **rejected** with
     `HarnessVersionUnparseable`. There is no normalization path: the parser
     does not salvage a version substring out of a banner it could not parse,
     because extracting a plausible-looking version from attacker-influenced
     bytes is how the hostile content gets carried forward under a safe-looking
     type.
  2. **Policy control: a multi-root recursive census, with planted controls.**
     The census takes an **explicit closed set of entry roots**. It does not
     start at one type and follow stored fields, because reachability is not
     the same as governance and this record broke that assumption itself:
     `remedies` computes a `RemedyPlan` rather than storing one, so there is no
     field edge from the error type to `RemedyPlan`, `RemedyAction` or
     `ProducerContext` at all. A reachability census rooted at the error enum
     would traverse a small tree, report success, and never look at the types
     that carry the operator-facing content. Every governed type is a root in
     its own right.

     The root set for the implementation is `PanelReceiptError`, `RemedyPlan`,
     `ProducerContext`, `RemedyAction`, and any other independently governed
     public or internal payload type in this surface. The set is **declared,
     versioned and closed**: adding a governed type means adding a root, and a
     governed type that is not a root is the census's blind spot rather than a
     silent pass. The check **fails closed** on an empty root set and on a
     declared root it cannot resolve.

     From every root the check **traverses the whole type tree**: every field
     of every struct, and every variant and every variant-field of every enum,
     recursively, at every nesting level. Enums are not a leaf. An enum whose
     variants are inspected only for their names, or a struct whose fields are
     inspected only one level down, is where the unsafe field survives.

     Each field's type must be a member of the closed approved set: redacting
     newtype, closed enum whose own variant-fields satisfy the same rule,
     bounded numeric, version or stage newtype. A raw `String`, `OsString`,
     `Path`, `PathBuf`, arbitrary text map or vector, or **any type the census
     does not recognise**, fails, whether or not anyone labelled it protected.
     Separately, the error enum is asserted to hold **no remedy field**, which
     is both the D8 invariant and the reason `RemedyPlan` must be censused as
     its own root.

     The derived `Debug` rendering of these types is then what control 3 scans,
     so the check covers the rendering that actually ships.

     Planted mock types are submitted to the same policy test and each must be
     **rejected**: a raw `String` field carrying no protected marking at all; a
     `PathBuf` field; a raw `String` on a **struct** field two levels below a
     root; a raw `String` or path on an **enum variant-field** two levels below
     a root, reached through another enum; and a planted violation inside the
     **`RemedyPlan` fixture specifically**, which must still be detected even
     though no field edge reaches it from the error fixture. That last control
     is what proves the census is multi-root rather than reachability-based;
     without it a single-root implementation passes the whole suite.

     The check reports, **per root**, the number of types, variants and fields
     it examined, and fails closed on an empty corpus for any root, on any type
     it cannot resolve, and on any shape it does not support, including a cycle
     it cannot traverse to a fixed point. Unresolved is a failure, not a pass:
     a census that silently skips what it cannot parse has counted the easy
     fields.

     **What lands when.** The panel receipt error enum does not exist yet, so
     this record does not claim a census over it. This pull request adds the
     census predicate, the declared root set and the planted fixtures as a
     Type 5 policy test now, exercising **multiple roots** over fixture types,
     which is non-vacuous because the planted mocks are real types the
     predicate accepts or rejects today. The implementation commit that
     introduces the real types adds them as roots to the **same** declared set
     rather than writing a second census, and the fixtures stay as the negative
     corpus. A predicate proven against fixtures and then pointed at the
     production types is a check that was working before the types it guards
     existed; a census written alongside the type is one nobody has seen
     reject anything.

  3. **Rendering control, with a planted negative.** The `Debug` and `Display`
     renderings of every variant are scanned and must contain no protected
     field, no invocation or argv, and no filesystem path. A **mock error type
     carrying an unredacted protected field, a path and an argv** is then
     routed through the same rendering check and must be **rejected**. Without
     that control the scan passes trivially because the real enum happens to
     hold only safe values today, which proves the enum's current contents and
     not the check.

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
    `HarnessVersionUnsupported { current, supported }` carrying both values as
    parsed newtypes;
  - a hostile or unparseable version banner returns
    `HarnessVersionUnparseable`, and **no** version value is extracted from it;
  - a locator that does not resolve returns `HarnessReceiptUnresolvable`;
  - a resolved binding contradicting the record returns
    `HarnessReceiptBindingMismatch`;
  - a submission carrying model and effort strings instead of a locator returns
    `SelfAssertedBindingRejected`.

  **Remedies are asserted as a computed plan, not a stored list.** Tests call
  `remedies(producer)` and match on the returned `RemedyPlan` rather than
  parsing a message.

  A table covers **every variant crossed with every producer**, twelve cells,
  each asserting the exact ordered plan: standalone `SelfAssertedBindingRejected`
  is `RunPanelMigrate`, then `RunPanelPreflight`, then
  `RerunOriginalPanelCommand`; standalone otherwise is the core spine in order
  ending with `RerunOriginalPanelCommand`; Gas City is the core spine in order
  ending with `RetryGasCityPanelStage { stage }` carrying the `SafeStageId`
  from the `ProducerContext`, for every variant. A cell with no expected plan
  fails the table.

  **Wrong plans are excluded structurally, not asserted against.** Earlier
  revisions required tests that built a permuted or wrongly-populated list and
  asserted it was rejected. Those are removed, because the type no longer
  represents them: `RemedyPlan` is constructible only by `remedies`, so there
  is no permuted list to instantiate and a test that appeared to build one
  would only be testing a test helper. What replaces them is a structural
  assertion through the policy mechanism that `RemedyPlan` exposes no public
  constructor, no public mutation and no `From` or collection conversion that
  would let a caller assemble one, and that no error variant carries a stored
  remedy field. A plan that cannot be built wrongly does not need a test
  proving the wrong one is caught; it needs a test proving it cannot be built.

  No action carries argv.

  **No rendered message contains an invocation, argv, or a path**, asserted by
  scanning every rendered message across all six variants. A test that accepts
  any refusal, or any remedy list, for any of these cases does not satisfy this
  item.

  **Display is verified positively, per action and in order.** A table test
  maps **every** `RemedyAction` variant to its expected safe rendered command
  or phrase and asserts the exact output. The same test asserts the **rendered
  order** of each computed plan against the fixed per-variant order: standalone
  `SelfAssertedBindingRejected` renders `make panel-migrate`, then
  `make panel-preflight`, then the rerun phrase, in that sequence; a Gas City
  plan renders the core spine before `RetryGasCityPanelStage`. Control flow
  still never parses a string; this covers the other half, that a correct plan
  does not render into something useless, out of sequence, or wrong for a
  human. A new variant with no expected rendering fails the test.

  `make panel-migrate` is exercised over a corpus of **seven contexts**, one
  success and six refusals, with its **output** asserted, not only its exit
  status. Seven is the exact size of the migration-state corpus, and the test
  asserts that size so a new state cannot be added without a case here.
  Transport selection is covered separately by a counted five-origin corpus,
  and both canonical configured upstream URLs are covered by two positive
  fixtures; those seven additional fixtures do not multiply the state machine:

  - **clean tree**: brings the branch onto current `upstream/v3`, carrying the
    supported skill and adapter, and the resulting head is asserted to be a
    descendant of the fetched `upstream/v3` rather than of any pinned SHA;
  - **unpublished migration** (`UnpublishedMigration`): the required panel
    migration commit is planted as **not reachable** from the fetched
    `upstream/v3`. The wrapper refuses with the typed error, emits **no git
    command**, and mutates nothing;
  - **dirty tree** (`DirtyTree`): refuses, output contains exactly
    `git status --short`, `git stash push -u -m panel-migrate`, the rerun of
    `make panel-migrate`, and `git stash pop`, and the tree is byte-identical
    afterwards;
  - **conflicting update** (`ConflictingUpdate`): refuses, output contains the
    **predicted would-conflict paths** matched against the known planted set,
    then `git fetch upstream`, `git rebase upstream/v3`, the per-stop sequence
    `git status --short`, `git add <resolved-paths-for-this-stop>`,
    `git rebase --continue`, the abandon branch `git rebase --abort`, and the
    exact rerun command, and the tree is byte-identical afterwards. An output
    that names `git status` instead of printing the paths fails this case;
  - **upstream remote missing** (`UpstreamRemoteMissing`): no `upstream` remote
    is planted. Output is asserted to contain exactly
    `git remote add upstream <canonical-url>`, `git fetch upstream` and the
    `make panel-migrate` rerun, in that order, and **no rebase command**. A
    generic access or connectivity message fails this case, because it sends a
    contributor to debug a working network.

    **Transport selection is asserted per fixture, not merely that some
    canonical URL appears.** An `https://github.com/<user>/d2b.git` origin
    renders the HTTPS canonical URL; a `git@github.com:<user>/d2b.git` origin
    renders the SSH one; and no `origin`, a non-GitHub origin, and an
    unparseable origin each render HTTPS. A fixture that renders the wrong
    transport for its origin fails, so the rule is proven to be the total
    function D8 describes rather than a constant that happens to match one
    case;
  - **upstream remote mismatch** (`UpstreamRemoteMismatch`): an `upstream`
    remote is planted with a URL equal to neither canonical form. The wrapper
    is asserted to emit **no mutating git command**: no `git remote set-url`,
    no `git remote rename`, no `git remote add`, and nothing touching `origin`.
    It refuses with the typed error, whose output is asserted to contain
    `git remote get-url upstream`, **both** accepted canonical URLs, and the
    rerun command. The planted non-canonical URL is asserted **not** to appear
    anywhere in the output, since it is contributor configuration and may
    itself carry credentials. Two further fixtures plant `upstream` at each
    accepted canonical URL and assert the wrapper **does not refuse**, proving
    both transports are accepted rather than one being tolerated by accident;
  - **canonical branch missing** (`CanonicalTargetMissing`): `upstream` is
    planted with the canonical URL and the fetch resolves **no**
    `upstream/v3`. The wrapper refuses with the typed error, emits **no git
    command at all**, and the tree is unchanged. The output is asserted **not**
    to contain the missing-upstream repair, in particular no
    `git remote add`, since the remote is already canonical and re-adding it
    repairs nothing. It is also asserted not to render a generic network or
    access message: the fetch succeeded and the branch is what is absent.

  **`origin` is asserted to be untouched in every context.** No rendered
  output across the whole corpus may contain `origin` as the object of any
  command: no `git remote rename origin`, no `git remote set-url origin`, no
  `git remote remove origin`, no push-remote reconfiguration, and no
  `origin/v3` ref. A planted output renaming or re-pointing `origin` is
  rejected. This is the control for the inverted-remote failure: an instruction
  that reads plausibly and quietly breaks the remote a contributor pushes
  through.

  **The three upstream states are asserted to be distinct, not merely
  present.** Each of `UpstreamRemoteMissing`, `UpstreamRemoteMismatch` and
  `CanonicalTargetMissing` is asserted to render a command set the other two do
  not, and a fixture that renders the add-upstream repair for either of the
  other two is rejected. Collapsing them into one message would tell a
  contributor whose `upstream` is already correct to add it again, and a
  contributor with a deliberately different remote that the branch is missing.

  **The bulk-add shape is asserted to be rejected.** A planted renderer output
  containing a single `git add` whose arguments are the complete predicted
  conflict set fails the test, as does any `git add` argument that is a literal
  path rather than the per-stop placeholder. The predicted set is the union
  across the replay and is never the unmerged set at one stop, so a bulk add
  stages files the replay has not reached and converts a resolution into an
  unrelated committed change.

  **The renderer audit parses git command lines rather than scanning for
  keywords.** For every rendered refusal it takes each git command line and:

  - rejects any **40-hex object name anywhere on the line**, in a positional
    argument or inside a flag assignment, explicitly including `--onto=<sha>`,
    `--hard=<sha>` and the separated `--onto <sha>` form. A token-skipping scan
    that only inspects positional arguments passes `--onto=<sha>`, which is the
    backwards rebase wearing a flag;
  - rejects any git subcommand or flag **not on the allowed list** rather than
    ignoring what it does not recognise. Unrecognised is a failure: an audit
    that skips tokens it cannot classify is an audit an unfamiliar flag walks
    straight through. The allowed list covers exactly the commands these
    refusals may print: `git status --short`, `git stash push`, `git stash
    pop`, `git fetch upstream`, `git rebase upstream/v3`, `git add`,
    `git rebase --continue`, `git rebase --abort`, `git remote add upstream`
    and `git remote get-url upstream`. `git remote set-url` and
    `git remote rename` are **not** on the list in any form, because the
    wrapper never instructs a remote mutation beyond adding a missing one;
  - rejects any ref that is not `upstream/v3`, including `origin/v3`, a foreign
    remote, a foreign branch and a bare pinned revision;
  - rejects any command whose object is `origin`, in any position, so a
    rendering cannot rename, re-point or remove the contributor's push remote;
  - rejects any URL outside the exact two-literal canonical set,
    `https://github.com/vicondoa/d2b.git` and
    `git@github.com:vicondoa/d2b.git`. Planted rejections include a wrong SSH
    host (`git@git.example.invalid:vicondoa/d2b.git`), a wrong repository owner
    (`git@github.com:someone/d2b.git`), a wrong repository
    (`git@github.com:vicondoa/other.git`), an `ssh://` spelling of an otherwise
    correct target, a URL carrying a userinfo component or token, an
    `x-access-token` form, and a canonical URL with an appended query string or
    fragment. The standard `git@github.com:` prefix on the exact canonical
    repository is **accepted**: it is a fixed service account, not a secret,
    and treating it as userinfo would reject the ordinary SSH clone every
    SSH-only contributor already has.

  Refusals that carry no git command at all - `UnpublishedMigration` and
  `CanonicalTargetMissing` - are audited for the absence of any git command
  line, not merely for the absence of a rebase. `UpstreamRemoteMismatch` is
  audited for the absence of any **mutating** command, `git remote get-url`
  being a read.

  **Ordering is asserted as a complete constraint set**, not a membership
  check: `git fetch upstream` before `git rebase upstream/v3`; the rebase before
  `git status --short`; the status before `git add`; the add before
  `git rebase --continue`; and the continue before the `make panel-migrate`
  rerun. On the abandon branch, `git rebase --abort` comes after the rebase and
  before the rerun. Continue and abort are alternative branches, but both
  appear after the rebase, and neither may appear before it.

  Planted negative cases, each asserted to be **rejected**: `--onto=<sha>`;
  `--hard=<sha>`; `--onto <sha>` separated; a foreign ref such as
  `upstream/main` or `origin/v3`; a bare 40-hex revision as the rebase
  target; a bulk `git add` over the predicted set; an unrecognised git flag;
  and out-of-order renderings, at minimum `git add` before `git status`,
  `git rebase` before `git fetch`, and `git rebase --continue` before the
  rebase. On the target-unavailable path the constraint set is
  `git remote add upstream` before `git fetch upstream`, and that before the
  `make panel-migrate` rerun; a rendering that fetches `upstream` before adding
  it is rejected, because it instructs a fetch of a remote that does not exist
  yet. The scan
  counts the rendered refusals and command lines it examined and fails closed
  on an empty corpus.

  The renderer policy test and its fixtures land in **this** pull request,
  alongside the contributing-docs fail-open fix, so the backwards-rebase and
  bulk-add instructions are rejected by a check that exists before the wrapper
  does.

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

- **M32 The `panel-preflight` notice and the target land together, enforced by
  a lint that ships with this record.** The Type 5 policy lint is added in
  **this** pull request, in
  `packages/d2b-contract-tests/tests/policy_docs.rs` (or a sibling
  `policy_*.rs` in that crate if it is cleaner there). Type 5 is the
  repository's existing tier for source-and-docs consistency checks, so this
  reuses a mechanism rather than inventing one, and it adds no new gate because
  that crate already runs. The commit that adds it carries a changelog
  fragment; this record itself remains Proposed and fragment-free.

  The lint reads two inputs, the presence of a `panel-preflight` target in the
  `Makefile` and the operator command plus notice markers in the contributing
  doc, and admits exactly two states:

  | State | `Makefile` target | Docs operator command | Future notice |
  |---|---|---|---|
  | Current | absent | `node scripts/copilot/check-bindings.mjs` | present |
  | Implemented | present | `make panel-preflight` | absent |

  In the current state the doc must **not** present `make panel-preflight` as a
  command to run. In the implemented state the node command may still appear,
  but only as the underlying implementation or a debugging aid, never as the
  operator instruction.

  **Every mixed state is rejected**, and the lint plants a fixture for each so
  the rejection is observed rather than assumed. At minimum: target absent with
  the docs naming `make panel-preflight` as the operator command while the
  notice is still present, which is the exact state a panel round caught in
  this record's own history; target absent with the notice removed, which
  leaves contributors with a doc that points at nothing; target present with
  the node command still given as the operator instruction, which is the drift
  that leaves half the preflight unrun; and target present with the notice
  still there, telling contributors a shipped target does not exist. The
  planted fixtures are what make the lint non-vacuous today, since the live
  tree currently sits in one state and exercises one row.

  The lint keys on stable markers rather than prose. This change adds them to
  `docs/contributing/copilot-agents.md` now, following the repository's
  existing `<!-- BEGIN ... -->` and `<!-- END ... -->` convention:
  `PANEL-PREFLIGHT-COMMAND` around the operator command block and
  `PANEL-PREFLIGHT-NOTICE` around the future notice. The markers do not change
  the truthful current command, and they give the lint a machine-readable
  surface instead of sentences that a later edit will reword.

  The lint **fails closed** when either marker pair is missing or unbalanced,
  or when it cannot read the `Makefile`, since a consistency check that cannot
  locate both sides has not shown consistency. It names no Gas City symbol, so
  M1 continues to hold.

  This is the drift the rest of this record was written to avoid and then
  committed anyway: an earlier revision documented `make panel-preflight` as
  the operator command before any such target existed. Nothing caught it
  because nothing was looking. The rule is the repository's, not this record's:
  contributor docs describe what works now.

The six items below are added by the 2026-08-04 amendment and belong to D21.

- **M33 Selection is a total function, and the table is the only input.**
  A pure `select(matched_rules, class, held)` is exercised over a counted,
  non-empty fixture corpus of change surfaces, and each case asserts the
  **exact** roster set, not merely its size. The corpus covers at minimum: the
  documentation-only ADR change of D21's worked example, whose roster is
  exactly the seven mandatory seats plus `agentic` and nothing else; a
  Rust-only change under `packages/*/src/**`, whose roster is asserted to
  contain no seat selected by a language rule, because no such rule survives
  for Rust; a Nix-only change, whose roster contains `nixos`; a code change
  matching four or more optional rules, whose roster is larger than ten; and a
  code change matching none, which reaches ten by taking `reliability`,
  `agentic` and `nixos` from the fill order in that order. Running `select`
  twice on the same fixture returns byte-identical output, and running it on
  two fixtures that differ only in path ordering returns the same roster, so
  the function is proven order-independent rather than assumed to be.

  **Profile binding is asserted on the same corpus**, because a mandatory
  multi-language seat whose profiles are wrong is a seat that reviewed the
  wrong standards. A pure `profiles(change_surface) -> map` is exercised and
  each case asserts the exact set: the Rust-only fixture binds
  `software.profiles = {rust}`; the Nix-only fixture binds `{nix}`; a fixture
  touching `.rs`, `.sh` and `.nix` in one delta binds `{rust, shell, nix}`,
  so a mixed diff is proven to activate every applicable profile rather than
  electing one; a fixture touching only `scripts/copilot/check-bindings.mjs`
  binds the **empty set** and its roster still contains `software`, so an
  unrecognised source type is proven not to remove the seat; a fixture touching
  only `tests/tools/layer1-jobs`, which has no extension, binds `{shell}`; and
  the documentation-only fixture binds the empty set while
  `product.profile = gascity`. A planted `software` record whose bound profile
  set differs from the dispatch record's is refused.

  Three refusals are required, each its own typed error: a change surface
  beyond its bound, which selects the **entire pool** rather than failing open
  or selecting less; an unrecognised rule identifier; and a trigger-table
  version the reader does not implement. A fixture that selects **fewer** seats
  on any of the three fails this item.

  Two table-integrity controls are required as well: an assertion that every
  **seat** rule in the committed table selects a member of the optional set,
  which mechanically catches both a rule naming a mandatory seat, since such a
  rule can never change an outcome and is therefore a rule that lies, and a
  rule naming a seat outside the pool, of which `rust` is the concrete case this
  amendment creates; and an assertion that the committed table's declared
  version is 1. A companion assertion covers the **profile** rules: each
  selects nobody, and each names a seat in the pool and a profile in that
  seat's declared profile set.

- **M34 Every composition invariant is enforced as a refusal, with a planted
  control each.** Planted rosters are each **rejected**: one missing a
  mandatory seat; one on a `code-operative` candidate with fewer than three
  optional seats; one on a `docs-only` candidate with no optional seat; one of
  size nine on a `code-operative` candidate; one of size seven on a `docs-only`
  candidate; one carrying two records for the same seat; and one carrying a
  record for a seat that is not on the roster. One planted roster of size eight
  on a `docs-only` candidate, one of size ten on a `code-operative` candidate
  and one larger than ten are each **accepted**, so the check is shown to admit
  the sizes it must. The attestation error text is asserted to name the actual
  roster size rather than a literal ten, and a grep over the delivery crate
  asserts no remaining ten-of-ten string.

  Separately, the request roster, the trusted dispatch record's roster and the
  attested record set are asserted to agree, and three planted disagreements
  are each refused: a request roster larger than the dispatch record's, one
  smaller, and one of the same size with a substituted seat. A planted
  disagreement between the request's surface class and the dispatch record's is
  refused as its own typed error, because a candidate whose class is in dispute
  has no defined floor.

- **M35 A session cannot forge relevance, selection, or its own release.**
  Each of these is attempted from a producer and **refused or ignored**, and
  the test asserts which of the two, because they are different outcomes: a
  record carrying a roster, a surface class, a selection reason or a rule
  identifier is **refused**, since those fields are not the producer's to
  write; a record with `relevant: false` and a non-empty `recommendations` is
  **refused**; a `relevant: false` from a seat that already latched effectively
  relevant is **accepted and ignored**, with the ledger showing both the claim
  and the derived value, and the seat still present on the next round's roster;
  and a `relevant: false` from a seat that has never been relevant is
  **accepted** as a pass, after which that seat is observed to be rotatable out
  of a later roster while the composition invariants still hold.

  A three-round lineage is then driven end to end: a seat returns a finding in
  round one, is present in rounds two and three under the same pinned seat
  identity, and a planted attempt to substitute a different reviewer identity
  for it before it signs off is **refused**. A mandatory seat is planted for
  removal in every round and is **refused** every time.

  The prompt renderer is asserted **not** to interpolate `selection_reason`,
  `matched_rules` or the surface class into any seat prompt: the rendered bytes
  for a `quorum_fill` seat and for the same seat selected by trigger are
  asserted identical, and a planted renderer that includes the reason is
  **rejected**. Without that control the fill seats learn they were seated to
  satisfy a cardinality invariant, and the floor buys nothing.

- **M36 Both relevance floors refuse, and they read the classifier.** A panel
  on which every mandatory seat returned `relevant: false` is **refused**. A
  panel whose candidate classifies `code-operative` while `software` returned
  `relevant: false` is **refused**. Two positive controls are required
  alongside them so the floors are not satisfied by refusing everything: a
  `docs-only` panel on which `observability`, `reliability`-class concerns and
  others legitimately declare irrelevance while at least one mandatory seat is
  relevant **passes**, and a code panel on which `software` is relevant
  **passes**.

- **M37 The three new seats exist end to end, and no role name drifts.**
  `simplicity`, `agentic` and `reliability` are present in the pool, produce
  records the gate accepts, and `agentic` and `reliability` are selected by
  their own trigger rules on a fixture that matches each. `simplicity` is
  asserted present on **every** roster in the M33 corpus, including the
  documentation-only cases, since it is mandatory.

  A test asserts, for **every** pool member, that the serialized role string,
  `as_str`, and the kebab-case name `check-bindings.mjs` derives from the enum
  variant are the same string, and a planted variant whose three spellings do
  not agree is **rejected**. Every pool member is asserted to be a single
  lowercase word with no hyphen and no underscore, which is what makes the
  three spellings coincide; the planted control is a two-word variant, and it
  must fail. Without this the mismatch is silent: the record file is written
  under one spelling and looked up under the other, and the failure reads as a
  missing seat rather than as a naming bug.

  `check-bindings.mjs` is asserted to require a committed
  `panel-<role>.agent.md` for **every pool member**, optional seats included,
  and to reject a `panel-*` agent that is not a pool member. A planted pool
  member with no agent file is **refused**, and so is a planted agent file with
  no pool member. Its bar-mismatch message is asserted not to name a literal
  seat count, and a planted fourteenth file with a paraphrased bar block is
  **rejected**.

  The `product` Gas City profile is exercised: on a candidate matching
  `adr-0053-product-profile` the dispatch record carries
  `product.profile = gascity`, a `product` record produced under any other
  profile is **refused**, and a producer-declared profile is **refused**
  whatever its value.

- **M38 The surface classifier is conservative, and every trap case is
  asserted.** A pure `classify(change_surface) -> class` is exercised over a
  counted fixture corpus in which each of the following classifies
  **`code-operative`**, one case each: a change to
  `.github/agents/panel-software.agent.md`; a change to
  `.github/skills/d2b-panel-round/SKILL.md`; a change to a root `AGENTS.md`;
  a change to `tests/AGENTS.md`; a change to `tests/README.md`, which is
  Markdown but sits outside every allowlisted prefix; a change to
  `.github/PULL_REQUEST_TEMPLATE.md`; a change to `docs/reference/error-codes.md`,
  which is Markdown under `docs/` and is a generated contract; a change to
  `docs/reference/schemas/v2/bundle.json`; a change to
  `nixos-modules/store.nix`; a change to `tests/tools/layer1-jobs.py`; a change
  to a `Makefile`, which has no extension; an **empty** change surface; and a
  rename whose old side is `docs/explanation/design.md` and whose new side is
  `packages/d2b-core/src/design.rs`, plus the same rename in the other
  direction. Each of the following classifies **`docs-only`**: the three-file
  surface of D21's worked example; a `changelog.d/` fragment; a
  `docs/how-to/` page; a `docs/contributing/` page; and a `CHANGELOG.md` edit.

  The generated-path list carried in the versioned table is asserted equal to
  the `drift_paths` array in `tests/unit/gates/drift-check.sh`, and a planted
  divergence **fails**, so the duplication the gate's lack of repository access
  forces is checked rather than trusted. A mixed surface of one `.md` docs page
  and one `.rs` file classifies `code-operative`, and the resulting roster is
  asserted to be at least ten, so the mixed case is proven to take the higher
  floor rather than the average of two.

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

The paragraphs below are added by the 2026-08-04 amendment.

**A ten-seat floor on code is expensive, and the expense is multiplicative.**
Seven mandatory plus at least three of five optional means every code candidate
pays at
least ten reviewer sessions per round, and because `signoff` is true if and
only if `recommendations` is empty, one non-empty record re-runs all ten. Seat
count therefore multiplies into round count rather than adding to it: at a
per-seat blocking rate of one in ten, the expected total sessions to unanimity
is roughly half again what an eight-seat panel costs, and this repository's own
recorded datapoint is far worse than one in ten, a Wave-1 panel that returned
zero of eight sign-offs with eleven HIGH findings. The floor is accepted
because the failures a specialist catches are the ones the static gates do not,
and because the documentation-only class, which is the most common candidate
class here, is explicitly exempted. It is not accepted because it is cheap.

**A selected roster costs more code in the gate than a fixed one, and the cost
is real.** A closed array of ten was one constant, one length check and one
comparison. What replaces it is a versioned trigger table, a surface
classifier, a selection function, a bounded change-surface artifact, a roster
artifact, a continuity ledger, per-seat selection reasons, and a duplicated
generated-path list that must be kept equal to a shell array in another tree.
Every one of those is a thing that can be wrong, and the delivery crate is the
part of this repository where being wrong is most expensive. The trade is
accepted because the alternative was paying three irrelevant reviews on most
candidates forever, and because over-selection is the fail-closed direction on
every ambiguity.

**Seven mandatory seats will truthfully report irrelevance, and that is not a
malfunction.** On a documentation-only candidate, `observability` and often
`security` and `test` have nothing to say, and `relevant: false` is the honest
and cheap answer. The cost is that a record can read as unanimous while most of
its seats read nothing, and the sharp edge is that the same one-line pass is
available to a seat that simply did not look. The two floors in D21 catch only
the degenerate shapes: nobody relevant at all, and `software` irrelevant on a
code-operative candidate. Everything between those and a real review is caught
by the PR body rendering `relevant: false` distinctly from a sign-off, which is
a human control, not a gate. Naming it as a human control is the point; calling
it a guard would be the lie.

**A wider pool overlaps more, and the ownership map is the only thing stopping
it.** `simplicity` and `software` both have opinions about abstraction;
`security` and `observability` both look at what leaks into telemetry;
`reliability`, `kernel` and `software` all border on races; and `software`'s
Nix profile and `nixos` read the same file by design. With ten fixed
seats the overlaps were tolerable. With twelve they are not, and the
mitigation is thin: an ownership table restated in every prompt, the explicit
`reliability`-versus-`kernel`-versus-`software`-versus-`test` boundary in D21,
the explicit `software`-versus-`nixos` split on Nix, the single owner for
performance, and the rule that a seat noticing something
in another seat's territory writes an observation rather than a
`recommendation`. That rule has no field to enforce it, because
`recommendations` is the only blocking channel the record has and this
amendment deliberately did not add a second one. The honest position is that
duplicate blocking findings are a prompt-quality problem, not a mechanical one,
and the first sign of failure will be rounds that grow rather than shrink.
Merging `rust` into `software` removed one seat boundary and therefore one
overlap, which is the one arithmetic improvement in this paragraph; it moved
that overlap inside a single prompt, where the failure mode changes shape
rather than disappearing, as the next paragraph records.

**Merging Rust into `software` buys one fewer seat and costs prompt weight,
and the specific failure it makes possible is shallow breadth.** The
`software` prompt now carries four shared sections plus four language profiles,
each with its own normative source set, and it is by a wide margin the longest
seat prompt in the pool. The failure that creates is not the old one, a seat
told to be a generalist that reviews nothing in depth; it is the new one, a
seat with five hundred lines of standards that reads the first section of each
and blocks on naming while an unsound `unsafe` block sits in the delta. Two
mechanisms are aimed at exactly that. The **order is stated and is a
requirement**: correctness before structure before convention before
performance, and a record whose findings are all convention-level while a logic
defect sits in the delta has not done the work. And **profile activation is
controller-bound rather than seat-chosen**, so the prompt cannot be diluted by
a seat quietly skipping the profile whose depth is expensive; the dispatch
record says which profiles applied and the gate refuses a record produced under
a different set. Neither mechanism proves depth. What they buy is that a
shallow review is visible in the record rather than indistinguishable from a
thorough one, and the first sign of failure will be `software` records whose
findings cluster in the convention section on candidates that changed Rust.

**Continuity state is machinery that did not exist, and it is the part most
likely to be got wrong first.** `held`, latched effective relevance, pinned
seat identity and the append-only ledger are four coupled mechanisms whose
whole purpose is to make one failure impossible, a finding disappearing between
rounds. Each is individually simple and together they are a state machine
carried across rounds by a controller the gate cannot audit. The first question
on any acceptance failure in this area is whether the roster the controller
computed is the roster the gate recomputed; the second is whether `held` was
computed from the ledger or from the round in front of it.

**The sources this panel reasons from move, some are nobody's standard, and
five seats plus one language profile have no premade prompt at all.** During
source collection four
GitHub Copilot documentation URLs redirected or returned 404, VS Code relocated
its whole customization tree, OpenTelemetry retired an attribute-naming path,
Anthropic moved its Claude Code guidance from `anthropic.com/engineering` to
`code.claude.com`, `github/awesome-copilot` removed its entire `prompts/` and
`chatmodes/` asset classes, and Diataxis rate-limited automated fetches.
Several load-bearing sources are advisory rather than normative: Google's
engineering practices, the SRE books, the Prometheus practices pages which say
up front that they are not requirements, and the upstream Gas City and
Compound Engineering prompts, which are product code and can change without
notice. Worse for the promise of "ideal prompts", an enumeration of five
collections found **no** ready-made review prompt for `nixos`, `networking`,
`kernel` or `reliability`, only vendor-specific ones for `observability`, and
none carrying the unsafe, FFI and SemVer depth that `software`'s Rust profile
now owns; those seats and that profile are authored from normative
specifications and local code instead,
which is more work and a thinner safety net. Merging `rust` into `software`
moved that gap rather than closing it: it is now a source gap inside one
profile of a mandatory prompt, where it is easier to overlook than it was as a
seat with its own file. The mitigation is retrieval dates,
explicit normative-versus-advisory markers, a licensing ladder, and a rule that
only a normative source or a demonstrable defect makes a finding blocking.
There is deliberately **no link-check gate**: a network-dependent blocking
check is a flaky blocking check, and a moved documentation URL is not a reason
to stop a merge.

**Local conventions and remote guidance disagree, and the disagreements now
need explicit decisions rather than silent precedence.** The `software` seat is
told to follow repository-local conventions first, which is `AGENTS.md`'s rule,
but on several points there is no local rule to follow and the external one
does not fit, and every one of them now lands inside a language profile of the
one mandatory seat rather than being spread across two.
Measured 2026-08-04: all twelve tracked Python files use
`kebab-case.py`, which PEP 8 forbids for modules and which makes them
unimportable; the Google Shell Style Guide is Bash-only while seven tracked
scripts declare a POSIX `sh` shebang, five as `#!/usr/bin/env sh` and two as
`#!/bin/sh`; Rust's own API guidelines mark crate and
feature naming "unclear"; and no standard exists for general directory naming.
Each of those is now a stated position in the supporting specification rather
than a seat's judgement call, and the Python one is a settled migration that
this record does not perform. The cost is that the panel cannot ship a
`snake_case.py` rule until that migration lands, so the seat prompt carries an
explicit transition rule in the meantime, and a transition rule is one more
thing that must be removed on time or it becomes permanent.

**Nothing here is implemented, and the gap is now larger than it was.** Before
this amendment the record described a Gas City panel path over an existing
ten-seat gate. It now also describes a gate that does not exist: a twelve-role
pool, a surface classifier, a `relevant` field, a selection function, a
per-seat profile binding and three
artifact classes. Any acceptance failure in this area has one more candidate
cause than it did, and the first question is whether the roster the controller
computed is the roster the gate recomputed.

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

The alternatives below were considered for the 2026-08-04 amendment.

**Keep the closed, fixed ten-role roster.** Rejected. It was cheap to check and
it is the reason three seats produce a sign-off on prose they have nothing to
say about, at three sessions of cost per candidate and three chances to force
another round on a peripheral nit. It also has no room for the three reviews
this repository most needs, deletion, the agent surface, and resource lifecycle
across error paths, without becoming a fixed twelve and making the prose case
three times worse. Note what is **not** rejected here: ten is a good number for
a code panel, and D21 keeps it as the floor for that class. What is rejected is
ten as a constant that ignores what changed.

**Make the floor an unconditional ten.** Rejected, and this is the closest
call in the amendment. It keeps one number instead of two and removes the
classifier and every trap case the classifier has to get right, which is real
simplicity in the part of the system where being wrong is most expensive. It
fails on arithmetic. Seven mandatory plus a floor of ten forces three of five
optional seats onto every candidate, so a change confined to `docs/how-to/`
that matches no trigger would seat `reliability`, `agentic` and `nixos` on
prose. That is the original defect with two of the three names changed, and it
would be paid on the most common candidate class in this tree. The conditional
floor costs a classifier; the unconditional one costs the thing the amendment
was written to fix.

**Fix the roster at exactly eight.** Rejected, and it is the more tempting
error because it keeps a constant. A hard eight forces a choice between
specialists whenever more triggers fire than seats remain, and the choice is
made by whatever tiebreak the implementation happens to have. A candidate that
touches a Nix module, a firewall rule, a syscall and a restart path needs four
specialists;
capping it at one means three of those reviews silently do not happen and the
record still reads as unanimous. Eight is now the floor for documentation only,
ten is the floor for code, and the ceiling in both cases is the pool.

**Let a model or a heuristic pick the roster.** Rejected outright. It is not
auditable, not reproducible, and not gate-checkable: two runs on the same
candidate could dispatch different seats, and no later reader could tell
whether a missing specialist was a judgement or a mistake. Worse, it puts
roster selection inside the same session boundary D7 spent an entire decision
excluding from every other authority. Upstream Compound Engineering does
exactly this and forbids keyword matching outright, so this is a deliberate
divergence rather than an oversight: a versioned constant table is less clever
and can be re-run offline by anyone with the change surface, which is the
property that makes the gate's recomputation check possible at all.

**Merge `reliability` into `software`, `test` and `kernel` rather than adding a
seat.** Rejected. That is where the concern lives today and it is why nobody
owns it. `software` reviews correctness inside a function; `test` reviews
whether a restart path is covered, which presumes somebody already decided what
the restart path should be; `kernel` reviews whether `pidfd_open` was used
correctly, not whether the descriptor is closed on the error branch three
frames up. The question that falls between all three is who owns this resource,
who releases it when the process dies here, and what the on-disk state means
after that. For a repository whose decision set is ADR 0011, 0027, 0034, 0040
and 0049, that is the largest unowned territory in the pool, and splitting it
three ways is how it stayed unowned. It is optional rather than mandatory
because a pure-documentation candidate genuinely does not have it, and it leads
the fill order because almost every code candidate does.

**Make simplification a step in the implementation formula rather than a
reviewer.** Rejected. `pr-pipeline`'s `mol-pr-ship` already offers exactly that
shape and it stops at a report; more to the point, a simplification step runs
before the change is finished and reviews the author's own work, which is the
one arrangement this record forbids everywhere else. Simplification needs a
verdict that can block, a seat identity, and a pinned binding, and only a
reviewer has those. A step produces advice; a seat produces a sign-off.

**Leave `simplicity` optional rather than mandatory.** Rejected. Its two
triggers, dependency-manifest paths and net-added source lines, fired on
essentially every code candidate already, so optional status bought nothing on
code and cost the two dead rules D21 deletes. What it did cost was the
documentation case, where the seat was absent precisely when a record was most
at risk of restating a contract twice or reintroducing a rejected alternative.
The price of making it mandatory is that a code-only charter would make it a
no-op on prose, which is why D21 gives it an artifact lens rather than just a
promotion.

**Keep `rust` as a separate optional depth seat.** Rejected, and this reverses
an earlier revision of this same amendment, which is recorded rather than
quietly rewritten. The argument for keeping it was that the depth where being
wrong is silent - unsafe and FFI soundness, the Cargo SemVer classification of
a public API change, workspace dependency direction - rests on different
documents and a different reading posture from general code review, and would
be a prompt nobody reads if it were bolted onto a generalist. What defeated it
is that the generalist stopped being a generalist. `software` is mandatory and
now carries an explicit standards profile per language, each with its own
normative source set, activated mechanically from the changed paths and bound
by the controller rather than chosen by the seat. Once the Rust profile exists
and is provably active on every candidate containing Rust, a separate seat is a
second reader of the same documents on the same candidates, whose non-overlap
with the first reader has to be maintained by prose. Two smaller facts
sealed it.
Performance had already moved wholly to `software`, which was the one topic
both seats plausibly owned, so the boundary that remained was thinner than it
looked. And an enumeration of five prompt collections found no premade deep
Rust review prompt at all, so the seat was going to be authored from the Rust
API Guidelines, the Cargo SemVer reference and the Rustonomicon either way -
and those are sources, not a seat.

**The cost is stated plainly: the `software` prompt is now the largest in the
pool and must enforce activation and priority itself.** It carries four shared
sections and four language profiles where it used to carry breadth and defer
depth. The failure that creates is shallow breadth - a seat that skims every
profile and blocks on naming while an unsound `unsafe` block sits in the delta.
D21 answers it with a stated review order that is a requirement rather than a
suggestion, and with controller-bound profile activation that the gate checks,
so a review that skipped the expensive profile is visible in the record. Those
make the failure legible; they do not make it impossible, and Consequences says
so. `nixos` stays a separate seat under the same reasoning applied honestly:
its territory is the module system's evaluation and merge semantics, not Nix
code quality, and that is a different question rather than a deeper reading of
the same documents.

**Let a reviewer release itself after prior relevance.** Rejected, and this is
the load-bearing rejection of the amendment. If a `relevant: false` could
retire a seat that had already raised a finding, the cheapest way past a
blocking review would be to run the round again and have the seat declare the
matter no longer its concern. Effective relevance is therefore derived by the
controller and latches, a later `relevant: false` from a latched seat is
recorded and ignored, and seat identity stays pinned until a true sign-off.
A never-relevant seat may still be rotated out, because nothing is being
escaped there.

**Disclose the selection reason to the seat.** Rejected. It looks like
transparency and it is a leak. A seat told it was seated by `quorum_fill`
learns that no rule matched it and writes `relevant: false` reflexively, at
which point the floor is a headcount rather than a review; a seat told which
rule matched reviews the rule rather than the diff. The reason is written to
the roster artifact, the seal and the PR body, where a human reads it and can
act on it, which is the audience it was always for.

**Add separate seats for previous comments, gap analysis, API contract,
deployment verification and coherence.** Rejected. Those are the five
conditional lanes upstream Compound Engineering carries that d2b does not, and
every one of them is a **lens** on a seat that already exists rather than a
territory of its own: prior-finding resolution is a duty of every held seat,
scope and gap analysis and external contract fidelity and the operator upgrade
path belong to `product`, and intra-document coherence belongs to `docs`. Each
would have been a seat that runs on every candidate, produces a record, and can
block, which is the most expensive way to add a checklist item. The same
reasoning now excludes language lanes as well: after this amendment language
depth is a **profile of `software`**, activated from the change surface, rather
than a seat with its own verdict. `nixos` remains a seat because the module
system is a different question, not a deeper reading of the same documents.

**Copy a leaked, extracted or licence-incompatible prompt.** Rejected, and
stated as a rule rather than a preference because the temptation is real: the
best-known reviewer prompts in circulation are extractions. Three concrete
refusals came out of source collection. Third-party compilations attributed to
the inventor of Claude Code are unverifiable, self-described as synthesised
from dozens of sources, and in one case published under a misspelled filename;
no first-party public prompt artifact under that authorship was found, so
nothing is attributed to it. `hesreallyhim/awesome-claude-code` is
CC BY-NC-ND at the surveyed revision, so it is a discovery index and nothing
may be adapted from it. `gastownhall/gascity-packs` carries no repository
LICENSE at the pinned commit, so its stage prompts may be read and cited for
structure and their text may not be copied, while its two vendored subtrees
record MIT provenance in `upstream.toml` and may supply adaptable structures
with attribution. The rule that falls out of all three is the same: extract
structures and checklists, record the licence and the provenance, and never
paste prose whose origin cannot be stated.

**Adopt the thresholds that come with a premade prompt.** Rejected, and this is
the subtler version of the previous rejection. The best available community and
upstream review prompts carry numbers: confidence anchors at 100, 75, 50 and
25; a file-size finding at 1000 lines; adversarial depth bands at 50 and 200
changed lines; a duplication threshold at three matching lines; an
exploitability bar at 80 percent confidence; four-band severity ladders with a
block, request-changes and approve decision matrix. Every one of those is tuned
to a codebase that is not this one, and importing a number silently creates a
threshold this repository never authorised. Worse, the multi-band ladders are
structurally untranslatable: d2b has exactly one blocking channel, `signoff` is
true if and only if `recommendations` is empty, so a four-band verdict has
nowhere to land. What is adopted is the **mechanism**, a stated rule for which
severities may enter `recommendations` at all, and the byte-identical
`## The bar for a finding` block stays the one place that rule is written.

**Commit twelve separate prompt-source files, one per seat.** Rejected. The
seat prompts already exist under `.github/agents/`; a second per-seat file set
doubles the surface that must stay in step and guarantees twelve independent
notions of what a good source is, which is the same drift the byte-identical
finding-bar gate exists to prevent. One collection point with per-role sections
is the artifact a reviewer of the panel design can actually read end to end.

**Add a link-check gate over the source set.** Rejected. The failure it detects
is a moved documentation URL, which is not a reason to block a merge, and the
check itself is network-dependent and would be the flakiest blocking job in the
repository. Retrieval dates, moving-source markers and the `docs` seat cover it
at the right cost.

**Carry a `sources_consulted` list in the record.** Rejected. It is
producer-written, unverifiable, and would be a free-form string vector on the
one artifact this record works hardest to keep free of them. A finding that
rests on a source cites it in the `recommendation` where a human will read it.

**Let the operator add a seat by hand.** Rejected for v1. It would make the
roster no longer a total function of the change surface, and the gate's
recomputation check would have to consult the approval record to distinguish a
legitimate addition from a tampered roster. If the table selects the wrong
seats, the remedy is to change the table, which is a reviewed commit.

**Rename the ten committed seats.** Rejected. `software`, `test`, `product`,
`docs`, `security`, `observability`, `nixos`, `networking`, `rust` and `kernel`
are implemented in a constant, in ten agent files, in a byte-identity check, in
contributor documentation and in five skills, and none of them is imprecise
enough to cause wrong review behaviour. `product` is the weakest fit now that
it carries operator experience, contract fidelity, scope analysis and the Gas
City upstream-claims profile, and it is still not worth the churn. `rust` is
the one committed seat that does not survive this amendment, and what happens
to it is a **removal**, not a rename: D21 deletes the pool member, the enum
variant and `panel-rust.agent.md`, and rewrites its standards into the
`software` prompt's Rust profile. The two renames D21 does make, `simplify` to
`simplicity` and `agentic-coding` to `agentic`, are free because neither seat
exists yet.
