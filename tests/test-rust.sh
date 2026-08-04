#!/usr/bin/env bash
# tests/test-rust.sh - one Rust execution leaf for `make test-rust`.
#
# GNU Make owns the Rust DAG. This file owns leaf environment setup and the
# explicit leaf modes only: api-surface, main-workspace, broker,
# guest-shell-runner, no-bash-ast, schema-reproducibility, supply-chain,
# inventory-stub, and fixture-contracts. The fixture mode emits the contract
# and CLI surfaces separately.
# If cargo is absent, re-enter through the repo-pinned nixpkgs toolchain.

set -euo pipefail
unset MAKEFLAGS MFLAGS MAKELEVEL

rust_mode="${1:-}"
if [ "$#" -eq 0 ]; then
  printf '%s\n' "test-rust.sh removed the no-argument all scheduler; run make test-rust" >&2
  exit 2
fi
case "$rust_mode" in
  api-surface|main-workspace|broker|guest-shell-runner|no-bash-ast|schema-reproducibility|supply-chain|inventory-stub|fixture-contracts)
    [ "$#" -eq 1 ] || {
      printf '%s\n' "test-rust.sh accepts one leaf mode; run make test-rust" >&2
      exit 2
    }
    ;;
  *)
    printf '%s\n' "usage: tests/test-rust.sh {api-surface|main-workspace|broker|guest-shell-runner|no-bash-ast|schema-reproducibility|supply-chain|inventory-stub|fixture-contracts}; run make test-rust" >&2
    exit 2
    ;;
esac

suite_started=$SECONDS

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}

# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

cd "$ROOT"

fixture_contracts_only=0
[ "$rust_mode" = fixture-contracts ] && fixture_contracts_only=1

D2B_RUST_CARGO_JOBS=${D2B_RUST_CARGO_JOBS:-1}
D2B_RUST_NEXTEST_THREADS=${D2B_RUST_NEXTEST_THREADS:-1}
case "$D2B_RUST_CARGO_JOBS" in ''|*[!0-9]*) D2B_RUST_CARGO_JOBS=1 ;; esac
case "$D2B_RUST_NEXTEST_THREADS" in ''|*[!0-9]*) D2B_RUST_NEXTEST_THREADS=1 ;; esac
[ "$D2B_RUST_CARGO_JOBS" -ge 1 ] || D2B_RUST_CARGO_JOBS=1
[ "$D2B_RUST_NEXTEST_THREADS" -ge 1 ] || D2B_RUST_NEXTEST_THREADS=1
export CARGO_BUILD_JOBS="$D2B_RUST_CARGO_JOBS"

rust_current_surface="rust-$rust_mode"
rust_surface_command_succeeded=0
rust_manifest_exit_publication_enabled=1

publish_manifest_fragment() {
  local leaf="$1" status="$2"
  [ -n "${D2B_EXECUTION_MANIFEST:-}" ] || return 0
  perl "$ROOT/tests/tools/execution-manifest.pl" fragment \
    --manifest "$D2B_EXECUTION_MANIFEST" \
    --leaf "$leaf" \
    --status "$status"
}

rust_surface_start() {
  rust_current_surface="$1"
  rust_surface_command_succeeded=0
}

rust_surface_success() {
  local surface="$1"
  rust_surface_command_succeeded=1
  publish_manifest_fragment "$surface" passed
  rust_current_surface=""
  rust_surface_command_succeeded=0
}

disable_rust_manifest_exit_publication() {
  rust_manifest_exit_publication_enabled=0
}

rust_leaf_exit() {
  local rc=$?
  # tests/lib.sh installs run_cleanups as its EXIT trap. Disable this
  # replacement before doing any work so cleanup cannot recursively invoke
  # the handler and the original status remains authoritative.
  trap - EXIT
  if [ "$rust_manifest_exit_publication_enabled" -eq 1 ] \
    && [ -n "${D2B_EXECUTION_MANIFEST:-}" ] \
    && [ -n "$rust_current_surface" ]; then
    local failed_fragment_published=0
    if publish_manifest_fragment "$rust_current_surface" failed; then
      failed_fragment_published=1
    else
      if [ "$rust_surface_command_succeeded" -eq 0 ]; then
        printf '%s\n' \
          "test-rust: failed to record failed Rust surface '$rust_current_surface' in the execution manifest; preserving the original surface failure." \
          >&2
      fi
    fi
    if [ "$rust_surface_command_succeeded" -eq 1 ]; then
      printf '%s\n' \
        "test-rust: required execution-manifest fragment publication failed after successful surface '$rust_current_surface'; evidence is incomplete; retry the target." \
        >&2
    elif [ "$failed_fragment_published" -eq 0 ]; then
      printf '%s\n' \
        "test-rust: failed to record failed Rust surface '$rust_current_surface' in the execution manifest; preserving the original surface failure." \
        >&2
    fi
  fi
  # Keep all registrations made by tests/lib.sh, d2b_mktemp, cargo-audit, and
  # fixture materialisation effective after taking ownership of EXIT.
  run_cleanups || true
  exit "$rc"
}
trap rust_leaf_exit EXIT

if [ "$fixture_contracts_only" = 1 ] && [ "${D2B_ENABLE_FIXTURE_BUILD:-0}" != 1 ]; then
  fail "fixture-contracts mode requires D2B_ENABLE_FIXTURE_BUILD=1; refusing to report a skipped gate as passing"
  exit 1
fi

manifest="$ROOT/packages/Cargo.toml"
lock_file="$ROOT/packages/Cargo.lock"
deny_config="$ROOT/packages/deny.toml"
broker_manifest="$ROOT/packages/d2b-priv-broker/Cargo.toml"
broker_lock_file="$ROOT/packages/d2b-priv-broker/Cargo.lock"
broker_deny_config="$ROOT/packages/d2b-priv-broker/deny.toml"
guest_shell_runner_manifest="$ROOT/packages/d2b-guest-shell-runner/Cargo.toml"
guest_shell_runner_lock_file="$ROOT/packages/d2b-guest-shell-runner/Cargo.lock"
guest_shell_runner_deny_config="$ROOT/packages/d2b-guest-shell-runner/deny.toml"
for required in "$manifest" "$lock_file" "$deny_config" "$broker_manifest" "$broker_lock_file" "$broker_deny_config" "$guest_shell_runner_manifest" "$guest_shell_runner_lock_file" "$guest_shell_runner_deny_config"; do
  if [ ! -f "$required" ]; then
    fail "missing Rust workspace input: $required"
    exit 1
  fi
done
toolchain_file="$ROOT/packages/rust-toolchain.toml"
pinned_channel=$(
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]\+\)".*/\1/p' "$toolchain_file" | head -1
)
if [ -z "$pinned_channel" ]; then
  fail "could not read pinned Rust channel from $toolchain_file"
  exit 1
fi
export pinned_channel

workspace_target_dir=$(d2b_cargo_target_dir workspace)
if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ] \
  || [ "${D2B_RUST_COLD_PROFILE:-0}" = 1 ]; then
  fixture_target_dir="$workspace_target_dir"
else
  fixture_target_dir="$ROOT/.scratch/rust-test-cache/fixture-contracts"
fi
# Separate target dirs for the broker's three concurrent feature passes so they
# don't lock-contend. They are DETERMINISTIC siblings of the broker target dir
# (not mktemp): sccache hashes the inherited CARGO_* environment, including
# CARGO_TARGET_DIR, so a random per-run target dir would change the cache key
# and defeat cross-run hits. Stable, distinct dirs keep the key stable (cache
# hits) while still avoiding lock contention. They are gitignored and reused
# across runs like the default broker/workspace target dirs.
broker_target_dir=$(d2b_cargo_target_dir broker)
broker_layer1_target_dir="${broker_target_dir%/}-layer1"
broker_fakebackends_target_dir="${broker_target_dir%/}-fakebackends"
guest_shell_runner_target_dir=$(d2b_cargo_target_dir guest-shell-runner)
no_bash_target_dir="$ROOT/tests/tools/no-bash-ast-walker/target"

# Keep fixture-dependent contract crates out of generic workspace tests.
# Full D2B_FIXTURES delivery to the sandbox/CI is tracked separately.
workspace_test_excludes=(--exclude d2b-contract-tests)

d2b_activate_rust_toolchain_path || true
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$pinned_channel}"
# No Rust compiler warning is advisory in this gate. Clippy already receives
# `-D warnings`; this also covers cargo check/build/test, doctests, nextest,
# standalone workspaces, and build scripts. Compile-fail fixtures replace this
# inherited value with their exact mutation flags so expected diagnostics stay
# attributable to the capability seal they exercise.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"
export RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-D warnings"

if [ -z "${D2B_RUST_GATE_IN_NIX_SHELL:-}" ] && ! command -v rustup >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "rustup not on PATH and nix is unavailable; rust gate cannot run pinned Rust $pinned_channel"
    exit 1
  fi
  rust_gate_scratch=$(d2b_mktemp .d2b-rust-gate.XXXXXX)
  add_cleanup "rm -rf -- \"$rust_gate_scratch\""
  log "  rustup not on PATH; re-entering via nix shell to acquire pinned Rust $pinned_channel toolchain"
  export D2B_RUST_GATE_IN_NIX_SHELL=1
  export D2B_RUST_GATE_BOOTSTRAP_RUSTUP=1
  export RUSTUP_HOME="$rust_gate_scratch/rustup"
  export CARGO_HOME="$rust_gate_scratch/cargo"
  disable_rust_manifest_exit_publication
  if nix shell --quiet --inputs-from "$ROOT" \
      nixpkgs#rustup nixpkgs#stdenv.cc nixpkgs#sccache \
      --command bash "$0" "$@"; then
    nested_rust_rc=0
  else
    nested_rust_rc=$?
  fi
  exit "$nested_rust_rc"
fi

if [ -z "${D2B_RUST_GATE_IN_NIX_SHELL:-}" ] && command -v rustup >/dev/null 2>&1; then
  export D2B_RUST_GATE_IN_NIX_SHELL=1
  export D2B_RUST_GATE_BOOTSTRAP_RUSTUP=1
  export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
  export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
  rustup toolchain install "$pinned_channel" --profile minimal --component rustfmt --component clippy
fi

if [ -z "${D2B_RUST_GATE_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "neither cargo nor nix is on PATH; rust gate cannot run"
    exit 1
  fi
  rust_gate_scratch=$(d2b_mktemp .d2b-rust-gate.XXXXXX)
  add_cleanup "rm -rf -- \"$rust_gate_scratch\""
  log "  cargo not on PATH; re-entering via nix shell to acquire pinned Rust $pinned_channel toolchain"
  export D2B_RUST_GATE_IN_NIX_SHELL=1
  export D2B_RUST_GATE_BOOTSTRAP_RUSTUP=1
  export RUSTUP_HOME="$rust_gate_scratch/rustup"
  export CARGO_HOME="$rust_gate_scratch/cargo"
  disable_rust_manifest_exit_publication
  if nix shell --quiet --inputs-from "$ROOT" \
      nixpkgs#rustup nixpkgs#stdenv.cc nixpkgs#sccache \
      --command bash "$0" "$@"; then
    nested_rust_rc=0
  else
    nested_rust_rc=$?
  fi
  exit "$nested_rust_rc"
fi

if [ -n "${D2B_RUST_GATE_IN_NIX_SHELL:-}" ]; then
  if [ -n "${D2B_RUST_GATE_BOOTSTRAP_RUSTUP:-}" ]; then
    log "--> rustup toolchain install $pinned_channel"
    rustup toolchain install "$pinned_channel" --profile minimal --component rustfmt --component clippy
    export PATH="$CARGO_HOME/bin:$PATH"
  else
    D2B_RUST_GATE_REAL_CARGO=$(command -v cargo)
    export D2B_RUST_GATE_REAL_CARGO
  fi
  rustc() {
    if [ -n "${D2B_RUST_GATE_BOOTSTRAP_RUSTUP:-}" ]; then
      command rustup run "$pinned_channel" rustc "$@"
    else
      command rustc "$@"
    fi
  }
  cargo() {
    local cargo_args=()
    if [ "$#" -ge 3 ] && [ "$1" = "--manifest-path" ]; then
      local manifest_arg=$2
      shift 2
      cargo_args=( "$1" --manifest-path "$manifest_arg" "${@:2}" )
    else
      cargo_args=( "$@" )
    fi
    if [ -n "${D2B_RUST_GATE_BOOTSTRAP_RUSTUP:-}" ]; then
      command rustup run "$pinned_channel" cargo "${cargo_args[@]}"
    else
      command "$D2B_RUST_GATE_REAL_CARGO" "${cargo_args[@]}"
    fi
  }
  export -f rustc
  export -f cargo
fi

assert_pinned_rust_toolchain() {
  local cargo_version rustc_version
  cargo_version=$(cargo --version)
  rustc_version=$(rustc --version)
  case "$cargo_version" in
    *"$pinned_channel"*) ;;
    *)
      fail "cargo version does not match packages/rust-toolchain.toml channel $pinned_channel: $cargo_version"
      exit 1
      ;;
  esac
  case "$rustc_version" in
    *"$pinned_channel"*) ;;
    *)
      fail "rustc version does not match packages/rust-toolchain.toml channel $pinned_channel: $rustc_version"
      exit 1
      ;;
  esac
  ok "Rust toolchain matches packages/rust-toolchain.toml ($pinned_channel)"
}

cleanup_cargo_special_files() {
  local label="$1" dir="$2"
  local removed=0
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    rm -f -- "$path"
    removed=$((removed + 1))
  done < <(find "$dir" -type s -print 2>/dev/null || true)
  if [ "$removed" -gt 0 ]; then
    ok "$label removed $removed stale socket artifact(s) from $dir"
  fi
}

cleanup_package_test_scratch() {
  local label="$1" dir="$2"
  if [ -d "$dir" ]; then
    rm -rf -- "$dir"
    ok "$label removed package-local test scratch $dir"
  fi
}

# sccache: a per-crate compilation cache (keyed on source + flags), shared
# across the main + broker workspaces and all feature passes - so the broker's
# rebuilds of crates the main workspace already compiled (d2b-core/host/ipc)
# and its three separate-target-dir feature passes become cache hits. Used
# locally by default. In CI it is OFF unless D2B_CI_SCCACHE=1 is set, because it
# only helps when a persistent backend survives across runs. CI opts in by
# pointing SCCACHE_DIR at a directory it restores/saves via actions/cache - we
# deliberately use sccache's LOCAL-DISK backend (NOT SCCACHE_GHA_ENABLED): the
# native GHA backend needs ACTIONS_RUNTIME_TOKEN exported into this process's
# environment, where the untrusted crate code this gate compiles and runs
# (build scripts, proc-macros, `cargo test`) could read and exfiltrate it.
# actions/cache performs its I/O in its own action process and never exposes
# that token to `run:` steps.
#
# The explicit RUSTC_WRAPPER="" below is a real opt-out (D2B_NO_SCCACHE, or CI
# without the opt-in), not a robustness workaround. Cargo configs route rustc
# through .cargo/rustc-wrapper.sh, which falls through to plain rustc when
# sccache is missing, so no command needs to clear the variable merely to keep
# working in a shell that omits nixpkgs#sccache.
_ci_active=0
if [ -n "${CI:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
  _ci_active=1
fi
if [ "${D2B_NO_SCCACHE:-0}" = 1 ] || ! command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER=""
  log "sccache: disabled (forced off or unavailable)"
elif [ "$_ci_active" = 1 ] && [ "${D2B_CI_SCCACHE:-0}" != 1 ]; then
  export RUSTC_WRAPPER="" CARGO_BUILD_RUSTC_WRAPPER=""
  log "sccache: disabled (CI without D2B_CI_SCCACHE opt-in)"
else
  # Point at the shim explicitly rather than at the sccache binary. Setting
  # RUSTC_WRAPPER to sccache directly would bypass .cargo/rustc-wrapper.sh,
  # which is what classifies a broken sccache invocation as a wrapper error
  # (exit 97) instead of letting it read like a compiler diagnostic.
  #
  # An export is required; unsetting is NOT equivalent. Cargo resolves
  # .cargo/config.toml from the CWD, this script runs from $ROOT and passes
  # --manifest-path to every invocation, and there is no $ROOT/.cargo/config.toml
  # - so relying on the config would silently disable the wrapper for the whole
  # gate. The four shim copies are byte-identical, so one absolute path serves
  # every workspace.
  _wrapper_shim="$ROOT/packages/.cargo/rustc-wrapper.sh"
  export RUSTC_WRAPPER="$_wrapper_shim" CARGO_BUILD_RUSTC_WRAPPER="$_wrapper_shim"
  if [ "$_ci_active" = 1 ]; then
    log "sccache: enabled via $_wrapper_shim (CI opt-in, local backend at ${SCCACHE_DIR:-default})"
  else
    log "sccache: enabled via $_wrapper_shim"
  fi
fi

log "--> rust toolchain version"
assert_pinned_rust_toolchain

# Test execution runs under cargo-nextest, which executes each test in its own
# process and parallelises across test binaries rather than one binary at a
# time. Two surfaces nextest cannot execute, and which therefore get their own
# companion invocations below:
#
#   * DOCTESTS. nextest does not run them, and this repository's are
#     load-bearing: the `compile_fail` doctests on AdmittedMutation and
#     OwnerIndexMutation are capability seals (see the "Capability mint surface
#     allowlist" row in AGENTS.md). A bare nextest swap deletes them silently.
#   * HARNESS-FREE TEST AND BENCH TARGETS. Zero-case kind "test" suites expose
#     no libtest interface, while kind "bench" targets are not nextest test
#     surfaces. `cargo test` runs both with their matching target selector, so
#     they need explicit invocations to stay gated.
#
# Both companions are wired per workspace. The zero-case test set is
# DISCOVERED from nextest JSON and the bench set from Cargo metadata rather
# than hard-coded, so a newly added nextest-unrunnable target cannot silently
# drop out of the gate.
require_nextest() {
  if cargo nextest --version >/dev/null 2>&1; then
    return 0
  fi
  if [ -z "${D2B_RUST_GATE_NEXTEST_SHELL:-}" ] && command -v nix >/dev/null 2>&1; then
    log "  cargo-nextest not on PATH; re-entering via nix shell to acquire it"
    export D2B_RUST_GATE_NEXTEST_SHELL=1
    exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-nextest \
      --command bash "$0" "$@"
  fi
  fail "cargo-nextest is required and neither PATH nor nix can provide it"
  exit 1
}

# Emit nextest-unrunnable test and bench targets as
# "<kind>\t<package>\t<target>" rows. A zero-case kind "test" suite exposes no
# libtest interface, so cargo-nextest builds it but never runs it. A kind
# "bench" target is not a nextest test surface at all; Cargo metadata is the
# authoritative target inventory for those targets because listing a
# harness=false bench would execute its assertion-bearing binary while nextest
# tries to enumerate it.
#
# Deriving both sets rather than pinning them means a newly added harness-free
# target cannot silently drop out of the gate.
nextest_bench_targets_from_metadata() {
  local manifest_path="$1"
  shift

  local metadata
  if ! metadata=$(cargo metadata --format-version 1 --no-deps \
      --manifest-path "$manifest_path"); then
    fail "cargo metadata failed while discovering bench targets"
    return 1
  fi
  # Validate the metadata fields used below before querying them. In
  # particular, an absent workspace_members or target kind must not look like
  # a workspace with no benches.
  if ! printf '%s' "$metadata" | jq -e '
        type == "object"
        and (has("packages")
          and (."packages" | type == "array" and length > 0))
        and (has("workspace_members")
          and (."workspace_members" | type == "array" and length > 0))
        and all(."packages"[];
          type == "object"
          and has("id") and (.id | type == "string")
          and has("name") and (.name | type == "string")
          and has("manifest_path") and (.manifest_path | type == "string")
          and has("targets") and (.targets | type == "array")
          and all(.targets[];
            type == "object"
            and has("name") and (.name | type == "string")
            and has("kind")
              and (.kind | type == "array" and length > 0
                and all(.[]; type == "string"))
            and has("test") and (.test | type == "boolean")
          )
        )
      ' >/dev/null; then
    fail "cargo metadata JSON did not match the expected workspace shape; refusing to infer an empty bench set"
    return 1
  fi

  local workspace_selected=0
  local -a selected_packages=() excluded_packages=()
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --workspace|--all)
        workspace_selected=1
        ;;
      -p|--package)
        [ "$#" -ge 2 ] || {
          fail "package selector is missing its package name while discovering bench targets"
          return 1
        }
        selected_packages+=( "$2" )
        shift
        ;;
      --package=*)
        selected_packages+=( "${1#*=}" )
        ;;
      -p*)
        selected_packages+=( "${1#-p}" )
        ;;
      --exclude)
        [ "$#" -ge 2 ] || {
          fail "workspace exclusion is missing its package name while discovering bench targets"
          return 1
        }
        excluded_packages+=( "$2" )
        shift
        ;;
      --exclude=*)
        excluded_packages+=( "${1#*=}" )
        ;;
    esac
    shift
  done

  local manifest_real
  manifest_real=$(readlink -f -- "$manifest_path")
  local workspace_packages
  workspace_packages=$(printf '%s' "$metadata" | jq -r '
        . as $metadata
        | $metadata.packages[]
        | select(.id as $id | $metadata.workspace_members | index($id))
        | [.name, .manifest_path]
        | @tsv
      ') || {
    fail "cargo metadata workspace package discovery failed"
    return 1
  }

  declare -A selected_set=() excluded_set=()
  local package_name
  for package_name in "${selected_packages[@]}"; do
    selected_set["$package_name"]=1
  done
  for package_name in "${excluded_packages[@]}"; do
    excluded_set["$package_name"]=1
  done

  local selected_count=0 package_manifest include excluded
  while IFS=$'\t' read -r package_name package_manifest; do
    [ -n "$package_name" ] || continue
    include=0
    if [ "${#selected_packages[@]}" -gt 0 ]; then
      [ "${selected_set[$package_name]+yes}" = yes ] && include=1
    elif [ "$workspace_selected" -eq 1 ]; then
      include=1
    elif [ "$package_manifest" = "$manifest_real" ]; then
      include=1
    fi
    excluded=0
    [ "${excluded_set[$package_name]+yes}" = yes ] && excluded=1
    if [ "$include" -eq 1 ] && [ "$excluded" -eq 0 ]; then
      selected_count=$((selected_count + 1))
    fi
  done <<<"$workspace_packages"
  if [ "$selected_count" -eq 0 ]; then
    fail "cargo metadata selected no workspace packages while discovering bench targets"
    return 1
  fi

  local all_bench_targets
  all_bench_targets=$(printf '%s' "$metadata" | jq -r '
        . as $metadata
        | $metadata.packages[]
        | select(.id as $id | $metadata.workspace_members | index($id))
        | .name as $package_name
        | .targets[]
        | select((.kind | index("bench")) != null)
        | [$package_name, .name]
        | @tsv
      ') || {
    fail "cargo metadata bench target discovery failed"
    return 1
  }
  while IFS=$'\t' read -r package_name target_name; do
    [ -n "$package_name" ] || continue
    include=0
    if [ "${#selected_packages[@]}" -gt 0 ]; then
      [ "${selected_set[$package_name]+yes}" = yes ] && include=1
    elif [ "$workspace_selected" -eq 1 ]; then
      include=1
    else
      while IFS=$'\t' read -r workspace_package workspace_manifest; do
        if [ "$workspace_package" = "$package_name" ] \
          && [ "$workspace_manifest" = "$manifest_real" ]; then
          include=1
          break
        fi
      done <<<"$workspace_packages"
    fi
    excluded=0
    [ "${excluded_set[$package_name]+yes}" = yes ] && excluded=1
    if [ "$include" -eq 1 ] && [ "$excluded" -eq 0 ]; then
      printf 'bench\t%s\t%s\n' "$package_name" "$target_name"
    fi
  done <<<"$all_bench_targets"
}

nextest_unrunnable_targets() {
  local manifest_path="$1"
  shift
  local listing
  # No stderr suppression and an explicit status check: this discovers a gate
  # surface, so a listing that errors must fail the gate rather than silently
  # yield an empty set and let the companion report a passing surface.
  if ! listing=$(cargo nextest list --manifest-path "$manifest_path" "$@" --message-format json); then
    fail "cargo nextest list failed while discovering nextest-unrunnable targets"
    return 1
  fi
  # Validate the shape the filter below actually reads, not just the top-level
  # key. A schema change that renamed any of these would otherwise let jq match
  # nothing and exit 0, which reads identically to "this workspace has no
  # harness-free targets" and would silently empty the gate surface.
  if ! printf '%s' "$listing" | jq -e '
        type == "object"
        and (has("rust-suites")
          and (."rust-suites" | type == "object" and length > 0
            and (to_entries | all(.value
              | type == "object"
              and has("kind") and (.kind | type == "string")
              and has("testcases")
                and (.testcases | type == "object" or type == "array")
              and has("package-name") and (.["package-name"] | type == "string")
              and has("binary-name") and (.["binary-name"] | type == "string"))))
        )
      ' >/dev/null; then
    fail "cargo nextest list JSON did not match the expected suite shape; refusing to infer an empty nextest-unrunnable set"
    return 1
  fi
  local nextest_targets metadata_targets
  nextest_targets=$(printf '%s' "$listing" | jq -r '
        ."rust-suites"
        | to_entries[]
        | .value
        | select(
            (.kind == "test" and ((.testcases | length) == 0))
            or .kind == "bench"
          )
        | "\(.kind)\t\(.["package-name"])\t\(.["binary-name"])"
      ') || {
    fail "cargo nextest target extraction failed"
    return 1
  }
  metadata_targets=$(nextest_bench_targets_from_metadata "$manifest_path" "$@") || return 1

  # `nextest list` may gain native bench records in a future release. The
  # metadata fallback is still required for the current harness=false benches,
  # so de-duplicate the two authoritative views before execution.
  printf '%s\n%s\n' "$nextest_targets" "$metadata_targets" \
    | awk 'NF' \
    | LC_ALL=C sort -u
}

# Run the surfaces cargo-nextest cannot: doctests, then any nextest-unrunnable
# test or bench targets. `label` names the workspace for the log line; the remaining
# arguments select the workspace and feature set and are shared with the
# preceding nextest invocation.
run_nextest_companions() {
  local label="$1" manifest_path="$2"
  shift 2
  log "  --> cargo test --doc ($label)"
  cargo test --jobs "$D2B_RUST_CARGO_JOBS" --doc --manifest-path "$manifest_path" "$@" -- --test-threads "$D2B_RUST_NEXTEST_THREADS"
  # Capture before looping. A process substitution hides its exit status from
  # `set -e`, so discovering through `done < <(...)` would let a failed listing
  # look like an empty one.
  local targets
  targets=$(nextest_unrunnable_targets "$manifest_path" "$@") || {
    fail "$label: could not discover nextest-unrunnable targets"
    exit 1
  }
  local kind pkg target cargo_selector ran=0 test_targets=0 bench_targets=0
  local -a cargo_profile=()
  while IFS=$'\t' read -r kind pkg target; do
    [ -n "$kind" ] || continue
    case "$kind" in
      test)
        cargo_selector=--test
        cargo_profile=()
        test_targets=$((test_targets + 1))
        ;;
      bench)
        cargo_selector=--bench
        cargo_profile=(--release)
        bench_targets=$((bench_targets + 1))
        ;;
      *)
        fail "$label: discovery returned unknown target kind '$kind'"
        exit 1
        ;;
    esac
    [ -n "$pkg" ] && [ -n "$target" ] || {
      fail "$label: discovery returned an incomplete $kind target row"
      exit 1
    }
    log "  --> cargo test -p $pkg $cargo_selector $target ($label; $kind, not a nextest surface)"
    # Forward the same selectors the listing used, so the companion runs the
    # configuration that produced it rather than a default-feature rebuild.
    cargo test --jobs "$D2B_RUST_CARGO_JOBS" "${cargo_profile[@]}" \
      --manifest-path "$manifest_path" "$@" -p "$pkg" "$cargo_selector" "$target"
    ran=$((ran + 1))
  done <<<"$targets"
  if [ "$ran" -eq 0 ] && [ "$label" = "main workspace" ]; then
    fail "$label: nextest-unrunnable discovery was empty; refusing to report a passing companion surface"
    return 1
  fi
  if [ "$label" = "main workspace" ] && [ "$test_targets" -eq 0 ]; then
    fail "$label: zero-case test discovery was empty; refusing to report a passing companion surface"
    return 1
  fi
  if [ "$label" = "main workspace" ] && [ "$bench_targets" -eq 0 ]; then
    fail "$label: bench discovery was empty; refusing to report a passing companion surface"
    return 1
  fi
  ok "$label companions (doctests + $test_targets test targets + $bench_targets bench targets)"
}

# The privileged broker is a SEPARATE workspace with three independent feature
# passes (default, layer1-bootstrap, fake-backends), each on its OWN target dir.
# They share nothing with the main workspace and nothing with each other. The
# streams stay serial by default because their tests manipulate process-global
# signal/reap state; an explicit timing-only opt-in can use their separate target
# directories. With sccache the shared crates are cache hits across all streams.
# Running serially in the foreground means a failing stream aborts the gate at
# the point of failure, with its output already on the gate's own stream.
#
# These three streams deliberately stay on `cargo test` rather than moving to
# cargo-nextest with the rest of the gate. The broker's tests are not
# process-per-test safe: under nextest, runtime::tests::usbip_bind_* fails with
# LiveHandler("USB device 1-2.3 is missing required sysfs attr devpath"),
# because whatever keeps handler selection off live sysfs does not survive being
# run in its own process. The same test passes under `cargo test` when filtered
# down to itself alone, so this is a harness-environment dependency rather than
# a flaky test or an inter-test ordering bug.
#
# The cost of not converting is nil: this suite runs 528 tests in about 1.4 s,
# so nextest's cross-binary parallelism has nothing to win here, and `cargo test`
# covers doctests and harness=false binaries without needing companions. Making
# the broker process-per-test safe would mean reworking test setup inside a
# critical, privileged, protected subsystem for no measurable gain.
#
# No stream runs `cargo check` before its `cargo test`. Check and test are
# distinct compilation modes that share no artifacts, so a check ahead of a test
# on the same target directory is time spent twice for one result. Measured cold
# on the layer1 stream: 153 s with the check against 89 s without, for an
# identical outcome and an unchanged test phase (85 s against 89 s). The
# fake-backends stream never had one, which is why it was already the fastest.
broker_stream_default() {
  cargo metadata --format-version 1 --manifest-path "$broker_manifest" >/dev/null
  rm -f -- "$broker_target_dir"/debug/deps/socket_activation-* 2>/dev/null || true
  CARGO_TARGET_DIR="$broker_target_dir" cargo test --jobs "$D2B_RUST_CARGO_JOBS" --workspace --manifest-path "$broker_manifest" -- --test-threads "$D2B_RUST_NEXTEST_THREADS"
}
broker_stream_layer1() {
  CARGO_TARGET_DIR="$broker_layer1_target_dir" cargo test --jobs "$D2B_RUST_CARGO_JOBS" --workspace --manifest-path "$broker_manifest" --features layer1-bootstrap -- --test-threads "$D2B_RUST_NEXTEST_THREADS"
}
broker_stream_fakebackends() {
  CARGO_TARGET_DIR="$broker_fakebackends_target_dir" cargo test --jobs "$D2B_RUST_CARGO_JOBS" --workspace --manifest-path "$broker_manifest" --features fake-backends -- --test-threads "$D2B_RUST_NEXTEST_THREADS"
}
broker_streams=(default layer1 fakebackends)

guest_shell_runner_gate() {
  cargo metadata --format-version 1 --manifest-path "$guest_shell_runner_manifest" >/dev/null
  CARGO_TARGET_DIR="$guest_shell_runner_target_dir" cargo fmt --manifest-path "$guest_shell_runner_manifest" --all --check
  CARGO_TARGET_DIR="$guest_shell_runner_target_dir" cargo clippy --jobs "$D2B_RUST_CARGO_JOBS" --manifest-path "$guest_shell_runner_manifest" --workspace --all-targets --features real-libshpool -- -D warnings
  CARGO_TARGET_DIR="$guest_shell_runner_target_dir" cargo nextest run --test-threads "$D2B_RUST_NEXTEST_THREADS" --manifest-path "$guest_shell_runner_manifest" --workspace --features real-libshpool
  CARGO_TARGET_DIR="$guest_shell_runner_target_dir" run_nextest_companions \
    "guest shell runner" "$guest_shell_runner_manifest" --workspace --features real-libshpool
}

run_fixture_contract_tests() {
  local eval_root system flake_ref eval_rc testname
  local fixture_contract_filter='not binary(video_binary_contract)'
  rust_surface_start rust-contract-tests
  for testname in "${D2B_FIXTURE_INDEPENDENT_POLICY_BINARIES[@]}"; do
    fixture_contract_filter+=" and not binary($testname)"
  done
  eval_root=$(d2b_mktemp ".d2b-eval-fixtures.XXXXXX")
  # The feature-rich full fixture is graphics-gated to x86_64-linux, so
  # eval-fixtures.sh exits 3 elsewhere. Leave D2B_FIXTURES_FULL empty there so
  # the per-role minijail tests skip, instead of aborting the whole gate.
  contract_fixtures_full=""
  eval_rc=0
  bash "$ROOT/tests/tools/eval-fixtures.sh" "$eval_root" >/dev/null || eval_rc=$?
  case "$eval_rc" in
    0) contract_fixtures_full="$eval_root/full" ;;
    3) log "  SKIP: eval-rendered full fixture (x86_64-linux only)" ;;
    *)
      fail "eval-fixtures.sh failed (exit $eval_rc)" || true
      exit "$eval_rc"
      ;;
  esac
  system=$(nix eval --raw --impure --expr builtins.currentSystem)
  flake_ref=$(d2b_flake_ref "$ROOT")
  log "--> nix build fixture-smoke (production-rendered minimal bundle)"
  contract_fixtures=$(nix build --no-link --print-out-paths \
    "${flake_ref}#checks.${system}.fixture-smoke")
  log "--> cargo nextest run -p d2b-contract-tests (realized minimal + eval-rendered full artifacts)"
  D2B_FIXTURES="$contract_fixtures" D2B_FIXTURES_FULL="$contract_fixtures_full" \
  CARGO_TARGET_DIR="$fixture_target_dir" \
    cargo nextest run --test-threads "$D2B_RUST_NEXTEST_THREADS" --manifest-path "$manifest" -p d2b-contract-tests \
      -E "$fixture_contract_filter"
  D2B_FIXTURES="$contract_fixtures" D2B_FIXTURES_FULL="$contract_fixtures_full" \
  CARGO_TARGET_DIR="$fixture_target_dir" \
    run_nextest_companions "contract crate" "$manifest" -p d2b-contract-tests
  rust_surface_success rust-contract-tests
  ok "d2b-contract-tests (realized minimal + eval-rendered full fixture-contract layer)"
}

run_cli_contract_tests() {
  local fixture_path="$1"
  local d2bd_bin
  rust_surface_start rust-cli-contract-tests

  # CLI-contract layer: spawn the real `d2b` binary against the rendered
  # fixture bundle (D2B_FIXTURES) + a synthetic system-state and validate the
  # JSON envelopes strictly against the committed ListOutputV2/StatusOutputV2
  # DTOs (deny_unknown_fields). Successor of the cli-rust-native-* bash gates.
  #
  # A few CLI-contract cases (audit/host-check daemon-backed paths) spawn a
  # real, KVM-free `d2bd serve --once --test-listen-on` and talk to it over
  # AF_UNIX + SO_PEERCRED. Build d2bd and hand its path to the test via
  # D2B_TEST_D2BD_BIN so those cases run instead of skipping. d2b
  # does NOT depend on d2bd (the static-rust-dependency-direction gate
  # forbids it), so the path is delivered out-of-band rather than via a dep edge.
  log "--> cargo build -p d2bd (CLI-contract daemon-spawn harness binary)"
  CARGO_TARGET_DIR="$fixture_target_dir" \
    cargo build --jobs "$D2B_RUST_CARGO_JOBS" --manifest-path "$manifest" -p d2bd
  d2bd_bin="$fixture_target_dir/debug/d2bd"
  [ -x "$d2bd_bin" ] || {
    fail "d2bd binary not found at $d2bd_bin"
    return 1
  }
  log "--> cargo nextest run -p d2b --tests (CLI-contract, D2B_FIXTURES = fixture-smoke)"
  D2B_FIXTURES="$fixture_path" \
  D2B_TEST_D2BD_BIN="$d2bd_bin" \
  CARGO_TARGET_DIR="$fixture_target_dir" \
    cargo nextest run --test-threads "$D2B_RUST_NEXTEST_THREADS" --manifest-path "$manifest" -p d2b --tests
  rust_surface_success rust-cli-contract-tests
  ok "d2b --tests (CLI-contract layer)"
}

if [ "$fixture_contracts_only" = 1 ]; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "D2B_ENABLE_FIXTURE_BUILD=1 requires nix to materialize D2B_FIXTURES"
    exit 1
  fi
  require_nextest "$@"
  run_fixture_contract_tests
  run_cli_contract_tests "$contract_fixtures"
  log "test-fixture-contracts OK (realized minimal fixture + eval full fixture + CLI-contract layers; duration: $((SECONDS - suite_started))s)"
  exit 0
fi

# The compiler-derived API census. Its own CI shard, because it shares nothing
# with the workspace build that would make serialising it behind one worthwhile:
# it renders through a separately pinned nightly toolchain into its own target
# directory, so it neither consumes nor produces artifacts that clippy and
# nextest use. Measured on the pull-request gate it ran 209 s inside a 878 s
# main shard whose peer shard finished 172 s earlier, so that 209 s sat directly
# on the gate's critical path while a runner stood idle. Splitting it moves the
# work sideways rather than removing it: total runner minutes barely change, the
# longest path shortens by roughly the whole 209 s.
#
# One compiler-owned workspace build replaces the old test's serial
# package-by-package HTML rustdoc loop. This is enforcing and cannot skip.
run_api_surface_gate() {
  rust_surface_start rust-api-surface
  log "--> compiler-derived API surface"
  bash "$ROOT/tests/tools/api-surface-json.sh"
  rust_surface_success rust-api-surface
  log "test-rust api-surface OK (duration: $((SECONDS - suite_started))s)"
}

run_main_workspace_gate() {
  require_nextest "$rust_mode"
  rust_surface_start rust-main-format
  log "--> cargo fmt --check"
  cargo fmt --manifest-path "$manifest" --all --check
  rust_surface_success rust-main-format
  ok "cargo fmt --check"

# --locked so a stale committed Cargo.lock fails the gate instead of being
# silently regenerated. flake.nix vendors the committed lockfile, so a lock
# that cargo quietly rewrites here cannot be reproduced by a Nix build.
rust_surface_start rust-main-clippy
log "--> cargo clippy --locked --workspace --all-targets -- -D warnings"
CARGO_TARGET_DIR="$workspace_target_dir" cargo clippy --jobs "$D2B_RUST_CARGO_JOBS" --locked --manifest-path "$manifest" --workspace --all-targets -- -D warnings
rust_surface_success rust-main-clippy
ok "cargo clippy"

rust_surface_start rust-main-workspace-tests
log "--> cargo nextest run --workspace ${workspace_test_excludes[*]}"
workspace_test_started=$SECONDS
CARGO_TARGET_DIR="$workspace_target_dir" cargo nextest run --test-threads "$D2B_RUST_NEXTEST_THREADS" --locked --manifest-path "$manifest" --workspace "${workspace_test_excludes[@]}"
CARGO_TARGET_DIR="$workspace_target_dir" run_nextest_companions \
  "main workspace" "$manifest" --locked --workspace "${workspace_test_excludes[@]}"
ok "workspace tests (duration: $((SECONDS - workspace_test_started))s)"

cleanup_cargo_special_files "workspace cargo test" "$workspace_target_dir"
cleanup_package_test_scratch "workspace cargo test" "$ROOT/packages/d2bd/target"
rust_surface_success rust-main-workspace-tests
  log "test-rust main-workspace OK (duration: $((SECONDS - suite_started))s)"
}

run_no_bash_ast_gate() {
rust_surface_start rust-no-bash-ast
# no-bash-exec AST layer (ADR 0017): the per-line `Command::new("bash")` scan
# is covered by d2b-contract-tests/tests/policy_source.rs, but the
# AST-level walk (which catches cross-line / obfuscated bash-exec sites the
# per-line regex cannot) lives in the standalone tests/tools/no-bash-ast-walker
# cargo tool. The retired tests/no-bash-exec-eval.sh ran it via `... all`; run
# it here so the AST coverage stays gated. Fails closed on any bash-literal
# Command::new site under packages/.
log "--> no-bash-ast-walker (ADR 0017 AST-level bash-exec scan)"
CARGO_TARGET_DIR="$no_bash_target_dir" \
  cargo run --jobs "$D2B_RUST_CARGO_JOBS" --release --quiet \
    --manifest-path "$ROOT/tests/tools/no-bash-ast-walker/Cargo.toml" \
    -- "$ROOT/packages"
rust_surface_success rust-no-bash-ast
ok "no-bash-ast-walker (zero Command::new bash-literal sites)"
}

run_broker_gate() {
# Broker workspace: run the three feature passes (default, layer1-bootstrap,
# fake-backends) - each on its own target dir - serially. Tests inside each
# cargo-test process manipulate process-global SIGCHLD/reap state, so do not
# overlap the three harnesses unless a dedicated isolation review proves it safe.
# The fail-closed
# `fake-backends` stream runs the broker's hermetic
# integration tests (e.g. tests/pidfd_handoff_scm_rights.rs,
# #![cfg(feature = "fake-backends")], pinned in
# tests/golden/pinned/pidfd-handoff.txt) that neither the default nor the
# layer1-bootstrap pass enables - without it those fd-passing tests would not
# run in the gate at all (the retired tests/pidfd-handoff.sh used --all-features).
for _stream in "${broker_streams[@]}"; do
  case "$_stream" in
    default) _surface=rust-broker-default ;;
    layer1) _surface=rust-broker-layer1 ;;
    fakebackends) _surface=rust-broker-fakebackends ;;
    *) fail "unknown broker stream: $_stream"; exit 1 ;;
  esac
  rust_surface_start "$_surface"
  log "--> broker cargo ($_stream feature pass, serial)"
  "broker_stream_$_stream"
  rust_surface_success "$_surface"
  ok "broker cargo ($_stream feature pass)"
done
cleanup_cargo_special_files "broker cargo test" "$broker_target_dir"
cleanup_cargo_special_files "broker layer1 cargo test" "$broker_layer1_target_dir"
cleanup_cargo_special_files "broker fake-backends cargo test" "$broker_fakebackends_target_dir"
}

run_guest_shell_runner_gate() {
require_nextest "$rust_mode"
rust_surface_start rust-guest-shell-runner
log "--> guest shell runner cargo (standalone workspace, real-libshpool feature)"
guest_shell_runner_gate
ok "guest shell runner cargo"
cleanup_cargo_special_files "guest shell runner cargo test" "$guest_shell_runner_target_dir"
rust_surface_success rust-guest-shell-runner
}

run_schema_reproducibility_gate() {
rust_surface_start rust-schema-reproducibility
schema_out="$ROOT/packages/xtask/out"
schema_out_preexisting=0
if [ -e "$schema_out" ]; then
  schema_out_preexisting=1
fi
snapshot_schema_out() {
  if [ ! -d "$schema_out" ]; then
    return 0
  fi
  (
    cd "$schema_out"
    find . -type f -print0 \
      | LC_ALL=C sort -z \
      | xargs -0 -r sha256sum
  )
}

log "--> schema generation reproducibility"
(cd "$ROOT/packages" && cargo xtask gen-schemas)
schema_snapshot_1=$(snapshot_schema_out)
(cd "$ROOT/packages" && cargo xtask gen-schemas)
schema_snapshot_2=$(snapshot_schema_out)
if [ "$schema_snapshot_1" != "$schema_snapshot_2" ]; then
  fail "schema generation reproducibility: cargo xtask gen-schemas output is not reproducible"
  diff -u \
    <(printf '%s\n' "$schema_snapshot_1") \
    <(printf '%s\n' "$schema_snapshot_2") >&2 || true
  exit 1
fi
if [ "$schema_out_preexisting" = "0" ]; then
  rm -rf -- "$schema_out"
fi
rust_surface_success rust-schema-reproducibility
ok "schema generation reproducibility"
}

run_supply_chain_gate() {
cargo_deny_check() {
  local label="$1" manifest_path="$2" config_path="$3"
  if command -v cargo-deny >/dev/null 2>&1; then
    log "--> cargo deny check ($label)"
    cargo deny --manifest-path "$manifest_path" check --config "$config_path"
    ok "cargo deny check ($label)"
  elif command -v nix >/dev/null 2>&1; then
    log "--> cargo deny check ($label via nix shell)"
    nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-deny --command \
      cargo deny --manifest-path "$manifest_path" check --config "$config_path"
    ok "cargo deny check ($label)"
  else
    fail "cargo deny check cannot run for $label: cargo-deny and nix are unavailable; ADR 0009 does not authorize a waiver"
    exit 1
  fi
}

cargo_audit_check() {
  local label="$1" lock_path="$2"
  shift 2
  local attempts=3 attempt audit_dir audit_out rc
  if ! command -v cargo-audit >/dev/null 2>&1 && ! command -v nix >/dev/null 2>&1; then
    fail "cargo audit cannot run for $label: cargo-audit and nix are unavailable; ADR 0009 does not authorize a waiver"
    exit 1
  fi
  audit_dir=$(d2b_mktemp ".cargo-audit.${label//[^A-Za-z0-9._-]/-}.XXXXXX")
  audit_out="$audit_dir/output.log"
  for attempt in $(seq 1 "$attempts"); do
    log "--> cargo audit ($label)"
    log "  attempt $attempt/$attempts"
    if command -v cargo-audit >/dev/null 2>&1; then
      set +e
      cargo audit --file "$lock_path" "$@" >"$audit_out" 2>&1
      rc=$?
      set -e
    else
      set +e
      nix shell --quiet --inputs-from "$ROOT" nixpkgs#cargo-audit --command \
        cargo audit --file "$lock_path" "$@" >"$audit_out" 2>&1
      rc=$?
      set -e
    fi
    if [ "$rc" -eq 0 ]; then
      cat "$audit_out"
      ok "cargo audit ($label)"
      return 0
    fi
    if [ "$rc" -eq 1 ]; then
      cat "$audit_out" >&2
      fail "cargo audit ($label) reported vulnerabilities"
      return 1
    fi
    [ "$attempt" -lt "$attempts" ] || break
    log "  RETRY: cargo audit ($label) after transient failure"
    sleep 5
  done
  cat "$audit_out" >&2
  fail "cargo audit ($label) failed after $attempts attempts"
  return 1
}

rust_surface_start rust-deny-main
cargo_deny_check "main workspace" "$manifest" "$deny_config"
rust_surface_success rust-deny-main
rust_surface_start rust-deny-broker
cargo_deny_check "broker workspace" "$broker_manifest" "$broker_deny_config"
rust_surface_success rust-deny-broker
rust_surface_start rust-deny-guest
cargo_deny_check "guest shell runner workspace" "$guest_shell_runner_manifest" "$guest_shell_runner_deny_config"
rust_surface_success rust-deny-guest

# Build-time wayland-scanner pulls quick-xml 0.39.4; runtime users were
# updated away from vulnerable 0.37.x. Remove once wayland-scanner publishes
# a release on quick-xml >= 0.41.
rust_surface_start rust-audit-main
cargo_audit_check "main workspace" "$lock_file" \
  --ignore RUSTSEC-2026-0194 \
  --ignore RUSTSEC-2026-0195
rust_surface_success rust-audit-main
rust_surface_start rust-audit-broker
cargo_audit_check "broker workspace" "$broker_lock_file"
rust_surface_success rust-audit-broker
# libshpool 0.11.0 pulls notify 7 -> notify-types -> instant 0.1.13.
# The helper pins and tracks that transitive unmaintained advisory explicitly
# while evaluating libshpool feasibility.
rust_surface_start rust-audit-guest
cargo_audit_check "guest shell runner workspace" "$guest_shell_runner_lock_file" --ignore RUSTSEC-2024-0384
rust_surface_success rust-audit-guest
}

run_inventory_stub_gate() {
rust_surface_start rust-stub-no-socket
log "--> tests/tools/stub-no-socket.sh"
bash "$ROOT/tests/tools/stub-no-socket.sh"
rust_surface_success rust-stub-no-socket
ok "stub-no-socket"

# Fail-closed Rust test inventory: every pinned workspace + broker test must
# still exist (catches a silently-deleted test that would otherwise vanish from
# coverage). The pinned set is committed under tests/golden/pinned/.
rust_surface_start rust-assert-pinned
log "--> tests/tools/assert-pinned-tests.sh"
bash "$ROOT/tests/tools/assert-pinned-tests.sh"
rust_surface_success rust-assert-pinned
ok "assert-pinned-tests"
}

# The execution-manifest helper is the only evidence plumbing used by these
# leaves. It resolves the parent first through openat2-equivalent anchored
# traversal with RESOLVE_NO_SYMLINKS and RESOLVE_NO_MAGICLINKS, then uses
# O_CLOEXEC, O_NOFOLLOW, and nonblocking F_OFD_SETLK on the persistent
# mode-0600 lock owned by the current uid. Its `manifest-lock-contended`
# telemetry is path-free and says that the execution-manifest lock is active:
# wait for the active run to finish and retry.
#
# Each current-user mode-0700 adjacent fragment directory is verified with
# fstat and same-filesystem device checks. Complete fragments use atomic
# rename; stale cleanup uses anchored openat and unlinkat, skips an invalid
# entry with continue, and never uses path-based recursive removal. The
# deterministic schema version 1 manifest contains run_status,
# completed_leaves, failed_surfaces, installables, and realized_checks.
#
# The helper runs the scheduler in a dedicated process group. SIGTERM and
# SIGINT are forwarded, the fixed 10 seconds are waited, survivors receive
# SIGKILL and are reaped, and idempotent finalization preserves the original
# status. Internal clock, process, and path boundaries are injectable for
# hermetic tests; there is no public shutdown-grace knob.
case "$rust_mode" in
  api-surface)
    run_api_surface_gate
    ;;
  main-workspace)
    run_main_workspace_gate
    ;;
  broker)
    run_broker_gate
    ;;
  guest-shell-runner)
    run_guest_shell_runner_gate
    ;;
  no-bash-ast)
    run_no_bash_ast_gate
    ;;
  schema-reproducibility)
    run_schema_reproducibility_gate
    ;;
  supply-chain)
    run_supply_chain_gate
    ;;
  inventory-stub)
    run_inventory_stub_gate
    ;;
esac
