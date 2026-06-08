# 0017. No bash fallbacks invariant (v1.1)

- Status: Implemented in v1.1
- Date: 2026-05-31
- Wave: v1.1-P1 (landed)
- Plan slice: v1.1 §"v1.1-P1 — Eliminate `exec_legacy_passthrough` and every bash fallback path"
- Companion ADRs: [ADR 0007](0007-bash-coexistence-and-migration.md), [ADR 0010](0010-wire-protocol-and-typed-errors.md), [ADR 0015](0015-daemon-only-clean-break.md)
- Verification: `tests/no-bash-exec-eval.sh` (3 modes); commit `3c1c019`.

## Context

ADR 0015 declared a daemon-only end-state at v1.0 and deleted the
bulk of the bash CLI surface: the `cli.nix` Nix module
(`ph6-p6-cli-nix-migrations` + `ph6-remove-bash-cli`) and the
lifecycle-verb fallback bridge (`ph4-cli-up`) both retired in P4/P6.
The W14c bash fallback bridge and its env-knob escape hatches
(`NIXLING_LEGACY_BASH_OPT_IN`, `NIXLING_LEGACY_CLI`,
`NIXLING_NATIVE_ONLY`) all became no-ops or were removed.

Two residual surfaces survived into v1.0 as dead/stub code:

1. **`exec_legacy_passthrough(args, warning)` in
   `packages/nixling/src/lib.rs:3990`.** At v1.0 HEAD `00b24c5`
   this function no longer execs `/run/current-system/sw/bin/nixling-legacy`
   (the binary was deleted in P6); it emits a typed
   `not-yet-implemented` (exit 78) envelope and is therefore a
   stub. The function is still called from seven live sites:
   the early-dispatch `should_fallback_to_legacy` arm at
   `lib.rs:1414`, audit `--strict` at `lib.rs:1615`, audit
   `AuditSocketOutcome::Unreachable` at `lib.rs:1629`,
   `cmd_console` at `lib.rs:1643`, `cmd_audio` at `lib.rs:1651`,
   keys list `KeysSocketOutcome::Unavailable` at `lib.rs:2728`,
   and keys show `KeysSocketOutcome::Unavailable` at
   `lib.rs:2799`. The *intent* of "if X then exec bash" remains
   in the source even though no bash exec actually fires; the
   v1.1 invariant retires the residual intent and prevents
   reintroduction.
2. **`should_fallback_to_legacy(args)` in `packages/nixling/src/lib.rs:3897`.**
   A predicate used by the early-dispatch path (`lib.rs:1413`) to
   route certain unrecognized argv shapes to the stub above. Its
   allow-list was chiselled down across P0–P7 until the function
   returns `false` for every shape the v1.0 CLI declares native,
   but the function and the dispatch arm remain.

Both surfaces were retained through v1.0 as a deliberate "kill the
caller before the function" sequencing decision: tests landed first
to prove each call site emits typed envelopes correctly, then the
plumbing was scheduled for v1.1 removal.

This ADR records the v1.1 invariant: **the Rust CLI never invokes
bash, and never executes any legacy entrypoint**, enforced by a
workspace-wide CI gate.

## Decision

### Hard invariant

> The `nixling` Rust binary path never executes a bash sub-process
> and never execs any `nixling-legacy` entrypoint. Daemon-unreachable,
> broker-error, not-yet-implemented, and legacy-arg-shape conditions
> all surface typed envelopes per
> [ADR 0010](0010-wire-protocol-and-typed-errors.md) with exit codes
> per [`docs/reference/error-codes.md`](../reference/error-codes.md).
> No fallback sub-process is spawned. No env knob restores the
> behaviour. No documented escape hatch exists.

### Source-tree implications

The following are deleted wholesale in v1.1-P1 (no `#[cfg(legacy)]`
escape hatch, no deprecation warnings, no off-by-default fallback):

- `exec_legacy_passthrough` (`packages/nixling/src/lib.rs:3990`).
- `should_fallback_to_legacy`
  (`packages/nixling/src/lib.rs:3897`) and its early-dispatch arm
  at `lib.rs:1413`.
- `tests/cli-legacy-bash-dispatch.sh` (no longer applicable; the
  test asserted dispatch routing that no longer exists).
- The four `#[cfg(test)] mod tests` cases that assert
  `super::should_fallback_to_legacy` at `lib.rs:5018, 5027, 5034,
  5047`.

### Call-site conversion contract

Each retired call site emits a typed envelope per the table below.
Anchors in the docs-anchor column resolve to
[`docs/reference/error-codes.md`](../reference/error-codes.md);
exit codes are normative there.

| Source location (v1.0 HEAD `00b24c5`)                       | Function context           | Pre-v1.1 behaviour                            | v1.1 behaviour                                | Docs anchor              | Exit |
|-------------------------------------------------------------|----------------------------|-----------------------------------------------|-----------------------------------------------|--------------------------|------|
| `lib.rs:1413-1414` `should_fallback_to_legacy` early arm    | top-level dispatch         | exec_legacy_passthrough (stub → exit 78)      | parse error → typed `#usage` envelope         | `#usage`                 | 2    |
| `lib.rs:1615` audit `--strict` arm                          | `cmd_audit`                | exec_legacy_passthrough (stub → exit 78)      | typed `#not-yet-implemented` envelope         | `#not-yet-implemented`   | 78   |
| `lib.rs:1629` `AuditSocketOutcome::Unreachable`             | `cmd_audit`                | exec_legacy_passthrough with warning          | typed `#daemon-down` envelope                 | `#daemon-down`           | 1    |
| `lib.rs:1643` `exec_legacy_passthrough(original_args, None)` | `cmd_console`             | exec_legacy_passthrough (stub → exit 78)      | typed `#not-yet-implemented` envelope         | `#not-yet-implemented`   | 78   |
| `lib.rs:1651` `exec_legacy_passthrough(original_args, None)` | `cmd_audio`               | exec_legacy_passthrough (stub → exit 78)      | typed `#not-yet-implemented` envelope         | `#not-yet-implemented`   | 78   |
| `lib.rs:2728` `KeysSocketOutcome::Unavailable` (list)       | `cmd_keys` list arm        | exec_legacy_passthrough with warning          | typed `#daemon-down` envelope                 | `#daemon-down`           | 1    |
| `lib.rs:2799` `KeysSocketOutcome::Unavailable` (show)       | `cmd_keys` show arm        | exec_legacy_passthrough with warning          | typed `#daemon-down` envelope                 | `#daemon-down`           | 1    |

The `#not-yet-implemented` envelopes set `target_wave: "post-v1.1"` for
`cmd_console`, `cmd_audio`, and audit `--strict` because their
daemon-side implementation is NOT in the v1.1 phase plan (P1..P13)
— v1.0 only retired the bash fallback for the *unreachable* path,
and v1.1 lands the typed-envelope rendering + remediation contract
(per the call-site table above) but NOT the underlying verb
implementations. v1.1-P1 retires the residual offline-fallback
intent in source AND lands the Rust `Display` impl for the
multi-line `Remediation:` block format per
`docs/reference/error-codes.md` "Remediation rendering
conventions". The daemon-side implementation for `cmd_console`,
`cmd_audio`, and audit `--strict` is queued for a future release
(v1.2+ or later, currently unscheduled); there is no v1.1 P<N>
TDD row for the implementation work. If a future release
schedules them, this ADR's `target_wave` field should be
updated accordingly at that time. The v1.1 deliverable for
these verbs is the rendering + remediation contract only,
not the verb implementation.

The human-readable remediation for every `#daemon-down` and
`#not-yet-implemented` envelope on these verbs cross-links the
v1.1 migration guide
(`docs/how-to/migrate-nixling-v0-to-v1.md` — future, v1.1-P12)
so a v1.0 operator who relied on the silent-fail stub has a
single-click path to the behaviour-delta explanation. The
cross-link is **authoritatively wired through
`nixling_core::error::Error::remediation()`**: the v1.1-P1
implementation extends every typed envelope kind in the table
above to include the **full repository-relative guide path**
(`docs/how-to/migrate-nixling-v0-to-v1.md`, NOT just the
basename) in the `remediation` string for the `cmd_audit` /
`cmd_console` / `cmd_audio` / `cmd_keys` verbs. CLI contract
tests under
`packages/nixling/tests/cli_remediation_migration_guide.rs`
(future, v1.1-P1) assert that for each retired site:
- JSON output (`nixling ... --json` on these verbs with daemon
  down) contains the literal substring
  `docs/how-to/migrate-nixling-v0-to-v1.md` (the full path,
  not just the basename) in the `error.remediation` field; AND
- human output contains the same full-path substring in the
  rendered remediation hint AND the path appears on a
  **dedicated line** (not embedded in a wrapping prose
  paragraph) so terminals < 80 cols do not split the path mid-
  string. **The authoritative rendering format is defined in
  [`docs/reference/error-codes.md`](../reference/error-codes.md)
  "Remediation rendering conventions"** (resolves R22 software-r22-1
  — the v1.1-P0 ADR/reference cross-reference contract). That
  reference defines TWO format variants per the deferred-verb
  category split:
  - **Category 1 (truly deferred — `console`, `audio`,
    `audit --strict`)**: 5-line block emitting
    `This subcommand was queued for v1.2+ (unscheduled).` +
    `See the operator migration runbook:` + a
    repository-relative path on its own indented line +
    `Specifically the "<verb-specific anchor>" section.`
  - **Category 2 (daemon-down only — `audit` (non-strict),
    `keys list`, `keys show`)**: 6-line block emitting
    `nixlingd is not reachable. Start the daemon and re-run:`
    + two `sudo systemctl start ...` lines + `For full v1.0
    operator runbook context, see:` + repository-relative path
    on its own indented line + verb-specific anchor line.

  Both variants render the path on a **dedicated indented
  line** for copy-paste safety on terminals < 80 cols (the
  shared invariant). The example snippet below is a
  shortened illustrative form for this ADR's narrative
  flow; the canonical byte-for-byte format is the
  error-codes.md spec, and the v1.1-P1 contract tests +
  golden fixtures
  (`tests/golden/cli-output/audit-*-deferred.golden`,
  `console-deferred.golden`, `audio-deferred.golden`,
  `keys-deferred.golden`) lock the error-codes.md format,
  NOT the simplified illustrative form below.

  Illustrative form (full canonical format per error-codes.md):
  ```text
  Remediation:
    See migration guide:
      docs/how-to/migrate-nixling-v0-to-v1.md
  ```
  (path on its own indented line; copy-paste-safe on any
  terminal width). Golden fixture
  `tests/golden/cli-output/remediation-migration-guide-human.txt`
  pins the simplified format above for general migration-guide
  cross-link rendering; the per-verb goldens listed above pin
  the full per-category formats.

  **`Remediation:` prefix convention scope** (resolves R6
  product finding): the `Remediation:` block format with
  dedicated indented body lines is a **migration-guide-specific
  exception**, NOT a wholesale standardization of all CLI
  human-form remediation output. Other typed envelopes in v1.1
  (`#daemon-down`, `#broker-validation-failed`, `#internal-io`,
  etc.) preserve their existing inline/parenthesized remediation
  rendering for operator grep-stability. The migration-guide-
  specific block-format is documented in
  `docs/reference/error-codes.md` under a new "Remediation
  rendering conventions" subsection that v1.1-P1 lands; the
  subsection explicitly says: "Most envelopes use inline
  remediation; the migration-guide cross-link uses a multi-line
  block format to keep the path copy-paste-safe on narrow
  terminals." Operator grep patterns targeting other
  remediation strings remain valid; patterns specifically
  targeting the migration-guide hint should use the
  multi-line format documented here.

Asserting the full path (rather than the basename
`migrate-nixling-v0-to-v1.md`) ensures the operator can
copy-paste the string into a browser or `view` invocation
without manually prepending `docs/how-to/`.

### CI enforcement

A new eval gate `tests/no-bash-exec-eval.sh` (future, v1.1-P1)
enforces the invariant at every commit. The gate is *layered* —
syntactic grep is the first defence and an allowlist is the
authoritative one. Negative fixtures are stored as text-only
`.rs.fixture` files that the Rust compiler never sees, so the
exclusion-vs-coverage contradiction the R2 panel flagged does
not arise.

**Layer 1 — syntactic grep (fast).** Reject any of:

```
ripgrep -P 'Command::new\("(/bin/)?(ba)?sh"\)|spawn.*"/bin/sh"|/usr/bin/env\s+bash|Command::new\("nixling-legacy"\)|nixling-legacy\b|#\[path\s*=\s*"\.\./tests/fixtures/no-bash-exec/'
  --type rust
  --glob '!packages/*/tests/**'
  --glob '!packages/*/examples/**'
  --glob '!tests/fixtures/**'
```

This catches direct `Command::new("bash")` / `"sh"` /
`"nixling-legacy"` invocations (qualified `std::process::Command`
and `tokio::process::Command` included by `--type rust`), any
re-introduction of the legacy entrypoint by name, AND any
production `#[path = "../tests/fixtures/no-bash-exec/..."]`
attempt to `include!` a fixture into the binary crate (the
fixture-bypass attack the R2 security reviewer flagged).

**Layer 2 — Command::new allowlist (authoritative).** A
test-mode-only inventory check parses `cargo metadata --format-version 1`
to enumerate the binary-crate compile units (Layer 2 covers
`packages/nixling/src/`, including any `mod` files reached
transitively from `lib.rs`). The check then walks the call-site
inventory in three passes:

1. **Direct literal pass.** `rg 'Command::new\("..."\)'` across
   the binary crate, with each literal compared against
   `tests/fixtures/cli-process-allowlist.json`. New sites
   land allowlist-first.
2. **Alias / wrapper pass.** `rg 'use\s+(std|tokio)::process::Command(\s+as\s+\w+)?'`
   enumerates aliased imports; the test then ripgreps for
   `<Alias>::new(...)` per detected alias and runs them through
   the same allowlist. Wrapper functions named
   `spawn_<*>` / `exec_<*>` in the binary crate are listed in a
   dedicated `tests/fixtures/cli-spawn-wrappers-allowlist.json`;
   any new wrapper without an allowlist entry fails the test.

   **`cli-spawn-wrappers-allowlist.json` schema** (validated
   before the gate runs):
   ```json
   {
     "$schema": "http://json-schema.org/draft-07/schema#",
     "type": "object",
     "required": ["wrappers"],
     "properties": {
       "wrappers": {
         "type": "array",
         "items": {
           "type": "object",
           "required": ["name", "source_path", "allowed_callees", "rationale", "owner"],
           "properties": {
             "name": { "type": "string", "pattern": "^(spawn|exec)_[a-z_]+$" },
             "source_path": { "type": "string", "pattern": "^packages/nixling/src/.+\\.rs$" },
             "allowed_callees": {
               "type": "array",
               "items": { "type": "string" },
               "description": "Literal Command::new(...) targets this wrapper is allowed to invoke. Must be checked against cli-process-allowlist.json."
             },
             "rationale": { "type": "string", "minLength": 20 },
             "owner": { "type": "string", "pattern": "^(rust|virt|networking|security|kernel|software|test|product|docs)$" },
             "expires_at": { "type": "string", "format": "date" }
           },
           "additionalProperties": false
         }
       }
     },
     "additionalProperties": false
   }
   ```
   The test fails closed on (a) any wrapper in source without an
   allowlist entry, (b) any allowlist entry without a matching
   wrapper in source (dead-allowlist detection), (c) any
   duplicate `name` across entries.

3. **Direct-syscall pass.** `rg 'nix::unistd::execv|nix::unistd::execvp|libc::execve|libc::fork|libc::posix_spawn|libc::clone|libc::clone3'`
   across the binary crate. Any match fails Layer 2; the CLI is
   expected to use `std::process::Command` only.

The check fails if any site spawns a shell-like literal
(`bash`, `sh`, `dash`, `ksh`, `zsh`, `fish`,
`/usr/bin/env`-with-shell-arg, `${SHELL}` indirection via
`std::env::var("SHELL")`, or any path containing
`nixling-legacy`). The `legacy-bash` cargo feature is also
explicitly denied: `cargo metadata --format-version 1 |
jq '.packages[].features | keys[]'` must not include
`legacy-bash` on the binary crate.

Negative-fixture tests under `tests/fixtures/no-bash-exec/`
exercise the patterns the test + security reviewers flagged.
Fixtures are stored as `*.rs.fixture` (NOT `*.rs`) so the Rust
compiler never sees them. The gate is invoked as
`tests/no-bash-exec-eval.sh check` for normal CI runs and
`tests/no-bash-exec-eval.sh fixture-coverage` for the
negative-fixture coverage assertion:

- **Normal mode** (`tests/no-bash-exec-eval.sh check`): excludes
  `tests/fixtures/**` per the glob above. Asserts the production
  tree is clean against BOTH Layer 1 and Layer 2.
- **Fixture-coverage mode** (`tests/no-bash-exec-eval.sh fixture-coverage`):
  drops the fixture-tree exclusion and reads every `.rs.fixture`
  file as if it were `.rs`. Asserts each fixture is caught by the
  layer documented in the per-fixture coverage table below (NOT
  by both layers blanket — different attack classes are intentionally
  caught by different layers; the syntactic Layer 1 catches direct
  literals while Layer 2 catches indirection).

Fixture files committed under `tests/fixtures/no-bash-exec/`,
with explicit per-fixture expected-catch table:

| Fixture file (under `tests/fixtures/no-bash-exec/`)           | Indirection class                  | Layer 1 catches? | Layer 2 catches? | Expected gate output (substring in failure message)            |
|---------------------------------------------------------------|------------------------------------|-----------------|------------------|---------------------------------------------------------------|
| `command_new_bash.rs.fixture`                                 | direct literal                     | ✅              | ✅               | `Command::new("bash")`                                        |
| `command_new_slash_bin_sh.rs.fixture`                         | direct literal                     | ✅              | ✅               | `Command::new("/bin/sh")`                                     |
| `command_new_env_bash.rs.fixture`                             | env + bash arg                     | ❌              | ✅               | `Command::new("env")...arg("bash")` (Layer 2 arg inspection)  |
| `command_new_shell_var.rs.fixture`                            | env-var indirection                | ❌              | ✅               | `Command::new(std::env::var("SHELL"...))`                      |
| `command_new_const_sh.rs.fixture`                             | const-held literal                 | ✅ (`/bin/sh` literal still in source)  | ✅ (no allowlist entry) | `const SH: &str = "/bin/sh"` (Layer 1) + `Command::new(SH)` (Layer 2) |
| `command_new_nixling_legacy.rs.fixture`                       | legacy entrypoint reintroduction   | ✅              | ✅               | `nixling-legacy`                                              |
| `command_new_alias.rs.fixture`                                | aliased Command import             | ❌              | ✅               | `use std::process::Command as C; C::new("bash")` (Layer 2 alias pass) |
| `command_new_wrapper.rs.fixture`                              | wrapper function                   | ❌              | ✅               | `fn spawn_my(arg: &str)` not in `cli-spawn-wrappers-allowlist.json` |
| `tokio_command_new.rs.fixture`                                | tokio::process::Command            | ✅              | ✅               | `tokio::process::Command::new("bash")` (Layer 1 catches via `--type rust`) |
| `libc_execve.rs.fixture`                                      | direct execve syscall              | ❌              | ✅               | `libc::execve` (Layer 2 direct-syscall pass)                  |
| `libc_fork.rs.fixture`                                        | direct fork syscall                | ❌              | ✅               | `libc::fork` (Layer 2 direct-syscall pass)                    |
| `libc_posix_spawn.rs.fixture`                                 | direct posix_spawn syscall         | ❌              | ✅               | `libc::posix_spawn` (Layer 2 direct-syscall pass)             |
| `libc_clone.rs.fixture`                                       | direct clone syscall               | ❌              | ✅               | `libc::clone` (Layer 2 direct-syscall pass)                   |
| `nix_execv.rs.fixture`                                        | nix crate execv                    | ❌              | ✅               | `nix::unistd::execv` (Layer 2 direct-syscall pass)            |
| `production_include_bypass.rs.fixture`                        | `#[path]` production bypass        | ✅ (Layer 1 catches the attribute itself) | n/a (this attack would never compile into the binary if Layer 1 is in place) | `#[path = "../tests/fixtures/no-bash-exec/` (Layer 1) |
| `macro_command_new.rs.fixture`                                | declarative macro expansion        | ✅ (CI-gate syn AST walk on cargo-expanded output catches macro-emitted `Command::new` literals; see below) | ✅ (same — both layers run the same AST walk; both fail if the literal is shell-like AND no allowlist entry exists) | `macro_rules! spawn_shell` (or ANY declarative macro emitting `Command::new(<shell-like>)`) not in `cli-spawn-wrappers-allowlist.json` → fails |
| `macro_indirect_command_new.rs.fixture`                       | declarative macro w/ `concat!`-built target    | ✅ (AST walk fully expands `concat!()` and inspects the resulting string literal) | ✅ (same)        | `macro_rules! spawn_indirect { () => { Command::new(concat!("ba","sh")) } }` → caught after macro + concat expansion |

The `macro_command_new` and `macro_indirect_command_new`
fixtures address the R2 + R3 + R4 test/security reviewers'
concern that grep-based macro coverage can fail open if the
macro uses `concat!`, `format!`, or any other compile-time
string construction. v1.1-P1 closes this with a
**dedicated `tests/no-bash-exec-eval.sh syn-ast-walk` test
mode** (NOT a build.rs gate — the R5 test reviewer correctly
flagged that build.rs invoking `cargo expand` on the same crate
recursively re-runs the build script and is not a safe
compile-time gate). The new test-mode contract:

1. The eval gate has THREE invocations:
   - `tests/no-bash-exec-eval.sh check` — normal CI: Layer 1
     grep + Layer 2 direct-literal/alias/syscall passes on
     unmodified source.
   - `tests/no-bash-exec-eval.sh fixture-coverage` — negative
     fixture coverage (per-fixture expected-catch table).
   - `tests/no-bash-exec-eval.sh syn-ast-walk` — macro expansion
     pass (the formerly-proposed build.rs gate, moved to a
     dedicated test-mode invocation to avoid the build-script
     recursion).
2. The `syn-ast-walk` mode runs `cargo expand --bin nixling
   --no-default-features` ONCE as a separate subprocess (NOT
   re-entered from build.rs), captures the expanded source to a
   tempfile, and invokes a syn-based walker
   (`tests/tools/no-bash-ast-walker/Cargo.toml`, a small
   dev-only Rust binary with `syn = "2"` dependency) on the
   expanded output.
3. The walker visits every `syn::ExprCall` whose path resolves
   to `std::process::Command::new`, `tokio::process::Command::new`,
   or any alias declared via `use ... Command as <name>`. For
   each call, it evaluates constant expressions (literal strings,
   `concat!()` already collapsed by `cargo expand`, `const`-folded
   `&str` refs) using `syn::Expr::Lit` recursion; non-constant
   arguments (function args, runtime expressions) are checked
   against the wrapper allowlist (`cli-spawn-wrappers-allowlist.json`)
   — the wrapper MUST declare every literal target it forwards.
4. The fail-closed contract: any unresolved/dynamic
   `Command::new(<not-statically-determinable>)` whose enclosing
   function is NOT in the wrapper allowlist causes the eval gate
   to fail with a remediation pointer to this ADR.

**Toolchain provisioning** (resolves the R5 test reviewer's
"cargo expand subcommand not provisioned" finding): cargo-expand
is provisioned via the **flake-pinned** devShell that
`tests/no-bash-exec-eval.sh` invokes:

```
nix develop .#cargoExpandShell --command cargo expand --bin nixling --no-default-features
```

(uses `nix develop` not `nix shell` — `.#cargoExpandShell` is a
`devShells.<system>.cargoExpandShell` flake-output declared in
`flake.nix`, NOT a `packages.<system>.cargoExpandShell`. The R7
test reviewer correctly flagged this distinction). The devShell
spec landed in v1.1-P1 as the following `flake.nix` addition:

```nix
# flake.nix (v1.1-P1 addition; concrete shape)
outputs = { self, nixpkgs, rust-overlay, ... }:
  let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [ rust-overlay.overlays.default ];
    };
    stableRust = pkgs.rust-bin.stable."1.94.1".default;
    # Concrete nightly date pin — bumped only by panel-approved
    # `v11-PNfuM` commit. The exact date below MUST match
    # tests/no-bash-exec-eval.sh's `expected_nightly` assertion.
    nightlyRust = pkgs.rust-bin.nightly."2026-04-15".default;
  in {
    devShells.${system}.cargoExpandShell = pkgs.mkShell {
      nativeBuildInputs = [
        stableRust
        nightlyRust    # cargo-expand uses -Zunpretty=expanded
        pkgs.cargo-expand
      ];
      shellHook = ''
        export RUSTC_BOOTSTRAP=0
        # cargo-expand picks up `+nightly` from the nightlyRust above
      '';
    };
  };
```

The nightly date `2026-04-15` is normative — `flake.nix`
records the exact date; `flake.lock` pins the **`rust-overlay`**
flake input (NOT the `nixpkgs` flake input — rust-overlay is
what provides `pkgs.rust-bin.nightly."2026-04-15"`; per the
R8 test reviewer, the prior draft's "nixpkgs flake-input rev"
wording was incorrect). `tests/no-bash-exec-eval.sh` asserts:
- `rustc +nightly --version` output matches the expected
  string for the pinned nightly date (e.g., `rustc 1.96.0-nightly
  (... 2026-04-15)`); fail-fast on mismatch.
- The `rust-overlay` flake-input rev in `flake.lock` matches
  the expected rev recorded in this ADR (or in a
  panel-reviewed `flake-input-pins.json` fixture). Mismatches
  fail-fast.
**Panel updates to the nightly date** require a panel-approved
(rust + test + security) review of the new toolchain, AND a
corresponding update to the recorded rev in
`flake-input-pins.json`.

**CI invocation hardening** (resolves R6 security major). The
release-blocking CI invocation of the gates uses ONLY the
flake-pinned devShell with `--offline` (no network access during
gate execution) and explicitly forbids `--override-input` /
`--impure` Nix flags:

```
nix --offline --extra-experimental-features 'nix-command flakes' \
    develop .#cargoExpandShell --command \
    tests/no-bash-exec-eval.sh syn-ast-walk
```

The CI workflow definition (e.g., GitHub Actions YAML) lists
this exact invocation as the gate step. PR reviews MUST reject
any change that introduces `--override-input` or `--impure` to
the gate invocation; the workflow file itself is covered by
the CODEOWNERS-equivalent panel-review requirement for
release-blocking infra changes.

**Proc-macro escape (security R5 major).** The cargo-expand
output captures the post-expansion source for `macro_rules!`
(declarative macros) and ALSO for proc-macros from the crate's
direct dependencies. However, adversarial proc-macros are
arbitrary build-time Rust code and CAN emit different tokens
during `cargo expand` vs the real `cargo build` (e.g., by
checking `cfg!(macro_expanded_for_display)` if such a cfg
existed, or via timing/environment side channels). The walker
therefore CANNOT make a complete-coverage claim against
adversarial proc-macros. The hardening:

- **Proc-macro dependency allowlist.** The `nixling` binary
  crate's direct proc-macro dependencies are enumerated in
  `tests/fixtures/cli-proc-macro-allowlist.json`. Every entry
  has the form
  `{name, version-req, source-rev-hash, registry-url, owner,
  audit-notes}`. Adding or upgrading a proc-macro dep requires
  panel-level (rust + security) review before the entry lands.
  **`registry-url`** is normative: each allowlist entry pins the
  exact source URL the dependency MUST come from (e.g.,
  `https://github.com/rust-lang/crates.io-index` for crates.io,
  or a specific git URL for git deps). The gate parses
  `Cargo.lock` to cross-reference the actual source URL of every
  proc-macro dep against the allowlist's `registry-url` field;
  a hostile `~/.cargo/config.toml` source replacement would
  divert downloads but cannot change `Cargo.lock`'s recorded
  source URL (which the gate trusts as the canonical mapping).
  **Committed `.cargo/config.toml` source replacement** at the
  workspace level is also denied: Cargo source replacement is
  configured in `.cargo/config.toml` files (NOT in `Cargo.toml`
  package manifests — the R10 security reviewer flagged the
  prior incorrect spec). Workspace `.cargo/config.toml` is read
  by Cargo even with a sanitized `CARGO_HOME` (the
  per-test-run `CARGO_HOME` strips host-level config, but the
  in-tree workspace `.cargo/config*` is still in scope). The
  gate ripgrep-greps every committed file matching
  `**/.cargo/config{,.toml}` for any `[source.*]` block or
  `replace-with =` directive and fails-closed if any are
  present in the v1.1 baseline (no source replacement is
  legitimate in the v1.1 nixling workspace; legitimate future
  source replacements would need panel review + an explicit
  allowlist entry that does not exist in the v1.1 baseline).
  This is enforced in addition to the `Cargo.lock` registry-url
  cross-reference above; the two checks together close the
  in-tree-config + Cargo.lock-divergence attack surface.
- **`tests/proc-macro-allowlist-eval.sh` runs in a sanitized
  Cargo environment.** The script invocation explicitly
  unsets `CARGO_HOME`, `CARGO_CONFIG`, `CARGO_NET_OFFLINE`,
  `CARGO_NET_GIT_FETCH_WITH_CLI` and sets `CARGO_HOME` to a
  per-test-run tempdir to neutralize any host-level Cargo
  config that could redirect registry sources. The flake-pinned
  devShell guarantees the rustc/cargo binaries themselves come
  from the locked nixpkgs input, not the host's `$PATH`.
- A new test `tests/proc-macro-allowlist-eval.sh` (future,
  v1.1-P1) parses `cargo metadata --format-version 1` for the
  `nixling` binary crate in the sanitized environment AND
  parses `Cargo.lock` for the per-dep `source` field. The
  proc-macro identification MUST use `cargo metadata.resolve`
  graph traversal: for each package in the closure reachable
  from the `nixling` binary crate root (including transitive
  deps such as `clap_derive` pulled in via `clap`), the gate
  inspects the package's `targets[]` array and selects every
  package whose `targets[].kind` array contains `"proc-macro"`
  (Cargo metadata represents proc-macros as a target kind on
  the package, NOT as a dependency `kind` — the prior draft
  saying `dependencies[].kind == "proc-macro"` was incorrect
  per R10 rust-r10-1 and would fail open or be unimplementable
  as written). Each identified proc-macro package is then
  matched against the allowlist by name + version-req +
  source-rev-hash + registry-url. Unallowlisted proc-macros
  fail the gate. The gate also verifies the `resolve` graph
  closure is fully reachable from the `nixling` binary crate
  root (a proc-macro pulled in only via `dev-dependencies` of
  a transitive dep is in-scope iff Cargo's resolver promotes
  it into the binary crate's build graph; the gate uses
  `cargo metadata --filter-platform=<host>` with
  `--manifest-path=packages/nixling/Cargo.toml` to scope to
  the binary crate's resolved closure).
- **AST walker source review** (resolves R7 security major).
  The `tests/tools/no-bash-ast-walker/` Cargo crate is exempt
  from the no-bash gate's path scanning (it's a dev tool, not
  the binary crate). To prevent the exemption from creating an
  unaudited backdoor, every change under `tests/tools/no-bash-ast-walker/`
  requires **panel-level rust + security review** AT COMMIT
  TIME (not only at allowlist-edit time). v1.1-P1 lands an
  explicit `.github/CODEOWNERS` (or equivalent) entry:
  ```
  tests/tools/no-bash-ast-walker/    @nixling-panel-rust @nixling-panel-security
  ```
  This makes the walker source as audit-controlled as the
  allowlist itself — any modification triggers the two
  discipline reviewers' approval at PR-merge time.
- The combination (cargo-expand walk + proc-macro allowlist
  with registry-url + sanitized environment + `.cargo/config*`
  `[source]` denial via the gate's repo-config scan +
  CI invocation hardening + walker source CODEOWNERS) closes
  the practical attack surface: declarative
  macros are covered by the AST walk (provably); proc-macros
  are covered by the allowlist (governance over what crates
  can ship a spawn-emitting proc-macro) AND the sanitized
  environment (no source-replacement attack — committed
  `.cargo/config.toml` source replacements are scanned for and
  rejected; the prior summary saying "Cargo.toml [source] denial"
  was incorrect per R10 security-r10-1 and R11 security-r11-1:
  Cargo source replacement is declared in `.cargo/config.toml`,
  not in package `Cargo.toml` manifests); the walker
  itself is covered by the CODEOWNERS panel review. The
  remaining gap — a malicious allowlisted proc-macro that
  hides spawn calls from `cargo expand` — is mitigated by panel
  review at allowlist edit time, and acknowledged as a residual
  risk in the v1.1 threat model. Operators who want zero
  residual risk can patch their consumer flake to deny ALL
  proc-macros (set `cli-proc-macro-allowlist.json` to empty);
  the build then fails with a clear "proc-macro denied" error.

**Dev-tool exempt-path allowlist file location** (resolves
R7 test minor). The `dev_tools_exempt_paths` array was
previously documented as a sibling field in
`cli-proc-macro-allowlist.json`; the R7 test reviewer
correctly observed that no-bash path exemptions are not a
proc-macro governance concern. v1.1-P1 lands the exempt-path
list in a **dedicated** fixture file:
`tests/fixtures/no-bash-exec-exempt-paths.json` (NOT in the
proc-macro file, NOT in the wrapper file). Schema:

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "required": ["exempt_paths"],
  "properties": {
    "exempt_paths": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["path", "rationale", "owner_discipline", "review_required_on_change"],
        "properties": {
          "path":     { "type": "string", "pattern": "^tests/tools/.+/$" },
          "rationale": { "type": "string", "minLength": 50 },
          "owner_discipline": { "type": "string",
            "enum": ["rust","virt","kernel","security","networking","software","test","product","docs"] },
          "review_required_on_change": { "type": "boolean", "const": true }
        },
        "additionalProperties": false
      }
    }
  },
  "additionalProperties": false
}
```

The `review_required_on_change: true` field is normative —
every entry MUST set it; the CODEOWNERS gate cross-references
the entries to ensure the per-path panel review is enforced.
`tests/no-bash-exec-eval.sh` validates the exempt-path file
end-to-end (resolves R8 security minor + R8 test minor):

1. **Schema validation**: parses
   `tests/fixtures/no-bash-exec-exempt-paths.json` against
   the JSON Schema above; fails on any unknown field,
   missing required field, or invalid pattern match.
2. **`review_required_on_change` value assertion**: asserts
   EVERY entry has the literal value `true` for
   `review_required_on_change`. A malicious edit setting
   the field to `false` is caught here, INDEPENDENT of
   CODEOWNERS enforcement.
3. **Stale-entry detection**: each `path` in the exempt list
   MUST correspond to an existing directory under
   `tests/tools/`. An entry without a matching directory
   fails (dead-entry detection).
4. **Missing-exemption detection**: each directory under
   `tests/tools/` that is intentionally NOT in the exempt
   list (e.g., a future dev tool that should NOT be exempt)
   is scanned by the normal no-bash gate. Directories that
   SHOULD be exempt but are missing from the list fail the
   normal gate (which is the desired signal: add the
   exemption with a panel-approved rationale).
The `macro_rules!`-name grep fallback documented in the R3
draft is REMOVED — it was vulnerable to false positives on
non-spawning macros (e.g., `spawn_log!`) and false negatives on
non-spawn-named macros (e.g., `cmd_factory!`) per the R4
security review. The `syn-ast-walk` test-mode AST walk (NOT a
build.rs gate per the R5 test reviewer's recursion concern) is
fail-closed and covers both cases by inspecting actual call
expressions, not macro names.

The gate is wired into `tests/README.md` under the eval-gate
inventory and runs as part of `cargo xtask test`. **All
no-bash-exec invocations** (`check`, `fixture-coverage`,
`syn-ast-walk`), the proc-macro allowlist eval
(`tests/proc-macro-allowlist-eval.sh`), AND the dev-tool
proc-macro allowlist eval
(`tests/dev-tool-proc-macro-allowlist-eval.sh`) are
**release-blocking CI steps** — they MUST be green before any
merge to `phase-daemon-only` or release tag.

**Sandbox-guard scope** (resolves R12 test-r12-2): only the
three `no-bash-exec-eval` modes (`check` / `fixture-coverage`
/ `syn-ast-walk`) MUST be invoked through
`tests/no-bash-exec-eval-sandbox-guard.sh` because the
`syn-ast-walk` mode invokes `cargo expand` which runs every
dependency's `build.rs` and proc-macro code under Cargo's
control (the threat model addressed by the sandbox per the
"Cargo build.rs sandboxing" subsection above). The
proc-macro / dev-tool allowlist gates
(`proc-macro-allowlist-eval.sh` and
`dev-tool-proc-macro-allowlist-eval.sh`) only parse
`cargo metadata` + `Cargo.lock` JSON; they do NOT invoke
`cargo expand` or execute any build.rs / proc-macro code,
so the sandbox is not required for them and direct invocation
is correct. The required CI invocation order is:

1. `tests/proc-macro-allowlist-eval.sh` — direct invocation
   (no sandbox needed; metadata + lock parsing only). Runs
   FIRST so an unallowlisted production proc-macro fails
   fast without invoking cargo-expand.
2. `tests/dev-tool-proc-macro-allowlist-eval.sh` — direct
   invocation (same reasoning; metadata + lock parsing for
   `tests/tools/no-bash-ast-walker/` and, in v1.1-P10+,
   `tests/tools/baseline-exception-validator/`).
3. `tests/no-bash-exec-eval-sandbox-guard.sh check` — wraps
   `tests/no-bash-exec-eval.sh check` (Layer 1 grep + Layer 2
   direct/alias/syscall passes on raw source). Note: this
   mode does NOT invoke cargo-expand but the guard wrapper
   is required so step-3 / step-4 / step-5 all share one
   invocation pattern (the CI workflow definition rejects
   any direct invocation of `no-bash-exec-eval.sh`).
4. `tests/no-bash-exec-eval-sandbox-guard.sh syn-ast-walk`
   (wraps `tests/no-bash-exec-eval.sh syn-ast-walk`;
   cargo-expand + syn AST walk under the sandbox — the
   sandbox is structurally required HERE because of
   cargo-expand).
5. `tests/no-bash-exec-eval-sandbox-guard.sh fixture-coverage`
   (wraps `tests/no-bash-exec-eval.sh fixture-coverage`;
   negative fixture assertions, last because slowest).

The `tests/README.md` eval-gate inventory documents this
ordering. PR merges that bypass the ordering (e.g., by
invoking only `check` and not `syn-ast-walk`, or by
invoking `tests/no-bash-exec-eval.sh` directly without the
sandbox guard wrapper) are explicitly disallowed by the
`.github/workflows/no-bash-gate.yml` CI workflow definition
that lands in v1.1-P1 (per the plan.md P1 panel scope
addition for `.github/workflows/`).

The `tests/tools/no-bash-ast-walker/` Cargo crate (the small
dev-only binary the `syn-ast-walk` mode invokes) is
**exempt** from the no-bash gate via an entry in the dedicated
`tests/fixtures/no-bash-exec-exempt-paths.json` file (per the
schema documented in the "Dev-tool exempt-path allowlist file
location" subsection above — NOT in `cli-proc-macro-allowlist.json`,
which is a separate proc-macro governance concern).

The sibling exemption for
`tests/tools/baseline-exception-validator/` (the audit-baseline-
exception YAML validator per ADR 0018) is **deferred to v1.1-P10**:
the validator crate is created in P10 alongside the
`tests/fixtures/broker-spawn-audit-baseline-exceptions.yaml`
fixture it validates, so the exempt-path entry is added in P10's
`no-bash-exec-exempt-paths.json` update (NOT in v1.1-P1). The
R9 test reviewer flagged keeping the exemption in P1 as a
stale-entry failure: ADR 0017's exempt-path stale-entry detection
would fail at P1 because the directory does not yet exist.

Both walker crates' source is governed by **commit-time
panel-level rust + security review** via `.github/CODEOWNERS`
(or equivalent) — this is stricter than the
allowlist-edit-time review documented in earlier drafts; any
modification under `tests/tools/no-bash-ast-walker/` (and
later, under `tests/tools/baseline-exception-validator/` once
landed in v1.1-P10) triggers the two
discipline reviewers' approval at PR-merge time. The
`.github/CODEOWNERS` entries land in v1.1-P1 and v1.1-P10
respectively:

```
# v1.1-P1
tests/tools/no-bash-ast-walker/             @nixling-panel-rust @nixling-panel-security
tests/fixtures/cli-process-allowlist.json   @nixling-panel-rust @nixling-panel-security
tests/fixtures/cli-proc-macro-allowlist.json    @nixling-panel-rust @nixling-panel-security
tests/fixtures/cli-spawn-wrappers-allowlist.json @nixling-panel-rust @nixling-panel-security
tests/fixtures/no-bash-exec-exempt-paths.json @nixling-panel-rust @nixling-panel-security

# v1.1-P10 (added when baseline-exception-validator crate lands)
tests/tools/baseline-exception-validator/   @nixling-panel-rust @nixling-panel-security
tests/fixtures/broker-spawn-audit-baseline-exceptions.yaml @nixling-panel-rust @nixling-panel-test
```

The combined coverage (commit-time CODEOWNERS for walker
source + allowlist files; rust+security panel for any change)
closes the recursion of "the gate verifying itself" — both
walkers (once the second one lands) cannot weaken the gate
without panel approval at PR-merge time.

**Dev-tool dependency governance** (resolves R8 security major).
At v1.1-P1, `tests/tools/no-bash-ast-walker/` has its own
Cargo.toml dependencies (syn, etc.); at v1.1-P10
`tests/tools/baseline-exception-validator/` will land and add
its own dependencies (serde_yaml, etc.). These dev tools are
NOT governed by the production-crate
`cli-proc-macro-allowlist.json` (which is scoped to the
`nixling` binary crate's proc-macro deps). Instead, each dev
tool declares its proc-macro/build-script dependencies in the
**sibling** allowlist `tests/fixtures/dev-tool-proc-macro-allowlist.json`
with the same schema as the production allowlist (name,
version-req, source-rev-hash, registry-url, owner,
audit-notes). The gate
`tests/dev-tool-proc-macro-allowlist-eval.sh` (future,
v1.1-P1) cross-references each dev-tool crate's
`cargo metadata` output against this allowlist. Any
unallowlisted proc-macro or build-script dep in a dev tool
fails the gate. The dev-tool allowlist is itself
CODEOWNERS-protected (same panel as the production
allowlist). When the baseline-exception-validator lands in
v1.1-P10 its deps are appended to the dev-tool allowlist as
part of that phase's panel review.

**Cargo build.rs sandboxing** (resolves R8 security major; tightened
in R9 per security-r9-1). The `cargo expand` invocation in
`syn-ast-walk` mode runs the crate's full build graph, which
executes EVERY dependency's `build.rs` script AND every proc-macro
under Cargo control. A malicious dep's build.rs or proc-macro
could write to the filesystem, open network sockets, or read
ambient CI secrets (Cargo `--offline` only prevents Cargo's own
fetches; arbitrary `connect(2)` from build.rs is unaffected). The
v1.1 sandbox is **fail-closed** on three axes — network isolation,
environment whitelisting, and CI assertion that the controls
were applied:

- **Network isolation** — the cargo-expand process MUST run inside
  a `bwrap --unshare-net` (or `nsjail` with an actual network-
  namespace drop: `nsjail --disable_clone_newnet` is INCORRECT
  because that flag DISABLES the namespace; the correct nsjail
  posture is to let the default network-namespace dropping behave
  AND assert it via the nsjail config flag `mode: ONCE` + setting
  `clone_newnet: true` in the nsjail config — i.e., explicit
  request for a fresh, empty network namespace). The earlier
  draft listed `--disable_no_new_privs` as an "equivalent" which
  was incorrect: that is the seccomp/no-new-privs flag and has
  no bearing on network isolation; the R10 security reviewer
  flagged this. **The CI guard MUST assert actual network-
  namespace creation** (e.g., by reading
  `readlink /proc/$cargo_expand_pid/ns/net` and asserting it
  differs from the host's net-namespace inode). `--offline` is
  treated as a defense-in-depth flag, NOT the primary network
  control. Any `connect(2)` from build.rs or proc-macros will
  see ENETUNREACH; CI secrets cannot be exfiltrated even if read.
- **Environment whitelisting** — the cargo-expand process MUST be
  launched via `env -i` with an **explicit whitelist** of
  required vars: `PATH` (set to a sandbox-internal value pointing
  at the flake-pinned toolchain), `HOME=/sandbox-home`,
  `CARGO_HOME` (set to the per-test-run tempdir),
  `CARGO_TARGET_DIR` (scratch tmpfs), `RUSTUP_TOOLCHAIN`
  (matching the `.#cargoExpandShell` nightly pin), and the
  `RUSTC_BOOTSTRAP=1` / `CARGO_BUILD_RUSTC_WRAPPER=` empty-
  override pair documented elsewhere. CI secrets (`GITHUB_TOKEN`,
  `CARGO_REGISTRIES_*_TOKEN`, `SSH_AUTH_SOCK`, etc.) MUST NOT be
  in the whitelist; `env -i` with the whitelist strips them.
- **Filesystem confinement** — the workspace is `bwrap --ro-bind`
  read-only; `target/` and `CARGO_TARGET_DIR` are a tmpfs;
  `/tmp` and `/home` are tmpfs; build scripts can only write to
  scratch.
- **CI assertion** — the gate invocation MUST be wrapped by a CI
  guard script `tests/no-bash-exec-eval-sandbox-guard.sh` (lands
  in v1.1-P1 alongside the gate). The guard `set -e` asserts: (a)
  the `bwrap`/`nsjail` argv contains `--unshare-net` (or the
  nsjail `clone_newnet: true` config setting requesting a fresh
  net namespace; the guard MUST NOT accept `--disable_no_new_privs`
  or any other flag that is not strictly a net-namespace control)
  AND additionally asserts at runtime that the launched
  cargo-expand process's `/proc/<pid>/ns/net` symlink target is
  a different inode from the host's `/proc/self/ns/net` (proving
  the netns was actually created — the R10 security reviewer
  required asserting network isolation directly, not merely
  asserting the requesting flag was passed); (b) the wrapping
  shell-prefix is `env -i ...` with the documented whitelist
  (and NO entry containing `TOKEN`, `SECRET`, `SSH_`, or `_KEY`);
  (c) `CARGO_HOME` points at a path created via `mktemp -d` in
  the same script. If any check fails the guard exits non-zero
  BEFORE the cargo-expand process runs (for static checks a + b
  + c) or kills the process and fails the gate (for the runtime
  netns inode comparison); CI MUST invoke the gate exclusively
  via the guard. Direct invocation of the gate without the
  guard is disallowed by the CI workflow definition. The
  `--offline` flag remains a defense-in-depth addition but is
  NOT the authoritative network control.
- **Residual risk**: a build.rs that performs purely in-memory
  computation can still influence the expanded output. This is
  acknowledged as a residual risk in the v1.1 threat model;
  the proc-macro allowlist's panel-review at edit time is the
  primary defence for the dep set. With network unshared and
  the env stripped, exfiltration vectors are closed.

## Alternatives considered

### A1. Soft deprecation (warn-then-remove)

Emit a runtime warning every time `exec_legacy_passthrough` is
invoked in v1.1, schedule removal for v1.2.

**Rejected** because:
- At v1.0 HEAD `00b24c5` `exec_legacy_passthrough` is already a
  typed-envelope-emitting stub (no longer execs the deleted
  `nixling-legacy` binary). The function name and call shape are
  the only remaining "intent" surface; emitting an additional
  runtime warning on top of the existing typed envelope adds noise
  without changing operator behaviour.
- The v1.0 → v1.1 boundary is the natural moment for the source
  cleanup; carrying dead code through v1.1 reopens the audit
  surface for no operator benefit.

### A2. Retain function as a compile-time gate

Keep `exec_legacy_passthrough` defined but mark it `#[deprecated]`
or `#[cfg(feature = "legacy-bash")]`, off by default.

**Rejected** primarily on operator/support grounds; the CI-grep
concern is secondary:
- *Operator/support.* A "legacy-bash" feature in `cargo metadata`
  output advertises a supported escape hatch to consumer flakes,
  which contradicts ADR 0015's daemon-only contract and the v1.1
  invariant above. Support threads would re-acquire the "which
  build feature is on?" axis.
- *CI surface.* A `#[cfg(feature = "legacy-bash")]` body is
  invisible to a syntactic grep but IS visible to the Layer 2
  allowlist gate (which parses `cargo metadata --format-version 1`).
  An adequate gate could enforce "no `legacy-bash` feature defined
  anywhere in `[features]`"; the operator/support concern above is
  the deciding factor, not the CI gate cost.
- *Cargo features have global effects on `cargo build` consumers;
  even a flag-off-by-default surfaces in `cargo metadata` and
  `cargo tree` output.

### A3. Escape hatch via env knob

Keep `NIXLING_LEGACY_BASH_OPT_IN` honoured as a "support break-glass"
toggle.

**Rejected** because:
- ADR 0015 already retired the env knob as a no-op in v1.0.
- Any retained escape hatch reopens the two-writer hazard ADR 0015
  closed: bash entrypoints can race the daemon for `/run/nixling`
  state.
- Support break-glass is served by the typed envelopes themselves
  (`daemon-down` envelope tells the operator exactly what to do —
  start the daemon).

## Consequences

### Positive

- **One audit surface.** Code review and threat modeling enumerate
  one binary's syscall surface; no "what does the CLI do if it
  decides to shell out" branch.
- **Deterministic exit codes.** Every non-success path returns a
  documented exit code from the [ADR 0010](0010-wire-protocol-and-typed-errors.md)
  table. No `bash` exit-code passthrough (where the legacy script's
  exit code would bleed through unmapped).
- **No silent-fail regression risk.** A future commit that re-adds
  `Command::new("bash")` fails CI before merge.
- **Smaller binary surface.** Removing the dead function + dispatch
  arm trims ~120 lines of Rust including the four `#[cfg(test)]`
  unit tests that pinned the dispatch shape.

### Negative

- **Operator surface change for the deferred-verb set when
  daemon is down.** v1.0 emitted a typed `not-yet-implemented`
  envelope from the stub for ALL five sites (`audit --strict`,
  `console`, `audio`, `keys list`, `keys show`). v1.1
  differentiates per the call-site table above and
  [`cli-contract.md`](../reference/cli-contract.md):
  - **Truly deferred verbs (`audit --strict`, `console`,
    `audio`)**: v1.1 retains the typed `#not-yet-implemented`
    envelope (exit 78) unconditionally; daemon-state-independent.
  - **Daemon-backed verbs (`keys list`, `keys show`, and `audit`
    without `--strict`)**: v1.1 emits the typed `#daemon-down`
    envelope (exit 1) ONLY when the broker is stopped; the v1.0
    successful-call path is unchanged when the daemon is up.
  Both categories' human-form remediation now cross-links the
  v0→v1 migration guide
  (`docs/how-to/migrate-nixling-v0-to-v1.md`, which includes
  the "v1.1 deferred verbs and daemon-down rendering pointers"
  section landed in v1.1-P0) so an operator who relied on the
  silent-fail bash fallback (which actually failed `ENOENT` for
  `nixling-legacy` even in v1.0; many operators may not have
  noticed) has a discoverable path to the behaviour-delta
  explanation.
- **No fallback for daemon-side bugs.** If the daemon crashes
  uncleanly in a future v1.x release and the operator wants to
  recover via bash, no such path exists. Recovery is via daemon
  restart. The exact runbook command
  (`systemctl restart nixlingd nixling-priv-broker.service nixling-priv-broker.socket`)
  is added to `docs/how-to/migrate-nixos-to-daemon.md` as part of
  v1.1-P4 (broker NixOS module), which is where the broker
  service unit becomes a first-class operator surface. Until P4
  lands, the v1.0 manual-spawn workaround documented in the side
  task remains the operator path.

### Neutral

- **ADR 0007 stays superseded.** ADR 0015 already supersedes ADR
  0007's coexistence path; ADR 0017 extends that supersession to
  cover the residual passthrough plumbing.
- **No CHANGELOG schema change.** The v1.1 CHANGELOG section
  records the removal under "Retired from v1.0 deferral list" with
  a bullet pointing here.

## Verification

- `tests/no-bash-exec-eval.sh` (future, v1.1-P1) enforces the
  invariant via the layered Layer 1 + Layer 2 gate described in
  the CI enforcement section. The two layers must both be green
  for a commit to be accepted.
- [`tests/legacy-unit-denylist-eval.sh`](../../tests/legacy-unit-denylist-eval.sh)
  (existing) continues to pass.
- `cargo test --lib` for `nixling` continues to pass after the
  dead-code removal (38 tests at v1.0 HEAD `00b24c5` per
  `cargo test --lib --package nixling`; the four
  `should_fallback_to_legacy` assertion tests are removed,
  matching coverage of the new envelope path is added in their
  place).
- Manual operator checks (use a non-strict `audit` to exercise the
  daemon-down arm; `audit --strict` returns from the early arm
  BEFORE socket probing per `lib.rs:1614-1616` and so does not
  reach the `Unreachable` envelope):
  - `systemctl stop nixlingd nixling-priv-broker.socket nixling-priv-broker.service`
    then `nixling audit` (no `--strict`) returns the typed
    `#daemon-down` envelope (exit 1) without any `execve` of
    `bash` / `nixling-legacy` in `strace -f`.
  - `systemctl stop nixlingd nixling-priv-broker.socket nixling-priv-broker.service`
    then `nixling audit --strict` returns the typed
    `#not-yet-implemented` envelope (exit 78); the early-strict
    arm is unaffected by daemon state.
  - `systemctl stop nixlingd nixling-priv-broker.socket nixling-priv-broker.service`
    then `nixling keys list` and `nixling keys show <id>` each
    return `#daemon-down` (exit 1).
  - `nixling console <vm>` and `nixling audio <vm>` each return
    `#not-yet-implemented` (exit 78) regardless of daemon state
    (these have no daemon-side implementation in v1.1; they
    surface the typed envelope unconditionally).
