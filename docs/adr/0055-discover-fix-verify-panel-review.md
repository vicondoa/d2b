# ADR 0055: Discover, fix, and verify panel review

- Status: Accepted
- Date: 2026-08-07
- Amended: 2026-08-07
- Partially supersedes: [ADR 0053](0053-gascity-contributor-infrastructure.md)
  D7 through D9 for panel-review trust and lifecycle, and D21 for panel
  selection. ADR 0053's classification of Gas City as contributor
  infrastructure and its unrelated orchestration and publication decisions
  remain.
- Related: [ADR 0048](0048-copilot-native-agent-surface.md), whose read-only
  Copilot reviewers, pinned model and effort, staged evidence, and record
  helper remain the implementation base; and
  [ADR 0052](0052-bazel-rust-build-and-test.md), whose build decision is not
  changed here.
- Scope: contributor panel selection, review lifecycle, generated review
  artifacts, compatibility, and improvement metrics.
- Non-scope: Gas City implementation, product behavior, and runtime or
  control-plane changes.

## Context

The current panel repeats open-ended review after each fix. That can reveal
real defects, but it also lets later rounds rediscover pre-existing MINOR or
NIT issues indefinitely. Reviewers receive separate findings, so contributors
must manually reconcile duplicates and prior responses.

At this amendment, committed code still implements a fixed ten-seat roster,
repeated rounds, immutable staged diffs, and
`signoff == recommendations.is_empty()`. That passing code is canon and stays
current until the replacement skill, scripts, prompts, compatibility adapter,
records, and checks land together. Implementation has not begun.

Panel review is an LLM-assisted engineering quality gate. It is not a
security boundary, an authorization system, or proof of perfect review.
Verdicts and ledgers are imperfect and bypassable process artifacts. They
need to be consistent and auditable enough for contributor workflow, not
protected from a repository maintainer.

## Amendment and withdrawal

This amendment replaces ADR 0055's original decision text. It explicitly
supersedes and withdraws the following proposed machinery:

- protected-authority and privileged-principal review models;
- a Unix authority service or any peer-authenticated authority socket;
- pidfd, cgroup, namespace, or other process-management machinery for review;
- a root-owned audit sink or privileged audit plane;
- capability tokens and assignment capabilities;
- cryptographic receipts, authority receipts, and receipt resolvers used as
  authorization;
- separate risk-acceptance authorization; and
- the detailed replay, recovery, migration, fencing, reserve, retention, and
  crash-state machinery built around those mechanisms.

These are not implementation requirements and must not be inferred from the
remaining process contract. Any ADR 0053 cross-reference that treated them as
an extension of its controller is withdrawn for panel review.

The implementation adds no daemon, root service, broker, socket, operating
system principal, cgroup, pidfd, namespace, credential broker, root audit
service, or product/runtime surface. Existing read-only Copilot panel agents,
model and effort binding, staged immutable diffs, ordinary repository
controls, and the current record helper are sufficient.

## Decision

### Discover, fix, verify

One panel lifecycle has exactly one comprehensive discovery panel, followed by
implementation and scoped verification. Its discovery-selected roster stays
fixed through verification.

#### Discovery

Every selected reviewer receives the full candidate, full relevant context,
validation evidence available at discovery time, and its seat guidance. Every
reviewer inspects the whole candidate and reports all reasonably discoverable
actionable findings without stopping after the first issue or saving
observations for a later pass.

Each finding has one of four severities:

- `BLOCKER`: unsafe to merge under any ordinary disposition;
- `MAJOR`: material correctness, security, data-loss, reliability, contract,
  or operability risk;
- `MINOR`: worthwhile but non-critical defect; or
- `NIT`: small actionable quality issue.

Each finding includes its impact and a concrete recommendation. Reviewer seat
and source ordinal are recorded.

The orchestrator automatically merges and deduplicates all selected-seat
outputs into one shared ledger. It assigns stable lifecycle-local identifiers
`R1`, `R2`, and so on in deterministic order. Every source finding maps to
exactly one ledger item; duplicate source findings may map many-to-one.

The helper validates:

- every selected seat supplied a complete discovery result;
- every source finding is represented;
- ledger identifiers are unique and stable;
- source-to-ledger mapping is complete; and
- identical inputs render identical output.

The helper is a consistency tool, not protected authority.

#### Fix

Implementation receives the whole ledger at once. Every item receives one
recorded disposition:

- `Fixed`, with the change and evidence;
- `Intentionally rejected`, with a concrete justification;
- `Deferred`, with a concrete justification; or
- `Withdrawn` or `Invalid`, when a finding is factually wrong or no longer
  applies.

`Withdrawn` and `Invalid` are simple factual dispositions, not an
authorization protocol. A `BLOCKER` must be fixed or verified as factually
invalid or withdrawn. A `MAJOR` must be fixed or plainly accepted by the
repository maintainer or merge owner in the recorded response.

Before verification, implementation records the applicable tests, lint,
formatting, static analysis, build results, uncovered areas, and a self-review.
MINOR and NIT findings remain in the ledger, but after disposition and review
they do not block approval.

#### Verification

Every selected reviewer receives:

- the complete ledger and source attribution;
- every implementation response and disposition;
- the recorded validation and self-review evidence;
- the latest delta and full candidate context; and
- that seat's obligations and previous status from the last complete
  discovery or verification artifact.

Verification checks prior issues, rejected and deferred items, introduced
regressions, and whether the evidence supports the responses. It is not a
fresh discovery pass.

A reviewer may add a new verification issue only for:

- an introduced regression;
- a previously missed `BLOCKER` or `MAJOR`; or
- a correctness, security, data-loss, or reliability issue that makes
  approval unsafe.

Verification does not admit late pre-existing MINOR or NIT findings, style
preferences, optional refactors, naming taste, untouched-code nits, or
theoretical out-of-scope edge cases.

Approval means merge-ready:

1. every `BLOCKER` is resolved;
2. every `MAJOR` is resolved or plainly accepted by the repository maintainer
   or merge owner in a recorded response;
3. required validation passes;
4. no introduced regression remains;
5. verification has no new `BLOCKER` or `MAJOR`; and
6. every selected reviewer signs off.

The existing verdict invariant remains:
`signoff == recommendations.is_empty()`. Verification recommendations contain
only merge-blocking conditions. Non-blocking MINOR and NIT history remains in
the ledger rather than keeping `recommendations` non-empty.

### Generated artifacts and compatibility

Scripts automatically generate the discovery request, merged ledger,
implementation-response template, and per-seat verification artifacts.
Operators do not hand-copy findings or responses.

Each per-seat verification artifact carries the complete ledger and, for every
item, its stable id, description, source attribution, disposition, previous
status, evidence, and seat obligations from the last complete round or ledger.
Artifacts may carry a version tag. The implementation prefers simple JSON and
Markdown under `.scratch/panel/<lifecycle>/` and the existing delivery state.

Generation is idempotent: the same inputs produce the same bytes. Regeneration
against conflicting existing bytes fails loudly instead of overwriting them.
History remains readable and auditable and is never rewritten.

Compatibility is a file transform, not a service:

- an already dispatched legacy round may finish;
- the adapter imports the latest complete legacy round and preserves fixes
  already underway;
- legacy source identity is deterministic from record, seat, and source
  ordinal, after which the orchestrator assigns `R` identifiers;
- a partial legacy round may retry missing seats or restart as the lifecycle's
  one new discovery panel without dropping findings from completed seats; and
- no protected migration or recovery service is introduced.

A completed legacy round can serve as the discovery input. A restarted
partial round is one logical discovery panel even when individual seat
attempts are retried.

### Improvement metrics

The lifecycle records:

- initial unique findings;
- late unique findings;
- late `BLOCKER` findings;
- late `MAJOR` findings;
- review iterations;
- implementation iterations; and
- average issues fixed per implementation iteration.

The average is the number of unique ledger items ending `Fixed` divided by
implementation iterations. A zero denominator produces `0.0`.

These values are improvement signals for prompts and workflow. They are never
approval thresholds, reviewer scores, or merge conditions.

### Panel selection

ADR 0053 D21's core selection concepts remain: one deterministic selector,
mandatory seats, surface-dependent floors, every triggered optional seat,
wider selection on ambiguity, profile binding, pinned reviewer identity, and
candidate-bound evidence. The pool grows to thirteen with optional `build`.
There is no separate `rust` seat; Rust depth is part of `software`.

| Class | Seats |
| --- | --- |
| Mandatory on every panel | `software`, `test`, `product`, `docs`, `security`, `observability`, `simplicity` |
| Optional by trigger or floor fill | `reliability`, `agentic`, `nixos`, `networking`, `kernel`, `build` |

Code and operative-configuration candidates have a floor of ten seats.
Documentation-only candidates have a floor of eight. Every optional trigger
that fires adds its seat even when the floor is already met. Ambiguous
classification or matching selects the wider result. Floor fill order is
`reliability`, `agentic`, `nixos`, `networking`, `kernel`, then `build`.

The following table is the concise normative guidance. Mandatory focus applies
on every panel. An optional seat is selected when any trigger in its row
matches.

| Seat | Focus | Trigger |
| --- | --- | --- |
| `software` | Correctness, control flow, error propagation, APIs, unsafe and FFI boundaries, language conventions, dependency direction, and testability. | Always; bind every applicable language profile. |
| `test` | Coverage of behavior and failure paths, invisible regressions, planted negatives, gate placement, and whether cited validation proves the change. | Always. |
| `product` | Scope, operator experience, CLI and exit codes, external contracts, migration, defaults, naming surface, and actionable errors. | Always. |
| `docs` | Diataxis placement, changelog and ADR index coverage, prose/schema drift, terminology, links, process-marker rules, and ASCII-only dashes. | Always. |
| `security` | Concrete attack surface, authorization and capability boundaries, privilege separation, sandboxing, secrets, PII, and audit exposure. | Always. |
| `observability` | Metric cardinality, spans, logs, audit shape, redaction, retention, exporters, and diagnosability. | Always. |
| `simplicity` | Small maintainable design, reuse, deletion, and avoidance of duplicated contracts, dependency sprawl, and unnecessary machinery. | Always. |
| `reliability` | Resource ownership, cleanup, restart and adoption, idempotency, ordering, concurrency, partial failure, and durable state. | Stateful lifecycle, process, storage, synchronization, cleanup, restart, migration, or concurrency surfaces. |
| `agentic` | Agents, prompts, skills, instructions, context construction, orchestration, handoffs, and mechanical enforcement of prompt claims. | Agent, prompt, skill, Copilot, Gas City, or contributor-orchestration surfaces. |
| `nixos` | NixOS options, module merging, `mkDefault` and `mkForce`, assertions, evaluation, activation ordering, and unit invariants. | Nix, NixOS module, flake, package-expression, template, or Nix-generated surfaces. |
| `networking` | Bridges, firewalls, DHCP, DNS, routes, MTU and MSS, sockets, isolation, and host-network coexistence. | Networking paths or operative network, firewall, routing, resolver, socket, or interface changes. |
| `kernel` | Syscalls, pidfd, cgroup v2, namespaces, mounts, signals, ioctl, filesystems, errno, and kernel-version assumptions. | Kernel-facing paths or operative syscall, process, mount, signal, cgroup, namespace, or filesystem changes. |
| `build` | Build graphs and orchestration, CI scheduling, toolchains, targets, hermeticity, runfiles, sandboxing, caches, dependencies, packaging, and release artifacts. | `BUILD`, `BUILD.bazel`, `MODULE.bazel*`, `WORKSPACE*`, `.bzl`, `.bazelrc`, Make or build orchestration, CI build/test/package/publish, toolchains, targets, cross compilation, Cargo/Bazel/Nix build integration, runfiles, build sandbox/cache/remote execution, dependency locks or hubs, packaging or release artifacts, or normative build contracts. |

Citation-only prose does not trigger `build`. A changed normative build
contract does.

The cutover table is version 2. One versioned machine-readable selection table at
`.github/skills/d2b-panel-round/selection-table.json` is authoritative for the
pool, classes, floors, fill order, profiles, focus, and triggers. The standard
panel skill derives the roster from it and either generates or byte-checks the
human guidance. Agent prompts do not select their own relevance.

The standard Copilot skill is implemented first. A future Gas City producer
must consume the same table and artifact formats and must produce the same
selection for the same inputs. Gas City implementation remains out of scope.

### Prompt contract

The discovery prompt says plainly:

> This first review is comprehensive. Spend the effort now, report every
> reasonably discoverable actionable finding, and do not save observations
> for later rounds.

The verification prompt says plainly:

> Verify prior findings, responses, evidence, and regressions. Do not reopen
> the whole review unless a new or previously missed critical issue makes
> approval unsafe.

Seat-specific guidance may add focus. It may not weaken these instructions.

### Implementation and cutover

Implementation modifies the existing standard panel skill, scripts, panel
agent prompts, and existing delivery record and helper surfaces only as
needed. Existing `xtask` code or repository scripts are preferred where code
is needed. No new Rust crate or service exists merely for architectural
purity.

Cutover is atomic. The current ten-seat repeated-round process remains current
until selection, generation, prompts, records, checks, and the compatibility
adapter land together. A half-converted workflow is not supported.

Behavior tests cover:

- selection-table parsing, floors, every trigger, ambiguity, and fill order;
- build triggers and citation-only negatives;
- complete artifact generation and conflicting regeneration;
- deterministic `R` identifiers and duplicate source mapping;
- complete legacy import and partial-round retry or restart;
- every disposition and required justification;
- late-issue admission and refusal;
- metric calculations, including zero denominators;
- discovery and verification prompt scope; and
- final sign-off and merge-readiness conditions.

A future Gas City parity fixture may be defined, but Gas City code is
deferred. No security theatre or exhaustive kernel/process state machine is
part of this implementation.

### Concrete failures and guards

| Failure | Guard |
| --- | --- |
| A source finding disappears during deduplication. | Completeness and source-mapping validation refuses the ledger. |
| Two generations for the same inputs disagree. | Byte-stable rendering and conflict refusal stop dispatch. |
| A late style nit restarts discovery. | Verification admission rejects the issue class. |
| Human guidance and roster selection drift. | The authoritative table generates or byte-checks guidance and drives selection. |
| A partial legacy round loses completed work. | Import preserves completed-seat findings before retry or restart. |

## Non-goals

- perfect or tamper-proof review;
- cryptographic trust or independent review authorization;
- preventing a maintainer from bypassing contributor process;
- product daemon or runtime integration;
- a privileged authority or root audit plane; and
- exhaustive crash handling for temporary panel artifacts.

## Consequences

The workflow becomes smaller, faster, and maintainable. Contributors fix one
complete ledger instead of chasing an open-ended sequence of reviews, while
scoped verification still checks the fixes and catches unsafe regressions.
Automatic artifacts remove hand-copying and keep legacy work usable.

The design relies on maintainer discipline and ordinary repository controls.
LLM reviewers will miss issues, merge owners can bypass the process, and the
ledger is not tamper-proof. Metrics expose where prompts and review practice
need improvement without pretending a threshold proves quality.

## Alternatives considered

### Keep repeated open-ended rounds

Rejected because each round can rediscover pre-existing low-severity issues
and prevent convergence.

### Run one panel with no verification

Rejected because fixes can be incomplete or introduce regressions.

### Copy findings and responses manually

Rejected because hand-copying drops attribution, duplicates work, and makes
verification inputs inconsistent.

### Keep the withdrawn authority architecture

Rejected as disproportionate security theatre for a bypassable contributor
quality gate. It adds privileged services, recovery states, and operational
burden without making LLM review complete or preventing maintainer bypass.
