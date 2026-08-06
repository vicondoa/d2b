# Make Target Compatibility Contract

## Shadow stage

Cargo remains authoritative for `make test-rust` and the eight existing leaf
names. The Bazel shadow adds:

```text
make test-bazel-rust
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
make bazel-shutdown
```

The aggregate covers exactly eighteen IDs. Slice targets cover only their
mapped rows. Workflows call approved Make targets, never Bazel directly.

All six shadow names enter `APPROVED_MAKE_TARGETS` in
`packages/xtask/tests/policy_ci.rs` in the same wave that introduces them,
owned by one exact spec003w1 slice. The allowlist change carries:

- a positive test that each of the six approved shadow names resolves to a real
  Makefile rule and that a workflow step calling it is accepted by the
  ci-uses-make guard;
- a negative test that a workflow step calling an unapproved
  `test-bazel-rust-<name>` is rejected;
- a negative test that an approved shadow name with no Makefile rule is
  rejected.

The Makefile-rule assertion is written test-first and is red until the
integrator adds the six entry points; it must be green on the integrated
candidate.

`D2B_EXECUTION_MANIFEST` and `D2B_RUST_BUDGET` keep their existing meanings.

## Contributor-only commands

After entering `nix develop` at repository root and `cd packages`:

```text
cargo xtask bazel-repin --hub product
cargo xtask bazel-repin --hub walker
cargo xtask bazel-module-refresh
cargo xtask bazel-yanked-refresh
cargo xtask bazel-yanked-check
cargo xtask gen-package-policy-inputs
cargo xtask gen-package-policy-inputs --check
cargo xtask bazel-evidence prepare-cold-local
cargo xtask bazel-qualification-validate
cargo xtask bazel-promotion-record-validate
cargo generate-lockfile --offline
cargo generate-lockfile --offline --manifest-path ../tests/tools/no-bash-ast-walker/Cargo.toml
```

None is a Make target. No workflow names one. Workflows reach offline checks
only through the approved carrier targets that own them.

`cargo xtask bazel-qualification-validate` takes no arguments, reads the
fixed repository-relative qualification record path, derives every threshold
from the record's immutable evidence references, and refuses omitted, forged,
duplicate, inconsistent, and wrong-candidate references. It is not a Make
target and no workflow names it.

`cargo xtask bazel-promotion-record-validate` also takes no arguments and is
unreachable from Make and workflows. After promotion merges, it binds the
fixed promotion record to the actual protected-`v3` pull-request merge commit
and the sealed `spec003w5` delivery identities. Alias-removal and
Cargo-retirement entry run it before consulting their own eligibility
evidence.

`main`, `broker`, and `guest` repin identifiers are retired and fail with the
exact product remediation contract before Bazel starts.

`cargo xtask bazel-module-refresh` takes no arguments, runs the measured
`bazel mod deps --lockfile_mode=update` child with the repository's absolute
startup options, permits only `MODULE.bazel.lock` to change, and is idempotent.
Module drift uses the `D2B-BZLDRIFT-MODULE` row from
`workspace-and-tool-pinning.md`, including the exact ordered sequence:

```text
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-module-refresh
Review and commit MODULE.bazel.lock.
Rerun cargo xtask bazel-module-refresh, then rerun the failed command.
```

Policy tests reject every contributor-only command through direct recipe text,
variable indirection, helper calls, generated workflow steps, and post steps.
There is no Make forwarding target for product-lock generation, module refresh,
hub repin, yanked refresh/check, policy generation, or evidence mutation.

## Promotion

- Required context stays `test-rust`.
- `make test-rust` routes eighteen surfaces through Bazel and retains the
  Cargo and Nix fixture path.
- Generated CI calls exactly:

  ```text
  make test-rust-slice-main
  make test-rust-slice-api
  make test-rust-slice-broker
  make test-rust-slice-aux
  ```

- All eight public leaf names remain with their existing semantics:
  `test-rust-api-surface`, `test-rust-main`, `test-rust-broker`,
  `test-rust-guest-shell-runner`, `test-rust-no-bash-ast`,
  `test-rust-schema`, `test-rust-inventory`, and
  `test-rust-supply-chain`.
- `test-rust-main` retains its current conditional fixture/CLI behavior.
- Each public leaf maps to the exact carrier subset in `coverage-map.md`; it is
  not an alias for a broader slice.
- Bazel-specific names become status-preserving forwarding aliases with these
  exact mappings and exact stderr lines:

  ```text
  test-bazel-rust -> test-rust
  make: test-bazel-rust is deprecated; use test-rust

  test-bazel-rust-main -> test-rust-slice-main
  make: test-bazel-rust-main is deprecated; use test-rust-slice-main

  test-bazel-rust-api -> test-rust-slice-api
  make: test-bazel-rust-api is deprecated; use test-rust-slice-api

  test-bazel-rust-broker -> test-rust-slice-broker
  make: test-bazel-rust-broker is deprecated; use test-rust-slice-broker

  test-bazel-rust-aux -> test-rust-slice-aux
  make: test-bazel-rust-aux is deprecated; use test-rust-slice-aux
  ```

  Each alias writes only its one remediation line to stderr before forwarding
  and exits with the forwarded target's exact status.
- No workflow calls a deprecated alias.
- `make bazel-shutdown` remains.
- The same implementation change updates `AGENTS.md`, `tests/AGENTS.md`,
  `docs/contributing/gates-and-lints.md`, `tests/README.md`, and
  `docs/reference/test-execution-manifest.md` from eight Rust CI leaves to
  four Bazel slices while retaining the stable rollup language. The last two
  are included because they also describe the eight CI jobs.
- Promotion documentation and its semantic changelog fragment list every exact
  replacement above.

## Removal

Bazel-specific aliases may be removed only after a published semantic release
contains the promotion commit. Entry is exactly:

```bash
set -euo pipefail
promotion_sha=$(git rev-parse --verify "<promotion-commit>^{commit}")
tag=
while IFS= read -r candidate; do
  git merge-base --is-ancestor "$promotion_sha" "$candidate^{commit}" \
    || continue
  remote_commit=$(
    git ls-remote --exit-code --tags origin \
      "refs/tags/$candidate" "refs/tags/$candidate^{}" \
      | awk '
          $2 ~ /\^\{\}$/ { peeled = $1 }
          $2 !~ /\^\{\}$/ { direct = $1 }
          END { print peeled != "" ? peeled : direct }
        '
  ) || continue
  test -n "$remote_commit" || continue
  test "$(git rev-parse "$candidate^{commit}")" = "$remote_commit" || continue
  release_state=$(
    gh release view "$candidate" --json isDraft,isPrerelease \
      --jq '[.isDraft, .isPrerelease] | @tsv' 2>/dev/null
  ) || continue
  test "$release_state" = "$(printf 'false\tfalse')" || continue
  tag=$candidate
  break
done < <(
  git tag --contains "$promotion_sha" \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' \
    | sort -V
)
test -n "$tag"
```

A containing tag that does not match `^v[0-9]+\.[0-9]+\.[0-9]+$` is not a
release tag: the repository already carries two-component tags such as
`v1.0`, `v1.1`, and `v1.2`. An unpushed tag, a divergent same-named local and
remote tag, a draft release, or a prerelease also fails entry.

spec003w6 first updates
`packages/d2b-bazel-runner/tests/make_interface.rs` to expect removal and
observes that test fail, then removes the aliases and makes it pass.

Cargo implementations for the eighteen migrated surfaces may be removed only
after ten distinct ordered green promoted `v3` run units, where a unit is one
push-created (run ID, head SHA) pair and never an attempt. Retirement does not
remove:

- `make test-rust`;
- any existing `make test-rust-<leaf>` name;
- fixture-contract mode or either fixture-backed surface.
- the exact Cargo compatibility carriers for mandatory socket-using tests,
  their census, or their same-commit verdict contribution to existing surface
  IDs.

## Guards

Existing workflow and policy tests reject:

- direct Bazel workflow invocation;
- every contributor-only command in Make or workflows;
- retired hub identifiers except in refusal tests and documentation contracts;
- a workflow calling a deprecated alias;
- an indirect, post-step, or unknown cache writer;
- a public Rust Make name removed at promotion or retirement.

No new top-level gate is created.
