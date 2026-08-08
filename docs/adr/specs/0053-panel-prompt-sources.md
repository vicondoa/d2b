# ADR 0053 panel prompt sources and construction contract

- Kind: decision-support specification for
  [ADR 0053](../0053-gascity-contributor-infrastructure.md).
- Status: current guidance for the accepted ADR 0055 Discover-Fix-Verify
  lifecycle.
- Retrieval date for external sources below: **2026-08-04**.
- Scope: the thirteen-seat selected-roster pool, its profiles and triggers,
  and the prompt construction contract for the standard Copilot panel.
- Non-scope: the operative seat prompts. They live in
  `.github/agents/panel-*.agent.md` and are checked against the selection
  table and prompt corpus.

ADR 0055 is the operative lifecycle decision. This file supplies source
guidance and ownership boundaries; it does not select seats at run time, merge
findings, or define a delivery wire format.

## 0. Source and licensing rules

### 0.1 Normativity

| Marker | Meaning |
| --- | --- |
| **N** | Normative standard, platform specification, repository rule, or vendor documentation about its own behavior. |
| **A** | Advisory engineering practice. It supports an argument but does not make a finding blocking by itself. |
| **M** | Moving source. Cite the retrieval date and prefer a pinned or versioned URL. |

The repository rules in `AGENTS.md`, `docs/contributing/`, ADR 0055, and the
selected lifecycle artifact override external advice. A blocking finding
needs a demonstrable defect in the candidate and a repository rule or named
normative source where one is relevant.

### 0.2 Reuse and provenance

Sources are used for review criteria, not copied prompt prose.

| Tier | Source shape | Permitted use |
| --- | --- | --- |
| T1 | Standards, official references, repository contracts, vendor documentation | Cite a rule and its section when a finding depends on it. |
| T2 | Mature project practice and engineering handbooks | Support an argument; do not make preference a blocker. |
| T3 | Permissively licensed prompt, agent, or skill collections at a pinned commit | Extract general checklist structure and cite the repository and license. Do not copy expressive prose. |
| T4 | Restricted, unattributed, leaked, or unverifiable prompt material | Behavioural evidence only. No copying, adaptation, or prompt reconstruction. |

Pinned sources used by the current prompts include:

- Rust Reference, Rust Style Guide, Rust API Guidelines, Cargo SemVer,
  Rustonomicon, Clippy, and Rust Performance Book, all cited by their official
  documentation URLs.
- POSIX Shell Command Language, ShellCheck rationale, and the Google Shell
  Style Guide for Bash-only practice.
- Nixpkgs `CONTRIBUTING.md`, `pkgs/README.md`, and
  `pkgs/by-name/README.md`, plus RFC 42 for structural configuration.
- Google Engineering Practices for review discipline and the repository's
  own ADR and contributor documents for binding behavior.
- `github/awesome-copilot` at
  `dab758a392cd6b06e806c1aa0444e2bc463b32f9` under MIT, used only for
  general agent structure.
- The vendored MIT portions of the Gas City prompt assets, where explicitly
  identified, used only for general checklist structure.

The repository does not use leaked vendor prompts, unattributed prompt
compilations, or restricted no-derivatives assets as templates. A source whose
license or provenance cannot be stated is dropped.

## 1. ADR 0055 lifecycle contract

The panel runs one comprehensive, complete discovery over the full candidate, not a
series of open-ended rediscovery rounds. The orchestrator:

1. creates one deterministic lifecycle selection artifact;
2. dispatches every selected seat with the full candidate, context, validation
   evidence, and seat focus;
3. collects one explicit complete result from every selected seat, including
   a positive zero-finding result when appropriate;
4. deduplicates source findings into one shared ledger with stable `R`
   identifiers and complete source mappings;
5. gives the full ledger to implementation, which records one supported
   disposition and evidence for every issue;
6. reselects over the full candidate and each fix delta, unioning the roster
   without narrowing it; and
7. runs scoped verification against the ledger, responses, self-verification,
   latest delta, and full candidate context.

Verification checks resolution, dispositions, evidence, regressions, and
unsafe late `BLOCKER` or `MAJOR` findings. A pre-existing late `MINOR` or
`NIT` is retained as history and does not become a new blocking
recommendation. Metrics are informational and never decide approval.

The selected lifecycle roster is the authority for dispatch and verification.
It is not a global headcount and it is not chosen by a reviewer. The current
selection table is version 2, the lifecycle selection schema is version 1,
and the roster is ordered by that table.

## 2. Selected roster and ownership

The current role domain has thirteen seats:

`software`, `test`, `product`, `docs`, `security`, `observability`,
`simplicity`, `reliability`, `agentic`, `nixos`, `networking`, `kernel`, and
`build`.

The selection table makes `software`, `test`, `product`, `docs`, `security`,
`observability`, and `simplicity` mandatory. It triggers optional seats by
changed path or explicit classification signal. Code and configuration
candidates have a floor of ten seats; documentation candidates have a floor
of eight; ambiguity widens to the applicable broader result. Build changes
select the `build` seat; citation-only prose does not. Rust review is a
`software` profile, not a current `rust` seat.

The selection table owns focus text, optional triggers, fill order, floors,
and profiles. Panel prompts must carry the exact focus for their seat. A
selection artifact must carry the candidate digest triple, selection versions,
profiles, and ordered roster. `panel-request` and `make-records.mjs` consume
that same artifact and refuse disagreement.

| Seat | Ownership boundary |
| --- | --- |
| `software` | Control-flow correctness, APIs, error propagation, dependency direction, unsafe and FFI soundness, and active language profiles including Rust. |
| `test` | Coverage, invisible regressions, planted negatives, gate placement, and whether supplied validation proves the change. |
| `product` | Scope, operator experience, naming, migration, CLI and artifact contracts, and actionable errors. |
| `docs` | Documentation placement, changelog and ADR coverage, terminology, links, schema prose drift, process markers, and ASCII dashes. |
| `security` | Exploitability, attacker model, authorization, trust boundaries, secrets, and PII. |
| `observability` | Metrics, spans, logs, audit shape, cardinality, redaction, retention, and diagnostics. |
| `simplicity` | Reuse, deletion, abstraction count, dependency adoption, indirection, and unnecessary machinery. |
| `reliability` | Ownership, cleanup, restart and adoption, idempotency, ordering, concurrency, partial failure, and durable state. |
| `agentic` | Agent profiles, prompt contracts, instruction layering, orchestration, handoffs, and mechanical enforcement. |
| `nixos` | NixOS module and option semantics, priorities, activation ordering, assertions, and evaluated configuration. |
| `networking` | Reachability, firewall, address and port allocation, routing, MTU and MSS, and coexistence. |
| `kernel` | Syscalls, descriptor and lock semantics, signals, mounts, filesystems, races, and kernel-version assumptions. |
| `build` | Build graphs, CI scheduling, toolchains, targets, hermeticity, runfiles, sandboxing, caches, dependencies, packaging, and release artifacts. |

The `build` seat is selected for actual build-system, build-orchestration,
build-contract, dependency, packaging, or release-artifact changes. It checks
the build graph and its boundary with the
changed code, but it does not own
ordinary code correctness, product migration, or generic test coverage.
`software` owns Rust and code-level dependency direction; `test` owns whether
the build validation is sufficient; `agentic` owns prompt and dispatch
mechanics. The `build` seat must not invent a second toolchain, service, broker
surface, or runtime path.

### Build seat source guidance

The `build` seat grounds findings in the repository's
`packages/Cargo.toml`, `Makefile`, `flake.nix`, `tests/layer1-jobs.json`, and
`docs/contributing/gates-and-lints.md`, plus the official Cargo, Nix, and
packaging references relevant to the changed surface. It checks dependency
graphs, target selection, hermetic inputs, cache and runfile assumptions,
parallel scheduling, packaging, and release artifacts. A citation-only
mention of a tool does not activate this seat. The seat reports a concrete
graph or artifact defect and does not propose a new service, daemon, broker,
principal, or runtime control path as a review convenience.

## 3. Prompt contract

Every selected seat receives the full candidate and supplied validation
evidence. Reviewers are read-only and do not rerun tests, builds, evaluations,
or heavy lanes unless the integrator explicitly asks for that action.

Every discovery result is explicit:

```json
{
  "seat": "software",
  "complete": true,
  "findings": []
}
```

An absent selected-seat result is an error. An empty `findings` array is a
complete zero-finding result, not an omitted result.

Each source finding identifies its seat and source ordinal and carries severity,
impact, recommendation, raw source text, and attribution. The orchestrator
maps every source exactly once to one stable ledger issue. A reviewer does not
mint or rename `R` identifiers and does not silently discard an observation.

Each verification prompt carries the complete ledger, every implementation
response, validation and self-review evidence, the latest delta, the full
candidate, prior status, and the seat's verification obligations. Verification
does not reopen discovery merely because a reviewer notices a pre-existing
optional improvement.

The only blocking channel remains the existing panel record contract:
`signoff` is true exactly when `recommendations` is empty. The selected roster
must be unanimous. Reviewers report observations outside their ownership in
their summary rather than laundering them into another seat's blocking
channel.

## 4. Implementation response and verification guidance

The ledger disposition set is unchanged:

- `Fixed`
- `Intentionally rejected`
- `Deferred`
- `Withdrawn`
- `Invalid`

Every issue has exactly one response. `Fixed` requires a changed surface and
non-empty evidence. `Intentionally rejected` and `Deferred` require a concrete
justification. `Withdrawn` and `Invalid` require verified factual status and
supporting evidence.

`BLOCKER` issues approve only as `Fixed`, or as factually verified `Invalid`
or `Withdrawn`. `Intentionally rejected` and `Deferred` never approve a
`BLOCKER`. A `MAJOR` approves as `Fixed`, as factually verified `Invalid` or
`Withdrawn`, or as unresolved `Intentionally rejected` or `Deferred` with
plain recorded acceptance by a repository maintainer or merge owner.
Acceptance is shape-checked process data, not authentication, identity
verification, a signature, a GitHub lookup, or an authority service.

Verification admits only introduced regressions, unsafe late issues, and
previously missed `BLOCKER` or `MAJOR` issues. A newly noticed pre-existing
`MINOR` or `NIT` is recorded as non-blocking history. Scope expansion is
refused and starts a new or explicitly rescoped lifecycle.

The lifecycle records initial findings, late findings, late severity counts,
review and implementation iterations, and average fixed issues per
implementation iteration. A zero implementation-iteration denominator is
`0.0`. No metric is a reviewer score or an approval threshold.

## 5. Stage and seam guidance

The standard Copilot skill owns selection, staging, discovery, ledger
generation, response templates, self-verification, and scoped verification.
`stage-diffs.sh` records the lifecycle id and selected selection path.
`make-records.mjs` writes current schema-version-2 records with
`panel_format_version: 1` for exactly the selected roles. The existing xtask
delivery path validates the request-bound roster and preserves strict legacy
records without rewriting them.

The selection artifact is the seam between lifecycle orchestration and
delivery:

```text
.scratch/panel/<lifecycle>/selections/<candidate-id>/<snapshot-sha256>.json
```

The lifecycle helper writes it deterministically and refuses conflicting
regeneration. Both consumers validate candidate identity, selection schema
version, selection-table version, and ordered roster before writing output.
Lifecycle metadata stays in that artifact and is not copied into the workspace
delivery schema.

Future Gas City producers may consume this selection and artifact contract.
Gas City implementation, a controller, a daemon, a broker, a socket, a
principal, a signature, a GitHub lookup, and a generic migration framework are
outside this standard panel change.

## 6. Local conflict table

| Superseded assumption | Current rule |
| --- | --- |
| A global headcount is the authority. | The versioned selection artifact and request-bound ordered roster are authoritative. |
| A reviewer chooses whether it is relevant. | The deterministic selection table chooses seats; a selected seat supplies a complete result. |
| Withdrawn: a four-field verdict carries `relevant`, `signoff`, `recommendations`, and `prior_resolutions`. | Current panel delivery carries the existing strict sign-off record; lifecycle discovery, ledger, responses, and verification artifacts carry their own fields. |
| Withdrawn: a held reviewer can be released or removed by a later verdict. | The lifecycle roster only widens and every selected seat remains obligated. |
| Every round repeats full discovery. | Discovery runs once; later work is batch fixing and scoped verification. |
| Reviewers rerun validation. | The integrator supplies validation evidence; reviewers inspect it unless explicitly asked to validate. |
| Rust requires a current standalone seat. | Rust depth belongs to the `software` profile; legacy `rust` records remain readable. |
| Adding a build seat is enough. | The build source guidance, ownership boundary, trigger, and current agent must agree. |

The old assumptions above are process history only. They are not operative
requirements and must not be copied into a new prompt or artifact.

## 7. Caveats

- External documentation moves. The retrieval date and pinned source rule are
  intentional; this repository does not add a network-dependent link gate.
- Community prompt assets are examples of structure, not authority. Local
  contracts and demonstrable candidate behavior win.
- The panel does not prove maintainer identity. Recorded acceptance remains
  ordinary repository process data.
- Metrics describe the lifecycle but never determine sign-off.
- Prompt compression, when selected, applies only to transient communication
  and the governed corpus rules. Persisted schemas, commands, paths,
  identifiers, negations, and panel JSON remain exact.

## 8. Change discipline

Changing role ownership, selection triggers, floors, profiles, the discovery or
verification contract, or the blocking bar changes panel behavior. Land the
corresponding selection table, agent prompts, binding checks, lifecycle tests,
contributor guidance, and prompt-corpus capture together. Do not create a
second selection source or a second verdict format.

The following legacy contract names are explicitly withdrawn: the withdrawn
fixed roster, the withdrawn fixed ten-seat roster, the withdrawn `relevant:`,
the withdrawn `prior_resolutions`, the withdrawn held-reviewer, the withdrawn
held-seat, the withdrawn repeated rounds, and the withdrawn old verification.
They describe superseded prompt behavior and are not instructions for the
current lifecycle.

## 9. Maintainer use

When a seat, lifecycle script, or delivery reader changes, maintainers compare
the selection table, lifecycle artifacts, prompt guidance, and bounded
delivery readers as one contract. The current panel remains contributor
tooling: it consumes repository evidence, writes ordinary process artifacts,
and does not become an authority service or a runtime dependency. A future
orchestrator may reuse these shapes only if it preserves deterministic
selection, complete discovery, ledger coverage, scoped verification, and the
same request-bound sign-off rule.
