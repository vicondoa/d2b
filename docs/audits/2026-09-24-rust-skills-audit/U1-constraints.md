# U1 - Rust skills audit constraint packet (priming feed)

Baseline: branch `v3` @ `6ebdd4cec` (HEAD at audit start; working tree clean except
one pre-existing untracked plan file, not ours)  -  Date: 2026-09-24  -  Lens revision:
`third_party/agent-skills/rewrite-rs/v0.1.0-alpha.1/skills/` (vendored at the same
HEAD)  -  Plan: `local://rust-skills-audit-plan.md` (durable copy; this packet
implements its Step 1).

Deliverable: `docs/audits/2026-09-24-rust-skills-audit/` - `README.md`
(consolidated report), `U1-constraints.md` (this packet), `lane/<id>.md` (one file
per lane), `VERIFICATION.md` (Step 5). Nothing else is written; the audit is
READ-ONLY on source, policy, and gates. No `cargo build`/`test`/`clippy`,
formatter, or project-wide command is run by any lane.

Every lane: read this file fully, then the assigned lens cards (section c), then
run the lane protocol (section b) over the lane's scope from the published
part-partition map (section f). Write only your lane file.

## (a) Lane map

Lane ids, crates, planning-time LOC (basis: `src/**` excluding `src/generated/**`
plus `tests/**`, `wc -l`), and part counts:

|lane id|crate|LOC|parts|
|---|---|---:|---:|
|`d2bd-p1`..`-p8`|`d2bd`|93,768|8|
|`d2b-broker-p1`..`-p7`|`d2b-broker`|79,531|7|
|`xtask-p1`..`-p5`|`xtask`|49,329|5|
|`d2bd-runtime-p1`..`-p4`|`d2bd-runtime`|43,123|4|
|`d2b-p1`..`-p3`|`d2b`|26,481|3|
|`d2b-bus-p1`..`-p2`|`d2b-bus`|20,894|2|
|`d2b-contracts-resource-p1`..`-p2`|`d2b-contracts-resource`|19,571|2|
|`d2b-core-p1`..`-p2`|`d2b-core`|17,644|2|
|`d2b-provider-toolkit-p1`..`-p2`|`d2b-provider-toolkit`|17,414|2|
|`d2b-provider-display-wayland-p1`..`-p2`|`d2b-provider-display-wayland`|16,248|2|
|`d2b-resource-runtime-p1`..`-p2`|`d2b-resource-runtime`|15,515|2|
|`d2b-contracts-provider-p1`..`-p2`|`d2b-contracts-provider`|14,706|2|
|`d2b-session-p1`..`-p2`|`d2b-session`|14,538|2|
|`d2b-resource-api-p1`..`-p2`|`d2b-resource-api`|13,931|2|
|`d2b-core-controller-p1`..`-p2`|`d2b-core-controller`|13,919|2|
|`d2b-provider-clipboard-wayland-p1`..`-p2`|`d2b-provider-clipboard-wayland`|13,710|2|
|`d2b-contracts-zone-session-p1`..`-p2`|`d2b-contracts-zone-session`|12,839|2|
|`d2b-host`|`d2b-host`|11,767|1|
|`d2b-provider-network-local`|`d2b-provider-network-local`|11,256|1|
|`d2b-contracts`|`d2b-contracts`|10,743|1|
|`d2b-provider-process`|`d2b-provider-process`|10,732|1|
|`d2b-provider-guest-cloud-hypervisor`|`d2b-provider-guest-cloud-hypervisor`|10,523|1|
|`d2b-provider-volume-local`|`d2b-provider-volume-local`|10,405|1|
|`d2b-zone-routing`|`d2b-zone-routing`|10,370|1|
|`d2b-resource-compiler`|`d2b-resource-compiler`|7,461|1|
|`d2b-provider-device-usbip`|`d2b-provider-device-usbip`|7,015|1|
|`d2b-provider-credential-secret-service`|`d2b-provider-credential-secret-service`|6,699|1|
|`d2b-session-unix`|`d2b-session-unix`|6,663|1|
|`d2b-provider-guest`|`d2b-provider-guest`|6,436|1|
|`d2b-provider-supervisor`|`d2b-provider-supervisor`|6,162|1|
|`d2b-provider-transport-azure-relay`|`d2b-provider-transport-azure-relay`|5,952|1|
|`d2b-provider-notification-desktop`|`d2b-provider-notification-desktop`|5,712|1|
|`d2b-provider-credential-managed-identity`|`d2b-provider-credential-managed-identity`|5,453|1|
|`d2b-provider-credential-entra`|`d2b-provider-credential-entra`|5,246|1|
|`d2b-audit`|`d2b-audit`|5,221|1|
|`d2b-contracts-broker`|`d2b-contracts-broker`|5,215|1|
|`d2b-contracts-control`|`d2b-contracts-control`|5,019|1|
|`d2b-resource-client`|`d2b-resource-client`|4,839|1|
|`d2b-provider-device-security-key`|`d2b-provider-device-security-key`|4,320|1|
|`d2b-process-conformance`|`d2b-process-conformance`|4,195|1|
|`d2b-provider-shell-terminal`|`d2b-provider-shell-terminal`|4,181|1|
|`d2b-provider-transport-vsock`|`d2b-provider-transport-vsock`|4,146|1|
|`d2b-provider-activation-nixos`|`d2b-provider-activation-nixos`|4,014|1|
|`d2b-provider-wayland-policy`|`d2b-provider-wayland-policy`|3,982|1|
|`d2b-provider-process-systemd`|`d2b-provider-process-systemd`|3,816|1|
|`d2b-provider-zone-link`|`d2b-provider-zone-link`|3,814|1|
|`d2b-provider-observability-otel`|`d2b-provider-observability-otel`|3,809|1|
|`d2b-provider-device-gpu`|`d2b-provider-device-gpu`|3,758|1|
|`d2b-provider-guest-qemu-media`|`d2b-provider-guest-qemu-media`|3,694|1|
|`d2b-provider-device-tpm`|`d2b-provider-device-tpm`|3,338|1|
|`d2b-provider-credential`|`d2b-provider-credential`|3,337|1|
|`d2b-provider`|`d2b-provider`|3,252|1|
|`d2b-unsafe-local-helper`|`d2b-unsafe-local-helper`|3,240|1|
|`d2b-provider-volume-binding`|`d2b-provider-volume-binding`|2,914|1|
|`d2b-provider-guest-azure-virtual-machine`|`d2b-provider-guest-azure-virtual-machine`|2,839|1|
|`d2b-provider-audio-pipewire`|`d2b-provider-audio-pipewire`|2,709|1|
|`d2b-provider-endpoint`|`d2b-provider-endpoint`|2,548|1|
|`d2b-provider-guest-azure-container-apps`|`d2b-provider-guest-azure-container-apps`|2,420|1|
|`d2b-provider-provider`|`d2b-provider-provider`|2,289|1|
|`d2b-provider-volume`|`d2b-provider-volume`|2,150|1|
|`d2b-provider-host`|`d2b-provider-host`|2,108|1|
|`d2b-provider-user`|`d2b-provider-user`|2,099|1|
|`d2b-provider-volume-virtiofs`|`d2b-provider-volume-virtiofs`|2,025|1|
|`d2b-telemetry`|`d2b-telemetry`|1,983|1|
|`d2b-provider-config-nixos`|`d2b-provider-config-nixos`|1,816|1|
|`d2b-provider-system-core`|`d2b-provider-system-core`|1,810|1|
|`d2b-broker-composition`|`d2b-broker-composition`|1,634|1|
|`tail-1`|`d2b-broker-fixture-handlers`, `d2b-broker-fixture-syscall-surface`, `d2b-controller-toolkit`, `d2b-host-activation-helper`, `d2b-provider-audio-binding`|1,121|1|
|`tail-2`|`d2b-provider-audio-service`, `d2b-provider-command`, `d2b-provider-device`, `d2b-provider-emergency-policy`, `d2b-provider-operation`|2,571|1|
|`tail-3`|`d2b-provider-process-minijail`, `d2b-provider-quota`, `d2b-provider-resource-export`, `d2b-provider-resource-import`, `d2b-provider-role`|1,759|1|
|`tail-4`|`d2b-provider-role-binding`, `d2b-provider-seccomp-profile`, `d2b-provider-shell-pool`, `d2b-provider-shell-session`, `d2b-provider-telemetry-binding`|2,549|1|
|`tail-5`|`d2b-provider-telemetry-service`, `d2b-provider-test-controller`, `d2b-provider-transport-unix`, `d2b-provider-wayland-session`, `d2b-provider-zone`|3,144|1|
|`tail-6`|`d2b-resource-types`, `d2b-sk-frontend`|2,173|1|

Cross-cutting lanes (not in the table): `lane/X1-supply-chain.md`,
`lane/X2-generated-boundary.md`, `lane/X3-cross-crate-duplication.md`.

Part-partition rule (mechanical, already applied in section f; no
renegotiation): for a crate with k>1 parts, its top-level units under `src/`
(files `src/*.rs`, directories `src/*/`, excluding `generated`) were sorted by
LOC descending and greedily packed (least-loaded-bin) into k groups each
`<= ceil(LOC/k) x 1.2`, keeping units whole; units exceeding the cap were
descended (directories) or split by item ranges (files, boundaries recorded in
section f). Section f publishes the concrete part -> module map; each lane stays
inside its assigned files/ranges.

## (b) Global rules

### Lane protocol (run in this order)

1. Read `U1-constraints.md` fully, then the assigned lens cards.
2. Read each assigned lens's SKILL.md (`skill://<skill-name>`; if that URI does
   not resolve, the vendored path in the lens table below). A SKILL.md may point
   to sibling reference files in the same directory (`ERROR-TYPES.md`,
   `NAMING.md`, `BOILERPLATE.md`, `FLOWS.md`, `CLONE-DECISIONS.md`,
   `SHARED-STATE.md`, `NUMERICS.md`, `TYPESTATE.md`, `SURFACE.md`,
   `DEPENDENCY-INJECTION.md`, `SEMVER.md`, `ALLOCATION.md`, `CANCELLATION.md`,
   `SAFETY-REVIEW.md`, `TEST-DESIGN.md`, `DIFFERENTIAL-TESTING.md`, `DENY.md`) -
   read on demand when the SKILL.md defers to them.
3. Enumerate the assigned files (all of `src/**` except `src/generated/**`;
   plus `tests/**` for the `test` lens; plus `build.rs` if present). If the lane
   is a part (`-p<k>`), stay inside the assigned module partition: for file-split
   parts restrict seed runs with `sed -n '<A>,<B>p' <file>` (or
   `awk 'NR>=A && NR<=B'`) and add `<A-1>` to reported line numbers so anchors
   stay absolute.
4. Per lens, in the card order: run the seeds, read every hit with enough
   context to judge (whole file for files <400 lines; otherwise the hit
   neighborhood, +/-40 lines, plus the file's item list). For `api`/`type`,
   additionally read the crate's public surface: `lib.rs`/`mod.rs` re-exports
   and all `pub` items.
5. Emit findings into the lane file as you go. Reading budget rule: never dump a
   whole large file into the lane; cite `path:line`.
6. Sampling rule: if one lens's seeds exceed 200 hits in the lane, read 50 hits
   sampled deterministically (every ceil(n/50)-th hit) and record
   `sampled: 50 of <n> hits` in the lens section; never claim exhaustiveness for
   that lens.
7. Do not run `cargo build`/`test`/`clippy`, formatters, or any project-wide
   command. Read-only audit. Write only your lane file.
8. Caller census rule (mandatory) for any finding that claims something is
   unused, redundant, unreachable, or reducible in visibility: search the symbol
   across `packages/`, `nixos-modules/`, `tests/`, `docs/reference/`, `labs/`,
   and `BUILD.bazel`/`*.bzl` files; record
   `census: <pattern> over <roots> = <N> hits`.

### Lane file format (verbatim contract)

```
# <lane-id> - <crate> [- part <k>/<n>]
Baseline: <HEAD OID read at lane start> | LOC audited: <n> (excl. src/generated/**) | modules: <list or "whole crate">
Lenses: <lens list for this lane> | Partitions: <module partition for parts>

## <lens>
- <lane-id>#<k> sev=<high|medium|low> blast=<leaf|family|wide> effort=<S|M|L> verdict=<actionable|policy-confirmed|needs-contract> - <what is wrong, one line, concrete> - fix: <concrete change naming symbols/targets> - [path:line, path:line]
  evidence: <seed regex + count; census result; or "static (unmeasured)" for perf>
- clean: <lens's seeds run + hit counts; one sentence naming what was checked>

## Coverage
<one line per assigned lens>: <lens>=<n> | clean | N/A: <seed evidence>
```

Schema notes (grammar the verifier keys on):

- A finding is exactly two lines: the `- <lane-id>#<k> sev=... - ... - fix: ...
  - [path:line, ...]` line, then one `  evidence: ...` line.
- Local finding ids: `<lane-id>#<k>`, k increments from 1 per lane.
- Coverage lines use one of: `- <lens>: <n> finding(s)` |
  `- <lens>: clean (seeds ran: <c1>/<c2>/...)` |
  `- <lens>: N/A (seeds: <c1>/<c2>/... all zero; <criteria note>)`.
- Tail lanes carry a `## <crate>` section per crate, each with the standard lens
  sections and its own `### Coverage` block; the lane header lists all crates.
- `LOC audited` is measured by the lane (`wc -l` over its assigned files,
  excluding `src/generated/**`).
- Consolidation assigns global `RS-####` ids; local ids stay lane-local.

### Severity rubric (fixed for all lanes; judge against these definitions)

- `high` - correctness, soundness, security, or measurable operational cost:
  unsound/incorrect code; a panic reachable from untrusted or caller input; a
  secret/PII path into logs/errors/metrics; a blocking call on an executor
  worker; unbounded allocation/scan in a hot path; a failure silently swallowed
  where the caller must know; a test that cannot fail.
- `medium` - API/model/maintainability with real cost: public surface exposing
  internals or leaking types; illegal states representable that the skill names
  an alternative for; an error taxonomy forcing callers to string-match; missing
  validation on wire input where a sibling type has it; hand-rolled duplicate of
  an existing in-tree helper (name the canonical home); missing doc contract on
  a non-obvious public item; contract behavior with no test.
- `low` - expression, consistency, naming, polish: iterator-vs-index loop;
  explainable-but-avoidable clone; naming drift; doc first-sentence shape.

### Verdict values

- `actionable` (implementable now), `policy-confirmed` (conflicts with a
  recorded deliberate decision - cite the policy file:line; requires a
  policy/ADR change first), `needs-contract` (touches wire formats, error codes,
  manifest schema, golden fixtures, or generated shapes).

### Blast radius and effort

- Blast radius: `leaf` (one crate), `family` (crates in one family - provider
  family, contracts family, etc.), `wide` (wire/contracts/daemon/broker/
  cross-cutting).
- Effort: `S` (one file or mechanical), `M` (a handful of files in one crate),
  `L` (multi-crate or design change).

### Evidence rules

- Every finding's `evidence:` line names the seed regex and its count, the
  census result, or `static (unmeasured)` for perf; a finding without evidence
  is a schema violation.
- An `api` claim that a change breaks callers, and an ownership claim that a
  clone is required, both cite the call site rather than assert it.
- Findings that propose an improvement already named by a gate/policy in
  section (d) are `policy-confirmed` with the policy file:line cited.
- Perf findings are static until a benchmark exists; never claim a measured
  win.

### Read-only rules

- No source edits, no gate runs, no formatter, no `git` state changes. The only
  write is the lane's own `lane/<id>.md` file.
- Do not fix anything found; correctness or security defects are kept as
  `sev=high verdict=actionable` with route `review-pass` noted in the row text.
- The pre-existing untracked file `docs/plans/2026-09-22-001-chore-post-plan-cleanup-wave-plan.md`
  is not ours; leave it untouched.

## (c) Lens cards

Lens table (all 16 audit lenses; paths relative to
`third_party/agent-skills/rewrite-rs/v0.1.0-alpha.1/`):

|lens|skill name|vendored SKILL.md|
|---|---|---|
|`idiom`|idiomatic-rust|`skills/rust/idiomatic-rust/SKILL.md`|
|`own`|ownership-not-clone|`skills/rust/ownership-not-clone/SKILL.md`|
|`type`|type-driven-design|`skills/rust/type-driven-design/SKILL.md`|
|`api`|rust-api-design|`skills/rust/rust-api-design/SKILL.md`|
|`err`|rust-errors|`skills/rust/rust-errors/SKILL.md`|
|`serde`|rust-serde|`skills/misc/rust-serde/SKILL.md`|
|`obs`|rust-observability|`skills/rust/rust-observability/SKILL.md`|
|`docs`|rust-docs|`skills/rust/rust-docs/SKILL.md`|
|`perf`|rust-performance|`skills/rust/rust-performance/SKILL.md`|
|`conc`|rust-concurrency|`skills/rust/rust-concurrency/SKILL.md`|
|`async`|async-rust|`skills/rust/async-rust/SKILL.md`|
|`unsafe`|unsafe-rust|`skills/rust/unsafe-rust/SKILL.md`|
|`ffi`|rust-ffi|`skills/misc/rust-ffi/SKILL.md`|
|`macro`|rust-macros|`skills/misc/rust-macros/SKILL.md`|
|`test`|rust-testing|`skills/workflow/rust-testing/SKILL.md`|
|`supply`|rust-supply-chain|`skills/misc/rust-supply-chain/SKILL.md`|

Seed construction note: every card's seed set begins with the patterns named in
that skill's own `## Verification` section. Where the skill names only cargo
subcommands (`cargo clippy`, `cargo doc`, `cargo miri`, `cargo bench`, ...), the
card carries a `verify:` line with those commands and the seed regexes are
proxies for the class those commands report. Every seed below is a single regex
that ran over this repository at `6ebdd4cec`; the per-crate hit counts are in
section (e).

### idiom (idiomatic-rust)

Judges: expression shape - iterator pipelines over index loops, `From`/`TryFrom`
over ad-hoc converters, derives over hand-written impls, newtypes over bare
primitives, naming discipline (`as_`/`to_`/`into_`, no `get_`, acronyms, free
functions). This is the lens for "reads like Rust" findings; defer ownership to
`own`, invariants to `type`.
Seeds (run each; record hit count per crate):
  1. `for \w+ in 0\.\.`            # index loop where an iterator pipeline is expected
  2. `impl (Default|From|PartialEq|Eq|Debug|Clone|Hash) for`  # hand-written impl a derive may replace
  3. `let mut \w+ = (String|Vec)::new\(\)`  # statement-style accumulation
verify: `cargo clippy --all-targets`, `cargo fmt --check`, `cargo test` (run by
remediation, not by lanes).
Applicable when: the crate declares `fn` bodies. N/A only when all three seeds
are 0 and the crate declares no `fn`.
Repo false positives: hand-written `Debug` impls that redact secrets (deliberate;
`redacted_debug!` exists and a derive would leak); hand-written `PartialEq` on
wire types with deliberate field exclusions; `Default` impls that preserve an
invariant (forbidden by a field-wise derive); `to_string()` on a type whose
`Display` is the wire rendering; `#[derive]` already present is not a finding.
Gate interaction: `make check` runs per-crate clippy inside the Bazel suite with
`-D warnings` on rustc; `clippy::pedantic` is not enabled, so its class is
proposals only. Naming/derive findings are not gate-enforced.

### own (ownership-not-clone)

Judges: whether every clone/`to_owned`/`Rc`/`RefCell`/`Arc<Mutex>` is
explainable in one sentence; borrows beat copies; argument position takes the
cheapest thing (`&str` over `&String`, `&[T]` over `&Vec<T>`); `mem::take`
instead of clone; avoid statics.
Seeds:
  1. `\.clone\(\)`                # count per file; inspect each
  2. `\.to_owned\(\)|\.to_vec\(\)|\.to_string\(\)`
  3. `Rc<|RefCell<|Arc<Mutex<|Arc<RwLock<`
  4. `Cow<`
verify: `cargo clippy --all-targets`, `cargo test`;
`cargo clippy -- -W clippy::redundant_clone` is a one-off proposal generator
(nursery, off by default - never proposed for the repo lint config).
Applicable when: the crate declares `fn` with parameters. N/A only when all
seeds are 0 and no `fn` takes parameters.
Repo false positives: `Arc` clones at `tokio::spawn` boundaries (required for
`'static`); `OwnedFd`/`File` duplication in fd-passing paths (ADR 0034 explicit
fd transfer); clones of `Copy` newtypes; `Arc<Mutex<...>>` in the daemon's shared
state where multiple owners genuinely exist (`d2bd/src/composition.rs`
`ServerState`); `.to_string()` at a wire-rendering boundary; test fixtures.
Gate interaction: none denies clones. `clippy::redundant_clone` is not enabled;
its findings are candidates only.

### type (type-driven-design)

Judges: illegal states representable - boolean flag soup, `Option` pairs where
exactly one is `Some`, stringly-typed state, validate-at-every-callsite instead
of parse-once types, typestate for protocol order. Stopping rule: encode an
invariant only where violating it is a real bug class.
Seeds:
  1. `fn validate_\w+|fn check_\w+`   # runtime validation a parsed type could replace
  2. `is_\w+: bool|\w+_flag: bool`    # boolean flags that may be state
  3. `(mode|kind|state): String`      # stringly-typed state
verify: `cargo clippy --all-targets`, `cargo test`; after a change, grep that
the now-impossible branch is deleted.
Applicable when: the crate declares a `struct` or `enum`. N/A only when all
seeds are 0 and it declares neither.
Repo false positives: wire types that must mirror a schema (generated shapes,
`deny_unknown_fields` admission gates, docs/reference-pinned fields); the
`debug_logging`-class pinned wire fields; one-variant or two-variant enums that
are declared extension points for a declared provider; `StateDirIntent`-class
tokens kept because the daemon references them; schema-mirroring booleans in
generated code (lane X2 owns those).
Gate interaction: `docs/reference/manifest-schema.md` + `schemars`-generated
schemas + `docs/reference/error-codes.md` pin wire shapes; restructuring a wire
type is `needs-contract`. Generated shapes are X2's.

### api (rust-api-design)

Judges: what callers can see and rely on - deliberate exports, one path per
item, no `Arc`/`Rc`/`Box`/`RefCell` or dependency types in public signatures,
trait design (small required surface, sealed where growth is planned), semver
breakage classes.
Seeds:
  1. `\bpub (fn|struct|enum|trait|type|const|mod) `   # exported surface size
  2. `pub .*\b(Arc|Rc|Box|RefCell)<`                  # internals in a public signature
  3. `^\s*pub use `                                   # re-export arms
verify: `cargo doc --no-deps`, `cargo clippy --all-targets`, `cargo test`;
`cargo semver-checks check-release` only if already installed (never install).
Applicable when: the crate has a `lib` target with `pub` items. N/A only when
all seeds are 0.
Repo false positives: contract crates intentionally export wide wire
vocabularies (that IS the contract; narrowing is `needs-contract`); `pub use`
re-export arms in `lib.rs` are the house single-surface pattern; `test-support`
feature-gated exports consumed by other crates' tests; `Arc<...>` in a public
signature where the value genuinely shares ownership (evaluate, cite the call
sites); generated `pub` surface (X2's).
Gate interaction: no semver gate in the repo. Exported types of contract crates
are pinned by docs/reference and goldens: a change there is `needs-contract`.

### err (rust-errors)

Judges: panic policy vs `Result` boundary, error taxonomy split by caller
action, context survival, wire error codes.
Seeds (seed 1 is the skill's own audit):
  1. `\.unwrap\(\)|\.expect\(`     # panic site outside tests
  2. `let _ = |\.ok\(\);`          # swallowed `Result`
  3. `\bpanic!\(|\bunreachable!\(|\btodo!\(|\bunimplemented!\(`
  4. `enum \w*Error`               # error taxonomy shape
verify: `cargo clippy --all-targets`, `cargo test`; the skill's targeted audit
`rg '\.unwrap\(\)|\.expect\(' --glob '!**/tests/**' --glob '!**/benches/**' src/`.
Applicable when: always (a crate with fns can be read for panic policy); N/A
only when all seeds are 0 and no `fn` exists.
Repo false positives: `unwrap` in `#[cfg(test)]` and in `#[tokio::main]`/`main`
startup preconditions; `expect("fixed ... serializes")` on literally-built
values; `format!`-built error strings that are in fact wire error codes pinned by
`docs/reference/error-codes.md`; generated error tables; `let _ =` on a
deliberately ignored best-effort cleanup (judge per site).
Gate interaction: no lint denies `unwrap`/`expect` today (`unwrap_used`/
`expect_used` are restriction lints, not enabled - propose, never switch on).
`d2b_core::error::Error::all_kinds()` is the wire error catalog; a finding that
restructures a wire-visible error enum is `needs-contract`.
`docs/reference/error-codes.md` is generated from it.

### serde (rust-serde)

Judges: serde as the boundary where untrusted input becomes a domain type -
`try_from` validation, `rename_all` conventions, the three optionality meanings,
the four enum representations, `deny_unknown_fields` as a per-type decision,
`flatten` costs, hand-written `Deserialize` as admission gate.
Seeds:
  1. `derive\([^)]*(De)?[Ss]erialize`
  2. `serde\((rename_all|deny_unknown_fields|try_from|untagged|flatten|default|skip_serializing_if)`
  3. `impl .*Deserialize.*for`     # hand-written deserializers
  4. `serde_json::from_|serde_json::to_`
verify: `cargo test` (round-trip + real-payload tests), `cargo clippy`.
Applicable when: seeds 1-4 hit. N/A when all are 0 (crate crosses no wire).
Repo false positives: hand-written `Deserialize` impls that are live admission
gates (qemu guest/provider spec shapes - recorded refusal; do not re-flag);
generated `Wire` deserialize blocks (76 across contract crates - X2's; cite,
never flag); `deny_unknown_fields` deliberately absent on service-consumed
messages; `try_from` validation already applied.
Gate interaction: `docs/reference/manifest-schema.md` and `tests/golden/**` pin
wire shapes - changes there are `needs-contract`. The contract-crate
macro/boilerplate consolidation row (#C4) is recorded not-applied; re-proposing
it OK but must cite the record row and current sites.

### obs (rust-observability)

Judges: structured events with named fields; `tracing` over `println`; spans for
context; error chains logged once at the boundary that handles them; never a
secret in a field; lazy field construction.
Seeds (seed 1 and 2 are the skill's own greps):
  1. `\bprintln!\(|\beprintln!\(`          # expect zero in a library
  2. `(info|debug|warn|error|trace)!\("`   # interpolated message with no fields = smell
  3. `\.instrument\(|#\[instrument`        # span usage (context for judging)
  4. `tracing::|log::`                     # presence check
verify: `cargo clippy --all-targets`, `cargo test`.
Applicable when: always; N/A only when all seeds are 0 and the crate has no
`tracing`/`log` dependency.
Repo false positives: CLI user-facing stdout in `d2b/src/**` and `bin/**`
(product output, not telemetry - the skill itself carves this out); `xtask`
generators whose stdout IS the artifact; test fixtures printing; message-only
events where the fields live in the enclosing span; `d2b-telemetry`/otel
providers whose payload is the metric, not a log.
Gate interaction: the ADR 0010/0028 identifier-in-log redaction scan
(`security-scan` job in `.github/workflows/pr-l1-static-fast.yml`, implemented
by `packages/xtask/src/diagnostic_redaction.rs`) already gates
identifier-in-log; sites it covers are `policy-confirmed` - cite the gate file.

### docs (rust-docs)

Judges: doc comment as API contract - one-line first sentence, module docs,
canonical sections (`# Examples`, `# Errors`, `# Panics`, `# Safety`), doctests
that run (`ignore` = unchecked), intra-doc links, magic values documented with
the why.
Seeds:
  1. `^\s*pub (fn|struct|enum|trait|const|type)`   # public items needing docs
  2. `/// # (Examples|Errors|Panics|Safety)`       # canonical sections present
  3. `-> Result<`                                  # items that should carry `# Errors`
verify: `cargo doc --no-deps`, `cargo test --doc`, `cargo clippy --all-targets`.
Applicable when: seed 1 hits (public items exist). N/A when all seeds are 0.
Repo false positives: internal crates whose contract is the dossier/README and
whose items are crate-internal (`pub(crate)` correct); bin-only crates (the
skill: never add `missing_docs` to a binary crate); doc comments that narrate
policy deliberately (kept); generated docs.
Gate interaction: `missing_docs` is not enabled anywhere; `cargo doc` is not in
`make check`. A finding proposing the lint is a proposal, never imposed.

### perf (rust-performance)

Judges: allocation out of hot paths (`format!` in loops, `with_capacity`, clear
and reuse, `Cow`), collection choice for the access pattern, hashing with
attacker-controlled keys, iterator bounds-check elision, codegen flags as the
last five percent.
Seeds:
  1. `format!\(`                                   # allocation sites
  2. `Vec::new\(\)|VecDeque::new\(\)|HashMap::new\(\)|BTreeMap::new\(\)`  # grow-by-push candidates
  3. `\.to_string\(\)`                             # copies at boundaries
verify: `cargo bench` against a recorded baseline, `cargo clippy`,
`cargo test --release`.
Applicable when: always; N/A only when all seeds are 0.
Repo false positives: `format!` in error paths, audit rendering, and one-shot
diagnostics (cold); `Vec::new()` where the empty case is common; `to_string()`
inside `Display` impls; deliberate `String` building where the artifact is text
(`xtask` generators, wire rendering); the repo has a perf budget gate
(`make perf`) - a finding that claims a budget regression must name the budget.
Gate interaction: perf findings are `static (unmeasured)` unless a benchmark
exists; never claim a measured win. `#[inline]`/codegen-flag advice is taste and
low severity at most.

### conc (rust-concurrency)

Judges: the concurrency model picked from the workload shape (data parallelism,
scoped threads, channels, shared state last), weakest correct atomic ordering,
`Send`/`Sync` claims written down, `thread_local!` over `static mut`.
Seeds:
  1. `std::thread::|thread::spawn|thread::scope`
  2. `\bMutex<|\bRwLock<`
  3. `Atomic\w+|Ordering::`
  4. `thread_local!|unsafe impl (Send|Sync) for`
verify: `cargo test` (incl. ignored stress tests), `cargo clippy`,
`cargo miri test` where unsafe `Send`/`Sync` or atomics are involved.
Applicable when: seeds 1-4 hit. N/A when all are 0.
Repo false positives: `std::sync::Mutex` on genuinely synchronous paths
(`d2b-broker/src/ops/**`, the dedicated bounded worker per plan R4); atomics as
counters; `tokio::sync::*` re-exports; test-only synchronization.
Gate interaction: the async-gate scanner (`make check-async-gate`) and
`disallowed_methods` police the blocking subset; `await_holding_lock`/
`await_holding_refcell_ref` are denied. `// async-gate-allow: <reason>` markers
(283 sites, inventory `packages/xtask/data/async-gate-inventory.json`) are
deliberate exceptions - cite, never re-flag.

### async (async-rust)

Judges: runtime choice at the top; blocking work inside an async context;
guards held across `.await`; cancellation safety (irreversible step in one
non-cancellable piece); shared state across tasks; `Send` bounds; future size.
Seeds:
  1. `async fn|async move|\.await`
  2. `tokio::spawn|spawn_blocking|JoinSet|select!\(|join!\(`
  3. `tokio::sync::(Mutex|RwLock|Notify)`
  4. `#\[tokio::(main|test)\]|Runtime::block_on`
verify: `cargo clippy --all-targets`, `cargo test`; async bugs are
timing-dependent - multi-thread test flavor where the path changed.
Applicable when: seeds 1-4 hit. N/A when all are 0.
Repo false positives: `block_on` at process entry points and plain `#[test]`
harnesses (sanctioned with inline allows); `sync_channel` recv on the dedicated
worker thread (plan R4); `async-gate-allow` markers (deliberate; cite);
`spawn_blocking` sites already in `packages/xtask/data/blocking-census-baseline.json`
(tracked work - merely being present is not a finding; a conversion of one is
already-planned work, and only a site above the baseline is new).
Gate interaction: `make check-async-gate` (source scanner),
`make check-census` (blocking-API census vs committed baseline),
`await_holding_lock`/`await_holding_refcell_ref` deny,
`clippy::disallowed_methods` deny at the workspace lint table (see (d) 1 for the
level discrepancy note). Findings that propose one of the `clippy.toml`-named
replacements are `policy-confirmed` unless the site is on a recorded exception
list.

### unsafe (unsafe-rust)

Judges: justification (FFI / named perf win / language-inexpressible primitive),
a `// SAFETY:` comment stating the invariant on every block, safe wrappers that
cannot be misused, `# Safety` on every `pub unsafe fn`, the UB hazard list
(aliasing, uninit, invalid values, transmute, unwinding across FFI, data races),
Miri as the verification.
Seeds:
  1. `\bunsafe \{|\bunsafe fn|\bunsafe impl|\bunsafe extern`
  2. `// SAFETY:`
  3. `transmute|from_raw|MaybeUninit|mem::zeroed`
  4. `unsafe_code`                  # lint setting presence (forbid/deny/allow)
verify: `cargo miri test` (say so explicitly if unavailable; never install
nightly), `cargo clippy --all-targets`, `cargo test`.
Applicable when: seeds 1-3 hit, or an `unsafe_code = "allow"` manifest exists.
N/A when seeds 1-3 are all 0 (seed 4 alone - a `forbid(unsafe_code)` attribute -
does not make the lens applicable).
Repo false positives: `d2b-broker/src/sys.rs` per-site `#[allow(unsafe_code)]`
call wrappers are the sanctioned syscall boundary; `d2b-broker-fixture-syscall-surface`
is `unsafe_code = "allow"` deliberately (fixture); `#[unsafe(link_section = ...)]`
is edition-2024 syntax for an attribute (not an unsafe block); `unsafe` in a
doc comment is prose.
Gate interaction: `unsafe_code = "forbid"` where workspace lints are inherited;
local tables per (d) 8. The enumerated exception set in (d) 8 is exhaustive as of
`6ebdd4cec`; a new site not in it is itself a finding.

### ffi (rust-ffi)

Judges: thin translation layer with logic in core crates; nothing panics across
the boundary (`catch_unwind` at entry); every pointer states ownership;
`repr(C)`/`repr(transparent)`; handle over module-level state; library-prefixed
symbols; `CStr`/`CString` and pointer+length for strings/slices; edition-2024
forms (`unsafe extern`, `#[unsafe(no_mangle)]`).
Seeds:
  1. `extern "C"|no_mangle|unsafe\(link_section`
  2. `catch_unwind`
  3. `repr\(C\)|repr\(transparent\)`
  4. `CStr|CString|c_char`
verify: `cargo test` (core crate), `cargo miri test` for the Rust side,
`cargo clippy`; a foreign-language harness is the boundary check.
Applicable when: seeds 1-4 hit. N/A when all are 0 (most crates).
Repo false positives: `extern "C"`-shaped declarations in `libc`-binding call
sites that never cross a foreign caller; `repr(transparent)` newtypes over
handles (intended); `catch_unwind` used for process supervision without FFI
(judge: is it a boundary?); `c_char` in `nix`/`libc` syscall wrappers.
Gate interaction: none specific. The FFI-carrier set is in (d) 8.

### macro (rust-macros)

Judges: macro as a last resort (the three genuine answers: variadic interface,
impl-per-type generation, non-Rust DSL); `macro_rules!` before proc-macro;
hygiene and `$crate`; narrowest fragment specifiers; `_private` module; spanned
errors (`syn::Error::new_spanned` -> `to_compile_error`), never panics; proc-macro
logic in a sibling crate.
Seeds:
  1. `macro_rules!`
  2. `proc_macro|syn::|quote!`
  3. `\$crate`
  4. `to_compile_error|new_spanned`
verify: `cargo expand --lib` (propose if absent, never require), `cargo test`
(trybuild suite if present), `cargo clippy`.
Applicable when: seeds 1-4 hit. N/A when all are 0.
Repo false positives: the exported `redacted_debug!`-class macros (deliberate
API; their contract is `docs`); test-only helper macros; std macros
(`format!`, `vec!`) are not `macro_rules!` definitions.
Gate interaction: repo policy forbids adding new linters/formatters; a `trybuild`
proposal is a proposal.

### test (rust-testing)

Judges: the form follows the assertion (unit/integration/doc/property/
snapshot/golden, differential for ports); behavior not implementation; error
variants not `Display` strings; determinism (seeded generators, injected time,
no network); table-driven with failure messages; a test must be able to fail;
the expected value is human-written or from an independent source.
Seeds (scope: `src/**` + `tests/**`):
  1. `#\[test\]|#\[tokio::test\]`                       # test mass
  2. `assert_eq!\(|assert_ne!\(|assert!\(`              # assertion mass
  3. `proptest!|insta::assert|rstest`                   # property/snapshot tooling
  4. `#\[ignore\]`                                      # ignored tests
verify: `cargo test --all-features`, `cargo test --doc`,
`cargo clippy --all-targets --all-features`; a new test must be observed failing
first.
Applicable when: seeds 1-4 hit. N/A when all are 0.
Repo false positives: golden/snapshot tests pinned under `tests/golden/**`
(authoritative); policy-required `tests/registration.rs` shims (13-17 lines
calling a shared assertion - documented pattern, do not flag as trivial);
tests deliberately pinning wire shapes; `#[ignore]` where the ignore is
documented (stress/root-only); `test-support` features.
Gate interaction: `make test-unit` is the Layer-1 development umbrella;
`tests/AGENTS.md` governs test placement. "A test that cannot fail" is `high`
severity per the rubric.

### supply (rust-supply-chain)

Judges: advisories reached from this repo, unmaintained crates, licence policy,
duplicate versions, tree weight - every finding ends in a decision (upgrade /
replace / vendor / accept with a written reason).
verify: `cargo deny check`, `cargo audit`, `cargo tree -d`,
`cargo update --dry-run`.
Applicable when: NEVER a per-crate lane lens. Lane X1 owns supply at workspace
level. A per-crate lane may note a directly evidenced crate-manifest problem
(unused dependency: dep name appears nowhere in that crate's `src/`+`tests/`)
tagged `supply`, with the census as evidence.

## (d) Repo constraints ledger

Lanes MUST consult this before writing any finding. `policy-confirmed` verdicts
must cite the file:line below.

1. **Workspace lints** (root `Cargo.toml`): `unsafe_code = "forbid"` under
   `[workspace.lints.rust]`; `await_holding_lock`/`await_holding_refcell_ref`
   `deny`; `disallowed_methods = "deny"` in the `[workspace.lints.clippy]`
   table. **Discrepancy recorded**: the `clippy.toml` header says the level
   "stands at `allow`" while the manifest table says `deny`; the manifest's
   actual text is `deny` (Cargo.toml, `[workspace.lints.clippy]`), and the
   Makefile `check-clippy` comment also says "allowed there". Record the
   manifest text as authoritative; a finding may not assume the level is
   advisory. Only 15 crates inherit the workspace table
   (`[lints] workspace = true`); the other 79 carry local `[lints]` tables that
   mirror `unsafe_code` + the three clippy lints (verified: every member manifest
   carries a lints table or reference; none lacks one).

2. **`clippy.toml` disallowed-methods list + replacement vocabulary**: tokio
   timer/sync/fs/net, `AsyncFd` over non-blocking descriptors,
   `d2b-core`'s `loader_worker` bounded-worker shape (one thread, bounded
   `sync_channel`, no `try_send` growth), `Notify` armed before check +
   `timeout`, `d2b-session-unix`'s `SeqpacketSocket` wrappers,
   `d2bd::forward_rendezvous`'s `AsyncFd<Socket>`. `parking_lot` is banned
   outright (KD3) except the R4 worker boundary. A finding that proposes one of
   these already-named replacements is `policy-confirmed` unless the site is on
   a recorded exception list (per-site allows with sanctioned reasons; see 4).

3. **Async gate**: `make check-async-gate` -> `cargo xtask check-async-gate`
   scanner (`packages/xtask/src/async_gate.rs`); inventory
   `packages/xtask/data/async-gate-inventory.json` (1,324 lines, 283 marker
   sites at HEAD); source marker `// async-gate-allow: <reason>`. Markers are
   deliberate exceptions - cite, do not re-flag. Scanner flags a
   `lock()`/`read()`/`write()` method call inside an async context not followed
   by `.await`.

4. **Blocking census**: `make check-census` -> `cargo xtask blocking-census
   --check packages/xtask/data/blocking-census-baseline.json` (per-crate
   baseline; a covered crate above baseline fails). Per-site
   `#[allow(clippy::disallowed_methods, reason = "...")]` allows are tracked by
   `xtask provider-crate-policy`: sanctioned reasons are exactly
   `"dedicated bounded worker per plan R4"`, `"synchronous path"`,
   `"CLI-only path"`, `"cfg(test) helper"`; one module-level blanket allow
   exemption exists (`packages/d2b-broker-composition/src/dependency_surface.rs`).

5. **Provider crate policy** (`packages/xtask/src/provider_crate_policy.rs`):
   README-only integration ratchet - exactly 18 crates (activation-nixos,
   audio-pipewire, clipboard-wayland, credential-entra,
   credential-managed-identity, credential-secret-service, device-gpu,
   display-wayland, notification-desktop, process-minijail, process-systemd,
   guest-azure-container-apps, guest-azure-virtual-machine,
   guest-cloud-hypervisor, system-core, transport-azure-relay, transport-unix,
   volume-virtiofs) whose `integration/*.rs` is a recorded scaffold rather than
   an executable scenario. Required paths per provider crate: `src`, `tests`,
   `integration`, `README.md`; nine required README sections. Also closes the
   accepted Provider matrix, shared-driver placements, family-knowledge
   ratchets, structural-knowledge ratchets, Bazel visibility, committed scope,
   and generated provenance. Findings that would remove or rewrite a
   ratcheted/policy-required path are `policy-confirmed` (cite
   provider_crate_policy.rs line).

6. **Refusal ledger**: `docs/explanation/over-engineering-audit-record.md`
   (the prior 242-finding provider/runtime audit; tree state `515cbf610`).
   Refused classes (finding on these is `policy-confirmed` unless it cites
   changed evidence):
   - policy-required scaffolds (`integration/*.rs` + README paths; finding 9 of
     the policy family);
   - declared-provider zero-caller artifacts (transport-unix, transport-vsock;
     pinned by policy matrix, `nixos-modules/provider-runtime-contracts.nix`,
     dossiers, committed schemas);
   - pinned wire fields / Nix-pinned catalogs (display Wayland global catalog,
     `debug_logging`);
   - hand-written `Deserialize` impls that are live admission gates (qemu
     guest/provider spec shapes);
   - cross-crate refactors refused for ownership (supervisor blocking executor,
     ZoneLink enrollment-machine merge, host/user driver merge);
   - the toolkit's unconsumed framework half (B3 - declared-but-unwired);
   - bus-side watch sink / `d2b-resource-api/src/watch.rs` (B1 kept half);
   - `d2b-provider` agent dispatcher half (B2 kept half);
   - audio `AudioMediator` defaults / `AudioReadiness` / `FakeAudioMediator`;
   - supervisor generic systemd seam; tpm state-intent tokens and
     `swtpm_argv` input fields; USBIP dossier-declared surface
     (`state_machine.rs`, effect-port tests, `BindingLifecycle`);
   - build/packaging consolidation findings (80-88) refused as repo-wide work.
   "Not applied" rows (no refusal reason recorded; unchanged code): A4-A8,
   B9, C1-C10, and the family gaps the record itself names. A finding on a
   not-applied row is actionable but must cite the row id and confirm the site
   still matches.

7. **Wire and contract surfaces**: `docs/reference/error-codes.md` (generated
   from `d2b_core::error::Error::all_kinds()`), `docs/reference/cli-contract.md`,
   `docs/reference/manifest-schema.md`, `docs/reference/daemon-api.md`,
   `tests/golden/**`, and every `src/generated/**` file -> any change here is
   `needs-contract`.

8. **`unsafe` exceptions (enumerated by seed at `6ebdd4cec`; never assume the
   set)**:
   - Files containing `unsafe` blocks/fns/impls (match counts): 
     `packages/d2b-broker/src/sys.rs` (103),
     `packages/d2b-host-activation-helper/src/main.rs` (24),
     `packages/d2b-broker/src/seccomp_compile_tests.rs` (5),
     `packages/d2b-broker-fixture-syscall-surface/src/lib.rs` (3),
     `packages/d2b-broker/tests/socket_activation.rs` (1),
     `packages/d2b-broker/src/ops/disk_init.rs` (1),
     `packages/d2b-resource-compiler/src/linux.rs` (1, `execveat` with SAFETY).
     (`d2bd-runtime/src/typed_error.rs` matched only a doc-comment word -
     false positive.)
   - `#[allow(unsafe_code)]` sites: `d2b-broker/src/sys.rs` (52),
     `d2b-broker/src/seccomp_compile_tests.rs` (3),
     `d2b-broker/src/ops/disk_init.rs` (1),
     `d2b-broker/tests/socket_activation.rs` (1).
   - Manifest `unsafe_code` settings: `forbid` (most, incl. all provider
     crates), `deny` (`d2b-broker`, `d2b-broker-composition`,
     `d2b-broker-fixture-handlers`, `d2b-sk-frontend`), `allow`
     (`d2b-broker-fixture-syscall-surface`), ABSENT (`d2b-audit`,
     `d2b-host-activation-helper`, `d2b-resource-compiler`, `d2b-telemetry`,
     `d2b-zone-routing`; of these, only host-activation-helper (24 sites) and
     resource-compiler (1) actually contain `unsafe`; the other three contain
     none). Crate-level `#![forbid(unsafe_code)]` appears in several `lib.rs`.
   - `// SAFETY:` comments: 35 across the workspace. A block without one is a
     finding (the skill's mechanical rule).

9. **Toolchain**: `rust-toolchain.toml` pins channel `1.97.0` (stable,
   components rustfmt+clippy); all 94 member manifests are edition 2024
   (verified). Lens advice must be edition-legal (let-chains and if-let chains
   are available; `#[unsafe(no_mangle)]`/`unsafe extern` forms required).

10. **Repo prose rules binding this report**: ASCII `-` only in every document
    written (including the ASCII hyphen prohibition list in AGENTS.md); no
    tool/model/agent attribution anywhere; finding ids (`RS-####`, and lane-local
    `<lane>#<k>`) are report-local - remediation later must not copy them into
    source comments, changelogs, commit messages, or PR bodies.

11. **Authoritative context, not audit targets**: `docs/explanation/over-engineering-audit-record.md`,
    `docs/adr/**`, `docs/specs/**` dossiers, `docs/residual-review-findings/**`.
    They may be cited and must not be flagged for change.

12. **Security invariants**: the Don'ts list in `AGENTS.md` plus
    `docs/contributing/critical-subsystems.md`. A finding that would violate a
    Don't is `policy-confirmed` and must cite the Don't (e.g. no per-Guest
    systemd units; no host-state mutation outside ownership markers; no broad
    chmod/chown/setfacl/`/run/d2b` sweeps; no new storage/ACL/lock ownership
    outside ADR 0034's single-repair-owner rule; no d2b cgroup mutation outside
    delegation).

## (e) Per-crate applicability matrix

Seed-hit counts per crate x lens, measured 2026-09-24 at `6ebdd4cec`. Basis:
matching lines summed across the lens's seeds over `packages/<crate>/src/**`
excluding `src/generated/**` (the `test` lens adds `tests/**`). Cell `0` = all
seeds zero for that crate (N/A candidate; the card's applicability criteria
decide). Cell `X1`: `supply` is a workspace-level lens owned by lane X1; it is
never applicable per crate. Hit counts are raw match mass, not finding counts;
noisy seeds (`own` `.clone()`, `docs` `-> Result<`) are expected to dominate and
are filtered by lane reading.

| crate | idiom | own | type | api | err | serde | obs | docs | perf | conc | async | unsafe | ffi | macro | test | supply |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| d2b | 15 | 500 | 46 | 117 | 240 | 194 | 33 | 335 | 319 | 44 | 77 | 4 | 0 | 0 | 1076 | X1 |
| d2b-audit | 14 | 166 | 6 | 149 | 204 | 84 | 5 | 167 | 58 | 18 | 0 | 1 | 0 | 1 | 206 | X1 |
| d2b-broker | 83 | 2689 | 69 | 625 | 2381 | 332 | 116 | 1162 | 1577 | 144 | 2123 | 283 | 79 | 6 | 2959 | X1 |
| d2b-broker-composition | 2 | 13 | 0 | 24 | 18 | 1 | 16 | 31 | 24 | 0 | 17 | 1 | 0 | 8 | 79 | X1 |
| d2b-broker-fixture-handlers | 0 | 1 | 0 | 1 | 0 | 0 | 0 | 1 | 1 | 0 | 1 | 1 | 0 | 0 | 0 | X1 |
| d2b-broker-fixture-syscall-surface | 0 | 0 | 0 | 3 | 0 | 0 | 0 | 3 | 0 | 0 | 0 | 4 | 1 | 0 | 0 | X1 |
| d2b-bus | 23 | 522 | 7 | 325 | 1376 | 2 | 15 | 470 | 169 | 127 | 661 | 0 | 0 | 1 | 712 | X1 |
| d2b-contracts | 6 | 146 | 16 | 552 | 209 | 378 | 0 | 636 | 107 | 0 | 0 | 0 | 0 | 7 | 467 | X1 |
| d2b-contracts-broker | 1 | 82 | 5 | 202 | 143 | 476 | 0 | 210 | 80 | 0 | 0 | 0 | 0 | 0 | 178 | X1 |
| d2b-contracts-control | 0 | 94 | 22 | 241 | 92 | 660 | 0 | 244 | 57 | 0 | 0 | 0 | 0 | 0 | 169 | X1 |
| d2b-contracts-provider | 9 | 142 | 29 | 591 | 404 | 213 | 1 | 720 | 75 | 7 | 3 | 0 | 0 | 1 | 475 | X1 |
| d2b-contracts-resource | 15 | 216 | 30 | 1065 | 585 | 774 | 0 | 1277 | 266 | 2 | 0 | 0 | 0 | 8 | 644 | X1 |
| d2b-contracts-zone-session | 5 | 127 | 19 | 673 | 339 | 355 | 0 | 822 | 127 | 0 | 0 | 0 | 0 | 5 | 422 | X1 |
| d2b-controller-toolkit | 0 | 0 | 0 | 22 | 0 | 0 | 0 | 18 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | X1 |
| d2b-core | 19 | 505 | 11 | 507 | 297 | 552 | 1 | 520 | 383 | 4 | 14 | 1 | 0 | 1 | 540 | X1 |
| d2b-core-controller | 5 | 441 | 11 | 512 | 468 | 49 | 0 | 681 | 58 | 36 | 87 | 0 | 0 | 0 | 395 | X1 |
| d2b-host | 18 | 404 | 16 | 306 | 300 | 125 | 28 | 405 | 295 | 2 | 318 | 19 | 3 | 1 | 550 | X1 |
| d2b-host-activation-helper | 0 | 1 | 0 | 0 | 14 | 0 | 7 | 2 | 5 | 0 | 0 | 26 | 7 | 0 | 7 | X1 |
| d2b-process-conformance | 0 | 58 | 4 | 227 | 147 | 24 | 0 | 264 | 14 | 1 | 8 | 0 | 0 | 1 | 158 | X1 |
| d2b-provider | 2 | 26 | 0 | 170 | 16 | 0 | 0 | 198 | 5 | 34 | 41 | 0 | 0 | 0 | 100 | X1 |
| d2b-provider-activation-nixos | 0 | 59 | 1 | 95 | 64 | 9 | 17 | 111 | 35 | 10 | 123 | 0 | 0 | 0 | 198 | X1 |
| d2b-provider-audio-binding | 1 | 1 | 0 | 14 | 0 | 0 | 0 | 16 | 0 | 0 | 0 | 0 | 0 | 0 | 20 | X1 |
| d2b-provider-audio-pipewire | 1 | 8 | 3 | 100 | 7 | 10 | 1 | 124 | 4 | 4 | 2 | 2 | 0 | 0 | 174 | X1 |
| d2b-provider-audio-service | 0 | 0 | 0 | 8 | 0 | 0 | 0 | 9 | 2 | 0 | 0 | 0 | 0 | 0 | 19 | X1 |
| d2b-provider-clipboard-wayland | 28 | 430 | 10 | 391 | 269 | 97 | 178 | 514 | 229 | 30 | 0 | 7 | 0 | 0 | 492 | X1 |
| d2b-provider-command | 0 | 6 | 0 | 27 | 19 | 10 | 0 | 30 | 1 | 0 | 0 | 0 | 0 | 0 | 14 | X1 |
| d2b-provider-config-nixos | 1 | 30 | 6 | 63 | 16 | 38 | 25 | 90 | 11 | 1 | 8 | 2 | 0 | 0 | 45 | X1 |
| d2b-provider-credential | 0 | 61 | 2 | 85 | 62 | 14 | 2 | 102 | 24 | 15 | 137 | 0 | 0 | 0 | 128 | X1 |
| d2b-provider-credential-entra | 0 | 43 | 2 | 73 | 20 | 0 | 35 | 120 | 6 | 5 | 101 | 1 | 0 | 0 | 226 | X1 |
| d2b-provider-credential-managed-identity | 0 | 52 | 1 | 88 | 23 | 0 | 26 | 129 | 7 | 4 | 55 | 1 | 0 | 0 | 213 | X1 |
| d2b-provider-credential-secret-service | 0 | 84 | 2 | 62 | 57 | 0 | 32 | 135 | 18 | 30 | 121 | 1 | 0 | 0 | 196 | X1 |
| d2b-provider-device | 0 | 3 | 0 | 31 | 0 | 1 | 0 | 35 | 1 | 5 | 16 | 0 | 0 | 0 | 9 | X1 |
| d2b-provider-device-gpu | 2 | 51 | 1 | 142 | 22 | 29 | 17 | 166 | 9 | 2 | 0 | 2 | 0 | 0 | 120 | X1 |
| d2b-provider-device-security-key | 1 | 55 | 1 | 187 | 68 | 9 | 21 | 213 | 11 | 16 | 77 | 1 | 0 | 0 | 151 | X1 |
| d2b-provider-device-tpm | 1 | 71 | 2 | 91 | 83 | 24 | 11 | 125 | 29 | 2 | 77 | 1 | 0 | 0 | 155 | X1 |
| d2b-provider-device-usbip | 1 | 104 | 5 | 313 | 22 | 26 | 45 | 424 | 9 | 11 | 22 | 0 | 0 | 0 | 229 | X1 |
| d2b-provider-display-wayland | 27 | 596 | 7 | 423 | 166 | 32 | 91 | 476 | 138 | 0 | 0 | 8 | 4 | 1 | 548 | X1 |
| d2b-provider-emergency-policy | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | X1 |
| d2b-provider-endpoint | 3 | 23 | 1 | 78 | 40 | 44 | 0 | 97 | 14 | 13 | 102 | 0 | 0 | 0 | 81 | X1 |
| d2b-provider-guest | 6 | 158 | 6 | 130 | 135 | 55 | 13 | 213 | 54 | 34 | 290 | 0 | 0 | 0 | 152 | X1 |
| d2b-provider-guest-azure-container-apps | 0 | 44 | 0 | 94 | 4 | 31 | 15 | 128 | 0 | 0 | 51 | 1 | 0 | 1 | 53 | X1 |
| d2b-provider-guest-azure-virtual-machine | 2 | 18 | 1 | 77 | 1 | 21 | 27 | 106 | 0 | 0 | 72 | 1 | 0 | 0 | 106 | X1 |
| d2b-provider-guest-cloud-hypervisor | 8 | 158 | 11 | 361 | 121 | 57 | 53 | 453 | 22 | 0 | 127 | 1 | 0 | 0 | 273 | X1 |
| d2b-provider-guest-qemu-media | 5 | 44 | 5 | 140 | 15 | 63 | 24 | 156 | 15 | 0 | 0 | 1 | 0 | 0 | 140 | X1 |
| d2b-provider-host | 1 | 32 | 4 | 47 | 40 | 4 | 0 | 64 | 16 | 18 | 145 | 0 | 0 | 0 | 95 | X1 |
| d2b-provider-network-local | 11 | 206 | 15 | 328 | 171 | 29 | 4 | 453 | 125 | 8 | 134 | 0 | 0 | 1 | 347 | X1 |
| d2b-provider-notification-desktop | 5 | 108 | 5 | 245 | 107 | 19 | 4 | 341 | 45 | 1 | 0 | 1 | 0 | 0 | 172 | X1 |
| d2b-provider-observability-otel | 5 | 52 | 3 | 128 | 66 | 13 | 3 | 131 | 12 | 20 | 0 | 4 | 0 | 0 | 158 | X1 |
| d2b-provider-operation | 1 | 1 | 0 | 68 | 17 | 49 | 0 | 72 | 0 | 0 | 0 | 0 | 0 | 0 | 20 | X1 |
| d2b-provider-process | 2 | 274 | 7 | 135 | 185 | 18 | 8 | 250 | 129 | 36 | 444 | 1 | 0 | 0 | 325 | X1 |
| d2b-provider-process-minijail | 0 | 2 | 1 | 15 | 2 | 0 | 1 | 22 | 1 | 0 | 19 | 0 | 0 | 0 | 74 | X1 |
| d2b-provider-process-systemd | 3 | 51 | 4 | 78 | 34 | 10 | 4 | 100 | 37 | 0 | 90 | 0 | 0 | 0 | 154 | X1 |
| d2b-provider-provider | 2 | 24 | 1 | 42 | 54 | 15 | 5 | 56 | 15 | 11 | 72 | 0 | 0 | 0 | 74 | X1 |
| d2b-provider-quota | 0 | 0 | 0 | 9 | 0 | 2 | 0 | 6 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | X1 |
| d2b-provider-resource-export | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | X1 |
| d2b-provider-resource-import | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | X1 |
| d2b-provider-role | 0 | 0 | 0 | 11 | 5 | 0 | 0 | 9 | 4 | 1 | 0 | 0 | 0 | 0 | 11 | X1 |
| d2b-provider-role-binding | 0 | 0 | 0 | 2 | 0 | 0 | 0 | 1 | 0 | 0 | 0 | 0 | 0 | 0 | 1 | X1 |
| d2b-provider-seccomp-profile | 0 | 2 | 0 | 20 | 10 | 30 | 0 | 21 | 1 | 0 | 0 | 0 | 0 | 0 | 9 | X1 |
| d2b-provider-shell-pool | 0 | 0 | 0 | 10 | 0 | 0 | 0 | 11 | 1 | 0 | 0 | 0 | 0 | 0 | 16 | X1 |
| d2b-provider-shell-session | 0 | 1 | 0 | 10 | 0 | 2 | 0 | 11 | 1 | 0 | 0 | 0 | 0 | 0 | 29 | X1 |
| d2b-provider-shell-terminal | 0 | 40 | 7 | 162 | 5 | 0 | 2 | 229 | 10 | 3 | 2 | 0 | 0 | 0 | 142 | X1 |
| d2b-provider-supervisor | 4 | 138 | 1 | 32 | 144 | 24 | 3 | 130 | 40 | 75 | 42 | 0 | 0 | 1 | 128 | X1 |
| d2b-provider-system-core | 0 | 7 | 0 | 74 | 11 | 11 | 2 | 83 | 0 | 1 | 15 | 0 | 0 | 0 | 83 | X1 |
| d2b-provider-telemetry-binding | 1 | 24 | 2 | 22 | 22 | 7 | 0 | 37 | 11 | 4 | 119 | 0 | 0 | 0 | 70 | X1 |
| d2b-provider-telemetry-service | 1 | 15 | 2 | 18 | 20 | 3 | 0 | 30 | 10 | 4 | 83 | 0 | 0 | 0 | 52 | X1 |
| d2b-provider-test-controller | 0 | 1 | 0 | 0 | 10 | 0 | 10 | 4 | 0 | 0 | 14 | 1 | 0 | 0 | 11 | X1 |
| d2b-provider-toolkit | 12 | 226 | 13 | 565 | 258 | 41 | 14 | 720 | 77 | 81 | 325 | 0 | 0 | 0 | 472 | X1 |
| d2b-provider-transport-azure-relay | 2 | 46 | 3 | 148 | 87 | 9 | 25 | 213 | 24 | 15 | 133 | 1 | 0 | 0 | 199 | X1 |
| d2b-provider-transport-unix | 2 | 0 | 2 | 50 | 3 | 0 | 9 | 48 | 4 | 1 | 1 | 0 | 0 | 0 | 26 | X1 |
| d2b-provider-transport-vsock | 3 | 12 | 0 | 116 | 16 | 6 | 13 | 144 | 3 | 24 | 112 | 2 | 0 | 0 | 135 | X1 |
| d2b-provider-user | 0 | 37 | 2 | 39 | 49 | 5 | 1 | 53 | 11 | 19 | 112 | 0 | 0 | 0 | 113 | X1 |
| d2b-provider-volume | 0 | 63 | 1 | 29 | 44 | 8 | 0 | 55 | 13 | 21 | 114 | 0 | 0 | 0 | 85 | X1 |
| d2b-provider-volume-binding | 0 | 81 | 0 | 36 | 76 | 24 | 1 | 58 | 24 | 24 | 141 | 0 | 0 | 0 | 134 | X1 |
| d2b-provider-volume-local | 8 | 87 | 16 | 319 | 121 | 90 | 23 | 459 | 45 | 111 | 60 | 22 | 0 | 0 | 369 | X1 |
| d2b-provider-volume-virtiofs | 0 | 13 | 1 | 69 | 20 | 12 | 15 | 79 | 12 | 4 | 29 | 0 | 0 | 0 | 128 | X1 |
| d2b-provider-wayland-policy | 0 | 49 | 3 | 81 | 46 | 23 | 0 | 134 | 15 | 11 | 98 | 0 | 0 | 0 | 100 | X1 |
| d2b-provider-wayland-session | 1 | 4 | 0 | 15 | 0 | 0 | 0 | 17 | 0 | 0 | 0 | 0 | 0 | 0 | 26 | X1 |
| d2b-provider-zone | 0 | 2 | 0 | 13 | 1 | 0 | 0 | 11 | 0 | 0 | 0 | 0 | 0 | 0 | 17 | X1 |
| d2b-provider-zone-link | 5 | 53 | 1 | 138 | 115 | 4 | 0 | 152 | 6 | 3 | 0 | 0 | 0 | 0 | 209 | X1 |
| d2b-resource-api | 10 | 408 | 4 | 134 | 685 | 40 | 38 | 232 | 147 | 40 | 340 | 0 | 0 | 4 | 441 | X1 |
| d2b-resource-client | 1 | 76 | 2 | 207 | 99 | 4 | 0 | 244 | 31 | 53 | 101 | 0 | 0 | 0 | 178 | X1 |
| d2b-resource-compiler | 12 | 113 | 14 | 67 | 51 | 27 | 1 | 118 | 157 | 5 | 0 | 8 | 5 | 0 | 148 | X1 |
| d2b-resource-runtime | 22 | 448 | 2 | 401 | 559 | 8 | 1 | 592 | 107 | 209 | 1259 | 0 | 0 | 0 | 707 | X1 |
| d2b-resource-types | 0 | 2 | 0 | 97 | 3 | 0 | 0 | 85 | 1 | 0 | 3 | 0 | 0 | 0 | 46 | X1 |
| d2b-session | 8 | 106 | 11 | 317 | 168 | 0 | 19 | 536 | 35 | 72 | 505 | 1 | 0 | 3 | 383 | X1 |
| d2b-session-unix | 12 | 23 | 10 | 157 | 66 | 0 | 4 | 251 | 25 | 34 | 130 | 5 | 0 | 1 | 214 | X1 |
| d2b-sk-frontend | 1 | 6 | 0 | 25 | 2 | 0 | 2 | 29 | 18 | 2 | 39 | 3 | 0 | 0 | 38 | X1 |
| d2b-telemetry | 0 | 23 | 3 | 94 | 85 | 8 | 0 | 107 | 10 | 11 | 0 | 1 | 0 | 0 | 114 | X1 |
| d2b-unsafe-local-helper | 5 | 77 | 4 | 39 | 120 | 22 | 10 | 103 | 36 | 20 | 0 | 2 | 0 | 0 | 112 | X1 |
| d2b-zone-routing | 11 | 147 | 3 | 208 | 225 | 0 | 0 | 248 | 36 | 38 | 6 | 0 | 0 | 1 | 528 | X1 |
| d2bd | 82 | 3416 | 25 | 316 | 2827 | 471 | 515 | 1160 | 1035 | 400 | 2850 | 22 | 0 | 0 | 2192 | X1 |
| d2bd-runtime | 58 | 1322 | 44 | 1031 | 1261 | 261 | 153 | 1356 | 576 | 317 | 583 | 11 | 0 | 0 | 1543 | X1 |
| xtask | 166 | 1059 | 81 | 457 | 926 | 301 | 75 | 774 | 1157 | 50 | 99 | 4 | 1 | 25 | 1819 | X1 |

Interpretation rules:

- A cell is `0` only when every seed returned zero matching lines; the lane then
  states N/A (with the criteria note) or clean, per the card.
- A partition lane's share of a crate's counts is its assigned files/ranges;
  the sum of parts approximates the crate cell (split files divide their
  matches).
- Cells are raw match mass; a lane's own run over its assigned scope is
  authoritative for its coverage lines.

## (f) Lane map + partition rule: published part map

Partitions below are final (computed mechanically; LPT packing, cap
`ceil(LOC/k) x 1.2`, units whole except the recorded item-range splits). Lanes
follow this map; no renegotiation. Each part lane's header records the part
(`part k/n`), its file list, and - for split files - the line ranges.

Item-range splits (recorded boundaries; 1-based inclusive):
- `d2bd/src/composition.rs` -> `1-10070` | `10071-20142` | `20143-30213`.
- `d2b-broker/src/runtime.rs` -> `1-10300` | `10301-20603`.
- `xtask/src/provider_crate_policy.rs` -> `1-5353` | `5354-11075`.

|part id|scope (files / dirs / ranges)|
|---|---|
|`d2bd-p1`|`src/resource_runtime.rs`|
|`d2bd-p2`|`src/composition.rs:10071-20142`, `src/audio_host_controller.rs`|
|`d2bd-p3`|`src/composition.rs:20143-30213`, `src/zone_enrollment.rs`|
|`d2bd-p4`|`src/composition.rs:1-10070`, `src/plane_port.rs`|
|`d2bd-p5`|`src/interaction_composition.rs`, `src/foundation_seed.rs`, `src/principal_allocation.rs`, `src/provider_shutdown.rs`, `src/process_resource_runtime.rs`|
|`d2bd-p6`|`src/resource_plane_v3.rs`, `src/provider_lifecycle.rs`, `src/resource_runtime/**`, `src/credential_resource_runtime.rs`|
|`d2bd-p7`|`src/process_provider_runtime.rs`, `src/provider_effects.rs`, `src/effect_service_actors.rs`, `src/main.rs`, `src/guest_target_session.rs`, `src/lib.rs`|
|`d2bd-p8`|`src/forward_rendezvous.rs`, `src/shared_provider_effects.rs`, `src/provider_registry.rs`, `src/audio_dispatch.rs`|
|`d2b-broker-p1`|`src/runtime.rs:10301-20603`, `src/ops/usbip_lock.rs`, `src/seccomp_compile_tests.rs`|
|`d2b-broker-p2`|`src/runtime.rs:1-10300`, `src/ops/gpu.rs`, `src/ops/modprobe.rs`|
|`d2b-broker-p3`|`src/live_handlers.rs`, `src/state_cells.rs`, `src/ops/store_sync.rs`, `src/ops/usbip_host.rs`, `src/ops/storage_contract.rs`, `src/ops/pidfd.rs`, `src/ops/sysctl.rs`, `src/ops/mod.rs`|
|`d2b-broker-p4`|`src/audit.rs`, `src/ops/tap.rs`, `src/ops/disk_init.rs`, `src/ops/route.rs`, `src/ops/store_sync_audit.rs`, `src/ops/spawn_runner.rs`, `src/ops/host_generation_handoff.rs`, `src/ops/store_sync_export.rs`|
|`d2b-broker-p5`|`src/sys.rs`, `src/ops/swtpm_dir.rs`, `src/ops/nft.rs`, `src/forwarding.rs`, `src/ops/cgroup.rs`, `src/ops/device.rs`, `src/ops/hosts.rs`, `src/fd_passing.rs`, `src/lib.rs`|
|`d2b-broker-p6`|`src/envelope/**`, `src/ops/exec_reconcile.rs`, `src/ops/audit_op.rs`, `src/ops/store_view_posture.rs`, `src/ops/device_worker.rs`, `src/ops/nm.rs`, `src/ops/security_key.rs`, `src/ops/store_view_farm.rs`, `src/ops/usbip_firewall.rs`|
|`d2b-broker-p7`|`src/ops/media.rs`, `src/kernel_ops.rs`, `src/ops/network.rs`, `src/catalog.rs`, `src/ops/state_dir.rs`, `src/ops/state-posture-contract.json`, `src/protocol.rs`, `src/bootstrap.rs`|
|`xtask-p1`|`src/provider_crate_policy.rs:5354-11075`, `src/main.rs`, `src/gen_layer_catalogs.rs`, `src/provider_registration_authority.rs`, `src/service_catalog.rs`|
|`xtask-p2`|`src/provider_crate_policy.rs:1-5353`, `src/gen_broker_operations.rs`, `src/delivery/eligibility.rs`, `src/diagnostic_redaction.rs`, `src/delivery/history_proof.rs`|
|`xtask-p3`|`src/delivery/recovery.rs`, `src/delivery/command.rs`, `src/resource_type_authority.rs`, `src/delivery/snapshot.rs`, `src/provider_packaging.rs`, `src/semantic_service_schemas.rs`, `src/deadcode.rs`, `src/authority_common.rs`, `src/bin/**`|
|`xtask-p4`|`src/delivery/storage.rs`, `src/production_closure.rs`, `src/zone_schema.rs`, `src/delivery/model.rs`, `src/operation_row_authority.rs`, `src/inventory.rs`, `src/delivery/mod.rs`|
|`xtask-p5`|`src/changelog.rs`, `src/async_gate.rs`, `src/blocking_census.rs`, `src/delivery/evidence.rs`, `src/nix_inventories.rs`, `src/bazel_evidence.rs`, `src/delivery/seal.rs`|
|`d2bd-runtime-p1`|`src/supervisor/**`, `src/typed_error.rs`, `src/autostart.rs`, `src/component_session_vsock.rs`, `src/daemon_config.rs`, `src/resource_api.rs`, `src/zone_authority.rs`, `src/shell_backend.rs`, `src/broker_transport.rs`, `src/public_read_model.rs`, `src/vm_start_support.rs`, `src/json_io.rs`|
|`d2bd-runtime-p2`|`src/resource_runtime_support.rs`, `src/guest_resource_runtime.rs`, `src/workload_dispatch.rs`, `src/runtime_process.rs`, `src/workload_target_index.rs`, `src/wire.rs`, `src/ssh_host_key_preflight.rs`, `src/public_projection.rs`, `src/resource_operator_activation.rs`, `src/exec_detached.rs`, `src/admission.rs`, `src/daemon_client.rs`, `src/runtime_capability.rs`|
|`d2bd-runtime-p3`|`src/exec_session.rs`, `src/unsafe_local_helper.rs`, `src/metrics.rs`, `src/authority_persistence.rs`, `src/readiness.rs`, `src/otel_host_bridge_readiness.rs`, `src/ownership_preflight.rs`, `src/unix_transport.rs`, `src/exec_session_real.rs`, `src/terminal_session.rs`, `src/pidfs_probe.rs`, `src/lib.rs`, `src/runtime_util.rs`|
|`d2bd-runtime-p4`|`src/target_runtime.rs`, `src/daemon_audit.rs`, `src/guest_mode.rs`, `src/kernel_module_check.rs`, `src/console_session.rs`, `src/guest_component_session.rs`, `src/ch_stats.rs`, `src/concurrency.rs`, `src/daemon_version.rs`, `src/ch_api.rs`, `src/typed_shell_targets.rs`, `src/wire_response_helpers.rs`, `src/exec_support.rs`|
|`d2b-p1`|`src/context.rs`, `src/activation.rs`, `src/zone_support_bundle.rs`, `src/share.rs`, `src/guest.rs`, `src/zone.rs`, `src/host_generation.rs`, `src/runtime.rs`, `src/main.rs`|
|`d2b-p2`|`src/doctor.rs`, `src/zone_audit.rs`, `src/resource.rs`, `src/host_validate.rs`, `src/shell.rs`, `src/host.rs`, `src/lib.rs`, `src/complete.rs`|
|`d2b-p3`|`src/exec_client.rs`, `src/dispatch.rs`, `src/debug.rs`, `src/zone_doctor.rs`, `src/exec.rs`, `src/endpoint.rs`, `src/provider.rs`, `src/terminal_client.rs`|
|`d2b-bus-p1`|`src/router.rs`, `src/authorization.rs`, `src/registry.rs`, `src/metrics.rs`|
|`d2b-bus-p2`|`src/session/**`, `src/session_seam_tests.rs`, `src/streams.rs`, `src/operations.rs`, `src/wire.rs`, `src/lib.rs`|
|`d2b-contracts-resource-p1`|`src/v3/network.rs`, `src/v3/resource_schema.rs`, `src/v3/operations/**`, `src/v3/device.rs`, `src/v3/resource_status.rs`, `src/v3/volume_state.rs`, `src/v3/payload_schema.rs`, `src/v3/error.rs`, `src/v3/host.rs`, `src/v3/bridge.rs`, `src/v3/limits.rs`|
|`d2b-contracts-resource-p2`|`src/v3/volume.rs`, `src/v3/process.rs`, `src/v3/identity.rs`, `src/v3/execution_policy.rs`, `src/v3/resource.rs`, `src/v3/storage.rs`, `src/v3/volume_binding.rs`, `src/v3/activation_nixos.rs`, `src/v3/user.rs`, `src/v3/mod.rs`, `src/v3/artifact.rs`, `src/lib.rs`|
|`d2b-core-p1`|`src/bundle_resolver.rs`|
|`d2b-core-p2`|`src/privileges.rs`, `src/host.rs`, `src/manifest_v04.rs`, `src/storage.rs`, `src/test_support.rs`, `src/console_ring.rs`, `src/allocator_config.rs`, `src/processes.rs`, `src/storage_lifecycle.rs`, `src/provider_artifact.rs`, `src/host_w3.rs`, `src/static_invariants.rs`, `src/sync.rs`, `src/runtime.rs`, `src/base64_codec.rs`, `src/sandbox_profile.rs`, `src/kernel_seat.rs`, `src/loader_worker.rs`, `src/site.rs`, `src/provider_capabilities.rs`, `src/bundle.rs`, `src/host_generation.rs`, `src/closures.rs`, `src/lib.rs`, `src/unsafe_local_workloads.rs`, `src/configured_argv.rs`, `src/contract_id.rs`, `src/error.rs`, `src/privileges_w3.rs`, `src/workload_identity.rs`|
|`d2b-provider-toolkit-p1`|`src/base/**`, `src/plane/**`, `src/operations/**`, `src/audit/**`, `src/declaration/**`, `src/bin/**`|
|`d2b-provider-toolkit-p2`|`src/testing/**`, `src/server/**`, `src/shared_provider.rs`, `src/credential.rs`, `src/service.rs`, `src/lib.rs`|
|`d2b-provider-display-wayland-p1`|`src/wayland_proxy/**`|
|`d2b-provider-display-wayland-p2`|`src/controller.rs`, `src/runtime.rs`, `src/process.rs`, `src/bin/**`, `src/spec.rs`, `src/policy.rs`, `src/session_children.rs`, `src/principal.rs`, `src/lib.rs`|
|`d2b-resource-runtime-p1`|`src/manager.rs`, `src/resource.rs`, `src/error.rs`, `src/provider.rs`, `src/metadata.rs`, `src/revision.rs`, `src/lib.rs`, `src/schema.rs`|
|`d2b-resource-runtime-p2`|`src/context.rs`, `src/target.rs`, `src/guest_target.rs`, `src/spec_store.rs`, `src/watch.rs`, `src/driver.rs`, `src/identity.rs`|
|`d2b-contracts-provider-p1`|`src/v3/provider.rs`, `src/v3/credential/**`, `src/v3/credential.rs`, `src/v3/provider_registry.rs`, `src/v3/mod.rs`, `src/lib.rs`|
|`d2b-contracts-provider-p2`|`src/v3/semantic_services/**`, `src/v3/credential_controller.rs`, `src/v3/telemetry_policy.rs`, `src/v3/telemetry_frame.rs`|
|`d2b-session-p1`|`src/driver.rs`, `src/server.rs`, `src/handshake.rs`, `src/operation.rs`, `src/streams.rs`, `src/scheduler.rs`, `src/cancellation.rs`, `src/fragmentation.rs`, `src/attachment.rs`, `src/metrics.rs`, `src/typed_stream.rs`|
|`d2b-session-p2`|`src/admission.rs`, `src/engine.rs`, `src/error.rs`, `src/client.rs`, `src/transport.rs`, `src/lifecycle.rs`, `src/record.rs`, `src/bootstrap.rs`, `src/deadline.rs`, `src/lib.rs`|
|`d2b-resource-api-p1`|`src/service.rs`, `src/adapter.rs`, `src/manager_backend.rs`, `src/client.rs`, `src/store.rs`, `src/watch.rs`|
|`d2b-resource-api-p2`|`src/authz.rs`, `src/manager_backend/**`, `src/admission.rs`, `src/error.rs`, `src/identity.rs`, `src/lib.rs`|
|`d2b-core-controller-p1`|`src/controller_assignment.rs`, `src/binding_children.rs`, `src/coordinator.rs`, `src/main.rs`, `src/controllers.rs`, `src/lib.rs`|
|`d2b-core-controller-p2`|`src/authority.rs`, `src/owner_reconcile.rs`, `src/authority_persistence.rs`, `src/migration.rs`|
|`d2b-provider-clipboard-wayland-p1`|`src/bin/**`, `src/fd.rs`, `src/runtime.rs`, `src/audit.rs`, `src/policy.rs`, `src/lib.rs`|
|`d2b-provider-clipboard-wayland-p2`|`src/clipd_host/**`, `src/service/**`, `src/history.rs`, `src/controller/**`, `src/picker.rs`|
|`d2b-contracts-zone-session-p1`|`src/v3/component_session.rs`, `src/v3/role.rs`, `src/v3/resource_export.rs`, `src/v3/zone_link.rs`, `src/v3/resource_import.rs`, `src/v3/mod.rs`, `src/lib.rs`|
|`d2b-contracts-zone-session-p2`|`src/v3/zone_routing.rs`, `src/v3/resource_bundle.rs`, `src/v3/zone_session.rs`, `src/v3/zone.rs`, `src/v3/role_binding.rs`, `src/v3/services.rs`, `src/v3/emergency_policy.rs`|

Single-part lanes (`d2b-host`, `d2b-provider-*`, ...): audit the whole crate
under `src/**` (excl. `src/generated/**`) plus `tests/**` for the `test` lens.

Partition notes: `d2bd/src/composition.rs` and `d2b-broker/src/runtime.rs` and
`xtask/src/provider_crate_policy.rs` are item-range splits of single oversized
files; their parts still count as one lane each (no `a`/`b` splits were needed).
Directories named with `/**` mean the whole subtree (e.g. `src/ops/**`), not the
bare file. `d2b-broker-p1`'s `state-posture-contract.json` entry under `d2b-broker-p7`
is a non-Rust data file inside `src/ops/`; skip it for seed runs.
