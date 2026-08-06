# Makefile - d2b repository top-level convenience targets.
#
# Maintainer-facing targets only; CI converges on this stable make-target
# interface incrementally during the test rearchitecture.

# Recipe shells must not inherit exported Bash functions from their caller.
# Function resolution precedes PATH lookup, so an inherited cargo/nix/jq
# function could silently redirect a gate that intends to execute a binary.
SHELL := $(CURDIR)/tests/tools/scrub-shell-environment

.PHONY: pre-tag smoke-lite i3-check \
        check check-static check-ci check-all check-fast check-tier0 \
        test test-unit \
        test-lint test-rust test-rust-api-surface test-rust-main \
        test-rust-broker test-rust-guest-shell-runner test-rust-no-bash-ast \
        test-rust-schema test-rust-inventory test-rust-supply-chain \
        test-rust-leaf-api-surface test-rust-leaf-main-workspace \
        test-rust-leaf-schema test-rust-leaf-inventory \
        test-rust-leaf-fixture-contracts test-rust-leaf-broker \
        test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast \
        test-rust-leaf-supply-chain \
        test-fixture-contracts test-proofs test-flake test-nix-unit \
        test-performance-budgets test-adr-index-coverage test-ci-coverage \
        test-flake-list test-flake-partition \
        test-drift test-policy test-integration test-host-integration test-hardware perf \
        heavy-lane-guard heavy-lane-integration heavy-lane-host-integration \
        heavy-lane-hardware heavy-lane-perf \
        heavy-lane-pre-tag heavy-lane-smoke-lite \
        heavy-gate-build heavy-gate-provision heavy-check heavy-cargo-test heavy-flake-check \
        heavy-test-integration heavy-test-host-integration heavy-test-hardware \
        layer1-workflow layer1-workflow-check \
        ledger-regen check-inventory pr-checklist-gate nix-unit-pin flake-matrix-pin \
        api-surface-pin runtime-ledger-pin clean

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)
NIX_FLAKE := nix --extra-experimental-features 'nix-command flakes'

# ===========================================================================
# Test-rearchitecture interface. The targets are the stable contract; the
# local/CI Layer-1 gate graph lives in tests/layer1-jobs.json.
#
#   make check          L1 PR-equivalent gate, locally parallelized.
#   make check-static   Legacy monolithic tests/static.sh full-static gate.
#   make check-ci       check + test-integration for local/manual compatibility.
#   make check-all      check-ci + test-hardware + perf - full local NixOS gate.
#   make test-<layer>   focused per-layer run (ledger-driven).
#   make test-integration  type-9 container integration; local host/manual pre-PR.
#   make test-host-integration  type-10 runNixOSTest; local NixOS/KVM pre-PR.
#   make test-hardware     G-hw real GPU/YubiKey/TPM passthrough - NixOS host only.
#   make heavy-<lane>      the same lane, serialized through the two-slot
#                          per-uid heavy-gate semaphore (see "Heavy lanes").
# ===========================================================================

## check - the Layer-1 PR-equivalent done-gate. The manifest runner executes
##          check-tier0 first, then safe L1 sub-targets in parallel, then
##          drift after the parallel phase. Tune with D2B_CHECK_JOBS and
##          D2B_FLAKE_JOBS.
check:
	bash tests/tools/layer1-jobs run-local

## check-static - legacy/full-static monolithic gate retained for explicit use.
check-static:
	bash tests/static.sh

## check-ci - W0: run check, then skip or run legacy G-ci on a suitable host.
check-ci:
	$(MAKE) check
	$(MAKE) test-integration

## check-all - the full local gate on a NixOS host with devices.
check-all:
	$(MAKE) check-ci
	$(MAKE) test-hardware
	$(MAKE) perf

## check-fast / check-tier0 - fast PR-loop subsets.
## check-fast is superseded by `make test-unit` (the new umbrella); left for
## back-compat but now aliases to test-unit.
check-fast: test-unit
check-tier0:
	bash tests/tools/tier0-first-pass.sh

# ===========================================================================
# Umbrella test targets (local / agent development).
#
#   make test-unit        L1 gate sub-targets (lint, rust, proofs, flake, drift,
#                         policy), run through the same manifest as CI.
#   make test             test-unit + test-integration (full local gate).
#   make test-integration L2 podman container integration tests.
#
# CI and local runs share tests/layer1-jobs.json. Locally, D2B_CHECK_JOBS bounds
# parallel sub-targets; CI renders .github/workflows/pr-l1-static-fast.yml from
# the same manifest.
# ===========================================================================

test: test-unit test-integration

test-unit:
	bash tests/tools/layer1-jobs run-local --skip-preflight

# ===========================================================================
# Sub-targets. Each has a corresponding tests/test-<name>.sh driver.
# ===========================================================================

## test-lint - preflight + nix-instantiate --parse + shellcheck (no eval, no cargo).
test-lint:
	bash tests/test-lint.sh

###############################################################################
# Rust DAG and resource budget.
#
# GNU Make owns the dependency graph. The recursive invocation is deliberately
# marked with +$(MAKE), so the jobserver reaches the scheduler. Leaf recipes
# below are ordinary non-submake recipes; Make closes its jobserver
# descriptors before the leaf shell starts.
#
# D2B_RUST_BUDGET is a positive requested upper bound. Invalid values are
# redacted, require a positive integer, and exit 2. Automatic sizing takes
# the smaller of logical CPUs and the memory cap derived from MemAvailable and
# cache-adjusted finite cgroup v2 memory.max or memory.high. It reserves 2 GiB
# for the host and budgets 3 GiB per heavy job. A visible but unreadable
# cgroup controller fails closed to budget 1 rather than guessing.
#
# When D2B_EXECUTION_MANIFEST is set, the plumbing helper removes the prior
# evidence before dispatch, holds the persistent execution-manifest lock, and
# publishes the deterministic v1 result after the scheduler exits. Its
# execution-manifest clock injection and process/path test boundaries are
# internal lifecycle hooks for hermetic tests;
# production exposes no shutdown grace knob.
###############################################################################

D2B_RUST_QUOTA_API ?= 1
D2B_RUST_QUOTA_MAIN ?= 1
D2B_RUST_QUOTA_SCHEMA ?= 1
D2B_RUST_QUOTA_INVENTORY ?= 1
D2B_RUST_QUOTA_FIXTURE ?= 1
D2B_RUST_QUOTA_BROKER ?= 1
D2B_RUST_QUOTA_GUEST ?= 1
D2B_RUST_QUOTA_AST ?= 1
D2B_RUST_QUOTA_SUPPLY ?= 1
D2B_RUST_PROFILE ?= aggregate
D2B_RUST_MAIN_PREREQS_aggregate := test-rust-leaf-schema
D2B_RUST_MAIN_PREREQS_cold :=
D2B_RUST_MAIN_PREREQS_api :=
D2B_RUST_MAIN_PREREQS_main :=
D2B_RUST_MAIN_PREREQS := $(D2B_RUST_MAIN_PREREQS_$(D2B_RUST_PROFILE))
D2B_RUST_SCHEMA_PREREQS_aggregate := test-rust-leaf-inventory
D2B_RUST_SCHEMA_PREREQS_cold := test-rust-leaf-inventory
D2B_RUST_SCHEMA_PREREQS_schema :=
D2B_RUST_SCHEMA_PREREQS := $(D2B_RUST_SCHEMA_PREREQS_$(D2B_RUST_PROFILE))
D2B_RUST_BROKER_PREREQS_aggregate := test-rust-leaf-inventory
D2B_RUST_BROKER_PREREQS_cold :=
D2B_RUST_BROKER_PREREQS_broker :=
D2B_RUST_BROKER_PREREQS := $(D2B_RUST_BROKER_PREREQS_$(D2B_RUST_PROFILE))
D2B_RUST_FIXTURE_PREREQS_aggregate :=
D2B_RUST_FIXTURE_PREREQS_cold := test-rust-leaf-api-surface test-rust-leaf-main-workspace test-rust-leaf-broker test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast test-rust-leaf-supply-chain
D2B_RUST_FIXTURE_PREREQS_main :=
D2B_RUST_FIXTURE_PREREQS := $(D2B_RUST_FIXTURE_PREREQS_$(D2B_RUST_PROFILE))
D2B_RUST_INVENTORY_PREREQS_aggregate :=
D2B_RUST_INVENTORY_PREREQS_cold := test-rust-leaf-fixture-contracts
D2B_RUST_INVENTORY_PREREQS_inventory :=
D2B_RUST_INVENTORY_PREREQS := $(D2B_RUST_INVENTORY_PREREQS_$(D2B_RUST_PROFILE))

ifeq ($(D2B_SKIP_FIXTURE_BUILD),1)
D2B_RUST_MAIN_LEAVES := test-rust-leaf-main-workspace
else
D2B_RUST_MAIN_LEAVES := test-rust-leaf-main-workspace test-rust-leaf-fixture-contracts
endif

# Execution-manifest v1 evidence is opt-in and starts before the recursive
# scheduler. The helper anchors the manifest parent first with openat2
# RESOLVE_NO_SYMLINKS and RESOLVE_NO_MAGICLINKS, then opens the persistent
# current-user mode-0600 `.lock` with O_CLOEXEC and O_NOFOLLOW and acquires
# nonblocking F_OFD_SETLK. It marks every evidence descriptor FD_CLOEXEC.
# `manifest-lock-contended` telemetry has one path-free diagnostic:
# execution-manifest lock is active; wait for the active run to finish and
# retry. The remedy never names a filesystem path.
#
# Fragment plumbing creates an adjacent same-filesystem current-user mode-0700
# directory with mkdirat mode 0700, rejects owner or mode mismatches with fstat,
# st_uid and current uid checks, removes only the prior manifest and verified stale
# entries, and uses unlinkat for anchored fd-relative cleanup. Invalid stale
# paths are skipped with continue. Complete leaf fragments are atomically renamed,
# and the final versioned manifest is atomically published. Failed and
# interrupted runs publish partial completed_leaves and failed_surfaces evidence
# with run_status failed or interrupted; passed runs publish run_status passed.
# Policy tests cover scheduler success, failure, interruption, and
# finalization-error paths. Failed and interrupted finalization publishes an
# atomic manifest replacement.
#
# The scheduler has a dedicated process group created with setsid. Handled
# SIGTERM or SIGINT is forwarded, waited for up to the fixed 10 seconds, then
# remaining children receive SIGKILL and are reaped before idempotent
# finalization. Close evidence file descriptors before exec. Clock,
# process-control, and path boundaries are injectable for hermetic tests;
# the preserved status is returned after finalization publishes evidence;
# production has no public grace knob.
define D2B_RUST_DISPATCH
set -eu; \
requested="$${D2B_RUST_BUDGET:-}"; \
if [ -n "$$requested" ]; then \
  case "$$requested" in \
    ''|*[!0-9]*) echo "D2B_RUST_BUDGET must be a positive integer (value redacted)." >&2; exit 2 ;; \
  esac; \
  if [ "$$requested" -lt 1 ]; then \
    echo "D2B_RUST_BUDGET must be a positive integer (value redacted)." >&2; exit 2; \
  fi; \
fi; \
logical_cpus="$$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || printf '1')"; \
case "$$logical_cpus" in ''|*[!0-9]*) logical_cpus=1 ;; esac; \
[ "$$logical_cpus" -ge 1 ] || logical_cpus=1; \
mem_available_kib="$$(awk '/^MemAvailable:/ { print $$2; exit }' /proc/meminfo 2>/dev/null || true)"; \
case "$$mem_available_kib" in ''|*[!0-9]*) mem_available_kib=0 ;; esac; \
host_available_bytes=$$((mem_available_kib * 1024)); \
cgroup_v2=0; \
if grep -q '^0::' /proc/self/cgroup 2>/dev/null; then cgroup_v2=1; fi; \
cgroup_unreadable=0; \
cgroup_dir=; \
if [ "$$cgroup_v2" -eq 1 ]; then \
  cgroup_mount="$$(awk '$$0 ~ / - cgroup2 / { print $$5; exit }' /proc/self/mountinfo 2>/dev/null || true)"; \
  cgroup_relative="$$(sed -n 's/^0:://p' /proc/self/cgroup | head -1)"; \
  if [ -z "$$cgroup_mount" ] || [ -z "$$cgroup_relative" ]; then \
    cgroup_unreadable=1; \
  else \
    cgroup_dir="$$cgroup_mount$$cgroup_relative"; \
  fi; \
  if [ "$$cgroup_unreadable" -eq 0 ] && { \
    [ ! -r "$$cgroup_dir/memory.max" ] || \
    [ ! -r "$$cgroup_dir/memory.high" ] || \
    [ ! -r "$$cgroup_dir/memory.current" ] || \
    [ ! -r "$$cgroup_dir/memory.stat" ]; \
  }; then cgroup_unreadable=1; fi; \
fi; \
available_bytes="$$host_available_bytes"; \
if [ "$$cgroup_v2" -eq 1 ] && [ "$$cgroup_unreadable" -eq 0 ]; then \
  memory_max="$$(cat "$$cgroup_dir/memory.max" 2>/dev/null || true)"; \
  memory_high="$$(cat "$$cgroup_dir/memory.high" 2>/dev/null || true)"; \
  memory_current="$$(cat "$$cgroup_dir/memory.current" 2>/dev/null || true)"; \
  inactive_file="$$(awk '$$1 == "inactive_file" { print $$2; exit }' "$$cgroup_dir/memory.stat" 2>/dev/null || true)"; \
  case "$$memory_max" in max|*[!0-9]*) [ "$$memory_max" = max ] || cgroup_unreadable=1 ;; esac; \
  case "$$memory_high" in max|*[!0-9]*) [ "$$memory_high" = max ] || cgroup_unreadable=1 ;; esac; \
  case "$$memory_current" in ''|*[!0-9]*) cgroup_unreadable=1 ;; esac; \
  case "$$inactive_file" in ''|*[!0-9]*) inactive_file=0 ;; esac; \
  if [ "$$cgroup_unreadable" -eq 0 ]; then \
    cached_usage=$$((memory_current - inactive_file)); \
    [ "$$cached_usage" -lt 0 ] && cached_usage=0; \
    remaining_bytes=; \
    if [ "$$memory_max" != max ]; then remaining_bytes=$$((memory_max - cached_usage)); fi; \
    if [ "$$memory_high" != max ]; then \
      high_remaining=$$((memory_high - cached_usage)); \
      if [ -z "$$remaining_bytes" ] || [ "$$high_remaining" -lt "$$remaining_bytes" ]; then remaining_bytes=$$high_remaining; fi; \
    fi; \
    if [ -n "$$remaining_bytes" ]; then \
      [ "$$remaining_bytes" -lt 0 ] && remaining_bytes=0; \
      if [ "$$remaining_bytes" -lt "$$available_bytes" ] || [ "$$available_bytes" -eq 0 ]; then available_bytes="$$remaining_bytes"; fi; \
    fi; \
  fi; \
fi; \
if [ "$$cgroup_unreadable" -ne 0 ]; then \
  echo "Rust budget: cgroup v2 controller visibility is unreadable; failing closed to budget 1. Fix controller visibility or run outside the constrained environment." >&2; \
  effective_budget=1; \
else \
  reserve_bytes=$$((2 * 1024 * 1024 * 1024)); \
  per_heavy_job_bytes=$$((3 * 1024 * 1024 * 1024)); \
  if [ "$$available_bytes" -le "$$reserve_bytes" ]; then \
    memory_cap=1; \
  else \
    memory_cap=$$(( (available_bytes - reserve_bytes) / per_heavy_job_bytes )); \
    [ "$$memory_cap" -ge 1 ] || memory_cap=1; \
  fi; \
  effective_budget="$$logical_cpus"; \
  [ "$$memory_cap" -lt "$$effective_budget" ] && effective_budget="$$memory_cap"; \
  if [ -n "$$requested" ] && [ "$$requested" -lt "$$effective_budget" ]; then effective_budget="$$requested"; fi; \
fi; \
[ "$$effective_budget" -ge 1 ] || effective_budget=1; \
runtime_budget="$$effective_budget"; \
profile='$(2)'; \
cold_profile=0; \
if [ "$$profile" = aggregate ] && [ ! -d packages/target ]; then \
  profile=cold; \
  cold_profile=1; \
fi; \
quota_api=1; \
quota_main=1; \
quota_schema=1; \
quota_inventory=1; \
quota_fixture=1; \
quota_broker=1; \
quota_guest=1; \
quota_ast=1; \
quota_supply=1; \
case "$$profile" in \
  aggregate) \
    lane_count=9; \
    active_lanes="$$runtime_budget"; \
    [ "$$active_lanes" -le "$$lane_count" ] || active_lanes="$$lane_count"; \
    if [ "$$runtime_budget" -gt "$$lane_count" ]; then \
      quota_api=$$((runtime_budget - lane_count + 1)); \
    fi; \
    frontier_quota=$$((quota_api + active_lanes - 1)); \
    ;; \
  cold) \
    active_lanes="$$runtime_budget"; \
    [ "$$active_lanes" -le 4 ] || active_lanes=4; \
    quota_api=1; \
    quota_main=1; \
    quota_broker=1; \
    surplus=$$((runtime_budget - active_lanes)); \
    turn=0; \
    while [ "$$surplus" -gt 0 ]; do \
      case $$((turn % 3)) in \
        0) quota_main=$$((quota_main + 1)) ;; \
        1) quota_broker=$$((quota_broker + 1)) ;; \
        2) quota_api=$$((quota_api + 1)) ;; \
      esac; \
      turn=$$((turn + 1)); \
      surplus=$$((surplus - 1)); \
    done; \
    quota_schema="$$runtime_budget"; \
    quota_inventory="$$runtime_budget"; \
    quota_fixture="$$runtime_budget"; \
    if [ "$$active_lanes" -lt 3 ]; then \
      frontier_quota="$$active_lanes"; \
    elif [ "$$active_lanes" -eq 3 ]; then \
      frontier_quota=$$((quota_api + quota_main + quota_broker)); \
    else \
      frontier_quota=$$((quota_api + quota_main + quota_broker + 1)); \
    fi; \
    ;; \
  api) \
    active_lanes=1; \
    quota_api="$$runtime_budget"; \
    frontier_quota="$$quota_api"; \
    ;; \
  main) \
    if [ "$${D2B_SKIP_FIXTURE_BUILD:-0}" = 1 ]; then \
      active_lanes=1; \
      quota_main="$$runtime_budget"; \
      frontier_quota="$$quota_main"; \
    else \
      active_lanes=2; \
      [ "$$active_lanes" -le "$$runtime_budget" ] || active_lanes="$$runtime_budget"; \
      if [ "$$runtime_budget" -eq 1 ]; then \
        quota_main=1; \
        quota_fixture=1; \
        frontier_quota=1; \
      else \
        quota_fixture=$$((runtime_budget / 2)); \
        quota_main=$$((runtime_budget - quota_fixture)); \
        frontier_quota=$$((quota_main + quota_fixture)); \
      fi; \
    fi; \
    ;; \
  broker) \
    active_lanes=1; \
    quota_broker="$$runtime_budget"; \
    frontier_quota="$$quota_broker"; \
    ;; \
  guest) \
    active_lanes=1; \
    quota_guest="$$runtime_budget"; \
    frontier_quota="$$quota_guest"; \
    ;; \
  no-bash) \
    active_lanes=1; \
    quota_ast="$$runtime_budget"; \
    frontier_quota="$$quota_ast"; \
    ;; \
  schema) \
    active_lanes=1; \
    quota_schema="$$runtime_budget"; \
    frontier_quota="$$quota_schema"; \
    ;; \
  inventory) \
    active_lanes=1; \
    quota_inventory="$$runtime_budget"; \
    frontier_quota="$$quota_inventory"; \
    ;; \
  supply) \
    active_lanes=1; \
    quota_supply="$$runtime_budget"; \
    frontier_quota="$$quota_supply"; \
    ;; \
  *) echo "internal Rust profile is invalid" >&2; exit 2 ;; \
esac; \
test "$$frontier_quota" -le "$$runtime_budget" || { echo "frontier quota exceeds runtime budget" >&2; exit 1; }; \
printf '%s\n' "Rust effective runtime budget: $$effective_budget job(s), $$active_lanes active lane(s), $$profile profile; D2B_RUST_BUDGET is the requested upper-bound control."; \
set +e; \
if [ -n "$${D2B_EXECUTION_MANIFEST:-}" ]; then \
  perl tests/tools/execution-manifest.pl run \
    --manifest "$$D2B_EXECUTION_MANIFEST" \
    --target test-rust \
    --commit "$$(git rev-parse --verify HEAD 2>/dev/null || printf '%s' unknown)" \
    -- "$(MAKE)" --keep-going --output-sync=target --no-print-directory -j "$$active_lanes" \
      "D2B_RUST_ROOT_PREREQS=0" \
      "D2B_RUST_PROFILE=$$profile" \
      "D2B_RUST_COLD_PROFILE=$$cold_profile" \
      "D2B_RUST_EFFECTIVE_BUDGET=$$effective_budget" \
      "D2B_RUST_ACTIVE_LANES=$$active_lanes" \
      "D2B_RUST_QUOTA_API=$$quota_api" \
      "D2B_RUST_QUOTA_MAIN=$$quota_main" \
      "D2B_RUST_QUOTA_SCHEMA=$$quota_schema" \
      "D2B_RUST_QUOTA_INVENTORY=$$quota_inventory" \
      "D2B_RUST_QUOTA_FIXTURE=$$quota_fixture" \
      "D2B_RUST_QUOTA_BROKER=$$quota_broker" \
      "D2B_RUST_QUOTA_GUEST=$$quota_guest" \
      "D2B_RUST_QUOTA_AST=$$quota_ast" \
      "D2B_RUST_QUOTA_SUPPLY=$$quota_supply" \
      $(1); \
  rust_dispatch_rc="$$?"; \
else \
  "$(MAKE)" --keep-going --output-sync=target --no-print-directory -j "$$active_lanes" \
    "D2B_RUST_ROOT_PREREQS=0" \
    "D2B_RUST_PROFILE=$$profile" \
    "D2B_RUST_COLD_PROFILE=$$cold_profile" \
    "D2B_RUST_EFFECTIVE_BUDGET=$$effective_budget" \
    "D2B_RUST_ACTIVE_LANES=$$active_lanes" \
    "D2B_RUST_QUOTA_API=$$quota_api" \
    "D2B_RUST_QUOTA_MAIN=$$quota_main" \
    "D2B_RUST_QUOTA_SCHEMA=$$quota_schema" \
    "D2B_RUST_QUOTA_INVENTORY=$$quota_inventory" \
    "D2B_RUST_QUOTA_FIXTURE=$$quota_fixture" \
    "D2B_RUST_QUOTA_BROKER=$$quota_broker" \
    "D2B_RUST_QUOTA_GUEST=$$quota_guest" \
    "D2B_RUST_QUOTA_AST=$$quota_ast" \
    "D2B_RUST_QUOTA_SUPPLY=$$quota_supply" \
    $(1); \
  rust_dispatch_rc="$$?"; \
fi; \
set -e; \
exit "$$rust_dispatch_rc"
endef

## test-rust - the bounded Make-owned Rust DAG. The prerequisite list is kept
## explicit for policy and inventory checks; its recipes are discovery no-ops
## while the recursive scheduler runs the same graph with the calculated
## budget. This keeps one scheduler in charge of the real leaves.
test-rust: test-rust-leaf-api-surface test-rust-leaf-main-workspace test-rust-leaf-schema test-rust-leaf-inventory test-rust-leaf-fixture-contracts test-rust-leaf-broker test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast test-rust-leaf-supply-chain
	@# D2B_EXECUTION_MANIFEST is removed by the lifecycle helper before dispatch.
	@# The recursive scheduler invokes +$(MAKE) --keep-going --output-sync=target.
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-api-surface test-rust-leaf-main-workspace test-rust-leaf-fixture-contracts test-rust-leaf-broker test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast test-rust-leaf-schema test-rust-leaf-supply-chain test-rust-leaf-inventory,aggregate)

test-rust: D2B_RUST_ROOT_PREREQS := 1

## Stable CI shard targets. Local callers should prefer make test-rust.
test-rust-api-surface:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-api-surface,api)

test-rust-main:
	+@$(call D2B_RUST_DISPATCH,$(D2B_RUST_MAIN_LEAVES),main)

test-rust-broker:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-broker,broker)

test-rust-guest-shell-runner:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-guest-shell-runner,guest)

test-rust-no-bash-ast:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-no-bash-ast,no-bash)

test-rust-schema:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-schema,schema)

test-rust-inventory:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-inventory,inventory)

test-rust-supply-chain:
	+@$(call D2B_RUST_DISPATCH,test-rust-leaf-supply-chain,supply)

## Leaf recipes are ordinary non-submake recipes. When they are seen as
## prerequisites of the outer test-rust declaration they intentionally do no
## work; the recursive child owns the real leaf dispatch.
test-rust-leaf-api-surface:
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_API)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_API)" bash tests/test-rust.sh api-surface

test-rust-leaf-main-workspace: $(D2B_RUST_MAIN_PREREQS)
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_MAIN)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_MAIN)" bash tests/test-rust.sh main-workspace

test-rust-leaf-schema: $(D2B_RUST_SCHEMA_PREREQS)
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_SCHEMA)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_SCHEMA)" bash tests/test-rust.sh schema-reproducibility

test-rust-leaf-inventory: $(D2B_RUST_INVENTORY_PREREQS)
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_INVENTORY)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_INVENTORY)" bash tests/test-rust.sh inventory-stub

## Fixture and CLI surfaces use a stable isolated target directory under
## .scratch/rust-test-cache, so their Nix and Cargo work can overlap the main
## workspace without sharing mutable Cargo state.
test-rust-leaf-fixture-contracts: $(D2B_RUST_FIXTURE_PREREQS)
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; \
	if [ "$(D2B_SKIP_FIXTURE_BUILD)" = 1 ]; then \
	  echo "Rust fixture/CLI surfaces skipped (D2B_SKIP_FIXTURE_BUILD=1; run the enforcing fixture lane separately)."; \
	elif command -v nix >/dev/null 2>&1; then \
	  D2B_ENABLE_FIXTURE_BUILD=1 D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_FIXTURE)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_FIXTURE)" bash tests/test-rust.sh fixture-contracts; \
	else \
	  echo "Rust fixture/CLI surfaces skipped (nix unavailable)."; \
	fi

test-rust-leaf-broker: $(D2B_RUST_BROKER_PREREQS)
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_BROKER)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_BROKER)" bash tests/test-rust.sh broker

test-rust-leaf-guest-shell-runner:
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_GUEST)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_GUEST)" bash tests/test-rust.sh guest-shell-runner

test-rust-leaf-no-bash-ast:
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_AST)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_AST)" bash tests/test-rust.sh no-bash-ast

test-rust-leaf-supply-chain:
	@if [ "$(D2B_RUST_ROOT_PREREQS)" = 1 ]; then exit 0; fi; D2B_RUST_CARGO_JOBS="$(D2B_RUST_QUOTA_SUPPLY)" D2B_RUST_NEXTEST_THREADS="$(D2B_RUST_QUOTA_SUPPLY)" bash tests/test-rust.sh supply-chain



## test-fixture-contracts - enforcing eval-rendered contract and CLI layer.
## Layer-1 local and CI orchestration set D2B_ENABLE_FIXTURE_BUILD=1.
test-fixture-contracts:
	bash tests/test-rust.sh fixture-contracts

## test-proofs - standalone proof crates under proofs/ (not members of packages/).
test-proofs:
	bash tests/test-proofs.sh

## test-flake - `nix flake check --no-build` for the native system (bounded
## memory). CI shards the x86_64 leg one-job-per-check via a dynamic matrix:
## set D2B_FLAKE_CHECK=<name> to instantiate just that one check (the matrix
## enumerates names with `make test-flake-list`); the aarch64 PR leg runs only a
## lightweight smoke eval. Set D2B_FLAKE_ALL_SYSTEMS=1 to cross-evaluate every
## system locally (like `make check`/static.sh).
test-flake:
	bash tests/test-flake.sh

## test-flake-list - emit the native-system flake check names as a JSON array on
## stdout (CI dynamic-matrix plumbing for the sharded test-flake; see
## .github/workflows/pr-l1-static-fast.yml). Invoke as `make -s test-flake-list`.
test-flake-list:
	@bash tests/test-flake-list.sh

## test-flake-partition - emit the native-system flake check names split into
## the three CI dispatch classes as `<key>=<json-array>` lines on stdout (CI
## dynamic-matrix plumbing for the sharded test-flake; see
## .github/workflows/pr-l1-static-fast.yml). Invoke as
## `make -s test-flake-partition`.
test-flake-partition:
	@bash tests/tools/flake-check-partition.sh

## test-nix-unit - build all sharded nix-unit corpus checks. Kept as explicit
## Layer-1 evidence even though test-flake also evaluates the checks.
test-nix-unit:
	bash tests/test-nix-unit.sh

## api-surface-pin - explicitly regenerate compiler-derived API snapshots.
api-surface-pin:
	D2B_API_SURFACE_UPDATE=1 bash tests/tools/api-surface-json.sh

## test-drift - generated-artifact drift gates (xtask gen-*, vms-json parity).
test-drift:
	bash tests/test-drift.sh

## test-policy - meta gates that guard the test architecture + cross-cutting
## invariants (ci-coverage, adr-index, deliverable-gate, etc.).
##
## The Provider crate layout policies run here as xtask commands, not as
## shell gates: the drift and meta gate set is closed.
test-policy:
	bash tests/test-policy.sh
	cd packages && cargo run --quiet -p xtask -- check-provider-crate-layout
	cd packages && cargo run --quiet -p xtask -- check-provider-layout

## test-performance-budgets - execute the self-gating performance canary.
## Hosted runners take the cheap skip path; pinned stable runners enforce it.
test-performance-budgets:
	bash tests/unit/gates/performance-budgets.sh

## Focused policy entrypoints used by the early CI preflight.
test-adr-index-coverage:
	bash tests/unit/meta/adr-index-coverage.sh

test-ci-coverage:
	bash tests/unit/meta/ci-coverage.sh

## test-integration - L2 podman container integration tests. Public heavy lane:
## it acquires a heavy-gate slot, then runs the raw work behind the gate so it
## can never oversubscribe a concurrent lane, even when invoked directly or via
## `make test` / `check-ci` / `check-all`.
test-integration: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-integration

## heavy-lane-integration - the raw L2 container work. Internal: reachable only
## from inside the gate (see heavy-lane-guard).
heavy-lane-integration: heavy-lane-guard
	bash tests/test-integration.sh

## layer1-workflow - regenerate the Layer-1 PR workflow from tests/layer1-jobs.json.
layer1-workflow:
	bash tests/tools/layer1-jobs render-workflow --write

## layer1-workflow-check - fail if the generated Layer-1 PR workflow is stale.
layer1-workflow-check:
	bash tests/tools/layer1-jobs check-workflow

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## check-inventory - fail-closed ledger drift check for CI.
check-inventory:
	bash tests/tools/gen-migration-ledger.sh --check

## ledger-regen - regenerate tests/migration-ledger.toml in place for humans.
ledger-regen:
	bash tests/tools/gen-migration-ledger.sh

## nix-unit-pin - regenerate the fail-closed nix-unit case-presence pins
## (tests/unit/nix/pinned/*.txt) after adding or removing cases.
nix-unit-pin:
	bash tests/tools/gen-nix-unit-pins.sh

## flake-matrix-pin - regenerate the fail-closed CI flake-check-matrix pin
## (tests/golden/flake-check-matrix/<system>.txt) after adding/removing a flake
## check. The drift gate (run by `make test-drift`) fails closed until this is
## rerun, so the sharded x86 CI matrix can't silently change coverage.
flake-matrix-pin:
	bash tests/tools/gen-flake-check-matrix-pin.sh

## W0 policy gate (also run by test-policy).
pr-checklist-gate:
	bash tests/unit/meta/pr-checklist-gate.sh .github/PULL_REQUEST_TEMPLATE.md

## test-host-integration - G-host: runNixOSTest VM integration tests (the
## `vmChecks` flake output, NOT swept by `nix flake check`). Each test boots a
## real NixOS VM with the d2b daemon surface and asserts live broker /
## daemon / host-posture behaviour (socket activation, bridge isolation,
## state-dir ACLs, broker privilege posture) - the hermetic, non-destructive
## successor to the `D2B_LIVE`-against-the-real-host scripts. Needs KVM (a local
## NixOS host; TCG software emulation is the slow fallback when /dev/kvm is
## absent). x86_64-linux only (a same-system VM builder is required).
## Public heavy lane: acquires a slot, then runs the raw work behind the gate.
test-host-integration: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-host-integration

heavy-lane-host-integration: heavy-lane-guard
	@set -eu; \
	system="$$(nix eval --raw --impure --expr builtins.currentSystem)"; \
	if [ "$$system" != "x86_64-linux" ]; then \
	echo "test-host-integration: vmChecks are x86_64-linux only (need a same-system VM builder); skipping on $$system"; \
	exit 0; \
	fi; \
	if [ ! -e /dev/kvm ]; then \
	echo "test-host-integration: /dev/kvm absent - runNixOSTest will fall back to slow TCG emulation"; \
	fi; \
	root="$$(pwd)"; \
	names="$$(nix eval --raw --impure --no-warn-dirty --expr "builtins.concatStringsSep \" \" (builtins.attrNames (builtins.getFlake \"git+file://$$root\").vmChecks.$$system)")"; \
	if [ -z "$$names" ]; then \
	echo "test-host-integration: no vmChecks present"; \
	exit 0; \
	fi; \
	echo "test-host-integration: building vmChecks: $$names"; \
	for name in $$names; do \
	echo "==> nix build .#vmChecks.$$system.$$name"; \
	nix build --no-link --print-build-logs ".#vmChecks.$$system.$$name"; \
	done
## test-hardware - G-hw: real GPU/YubiKey/hardware-TPM passthrough + full
## microVM boot. NixOS host WITH the devices only; CI cannot run this.
## Public heavy lanes: acquire a slot, then run the raw work behind the gate.
test-hardware: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-hardware
perf: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-perf

heavy-lane-hardware: heavy-lane-guard
	bash tests/tools/run-layer.sh test-hardware
heavy-lane-perf: heavy-lane-guard
	bash tests/tools/run-layer.sh perf

## heavy-lane-guard - fail closed when a heavy-lane internal target is invoked
## outside the gate. It does not trust the mere presence of D2B_HEAVY_GATE
## (any process can export that); instead it asks the wrapper to verify that
## this process genuinely holds a slot via its open file description lock.
##
## verify-slot reports its verdict purely through its exit status, so branch on
## the typed codes rather than collapsing every nonzero status into one:
##
##   0  a genuinely held slot            -> proceed
##   3  no slot is held                  -> reacquire by running the PUBLIC lane
##                                          (which acquires a slot and re-runs
##                                          this lane through the gate). A shared
##                                          prerequisite cannot exec that itself
##                                          without double-running the parent
##                                          recipe, so guide the operator to the
##                                          acquiring entrypoint and fail closed
##                                          with the typed unheld code.
##   *  the verifier itself malfunctioned -> propagate the exact code unchanged
##                                          and fail closed
##
## Collapsing 3 and every malfunction into one "outside the semaphore" exit hid
## a broken gate behind a slot-bypass message; keeping the codes distinct lets a
## caller tell "ran the raw target directly" apart from "the verifier is broken".
heavy-lane-guard: heavy-gate-build
	@rc=0; $(HEAVY_GATE_BIN) heavy-gate verify-slot || rc=$$?; \
	if [ "$$rc" -eq 0 ]; then \
	  exit 0; \
	elif [ "$$rc" -eq 3 ]; then \
	  echo "heavy lane invoked outside the heavy-gate semaphore (no slot held)." >&2; \
	  echo "Run the public lane (e.g. 'make test-integration'), which acquires a slot" >&2; \
	  echo "and re-runs this lane through the gate; do not run the internal target directly." >&2; \
	  exit "$$rc"; \
	else \
	  echo "heavy-gate verify-slot failed closed (exit $$rc); refusing to run heavy work unsynchronised." >&2; \
	  exit "$$rc"; \
	fi

# ===========================================================================
# Heavy lanes.
#
# Every Layer-2, host-integration, hardware, live, and perf-heavy command runs
# through ONE semaphore: `cargo xtask heavy-gate`. It grants two slots per uid
# via open file description locks, so concurrent lanes cannot oversubscribe the
# shared Nix store, cargo target directory, or KVM device. Do not add a second
# lock file, sleep-and-retry loop, or per-crate heavy-lane guard.
#
# Run the heavy-* target instead of the bare target whenever another heavy lane
# might be running; the bare targets stay available for a serial console.
# ===========================================================================

# Normalize CARGO_TARGET_DIR to an absolute path so the wrapper is built and
# executed at the same location. cargo runs the build from packages/, so a
# *relative* CARGO_TARGET_DIR is interpreted relative to packages/ - but
# HEAVY_GATE is invoked from the repo root, so a bare relative path is looked up
# in the wrong place (packages/relative/debug/xtask built, relative/debug/xtask
# executed). Resolve a relative value against packages/ and pass the resolved
# absolute path back to cargo, so both the build and the execution agree
# regardless of the caller's value.
ifeq ($(CARGO_TARGET_DIR),)
HEAVY_GATE_TARGET_DIR := $(CURDIR)/packages/target
else ifeq ($(filter /%,$(CARGO_TARGET_DIR)),)
HEAVY_GATE_TARGET_DIR := $(abspath $(CURDIR)/packages/$(CARGO_TARGET_DIR))
else
HEAVY_GATE_TARGET_DIR := $(CARGO_TARGET_DIR)
endif
HEAVY_GATE_BIN := $(HEAVY_GATE_TARGET_DIR)/debug/xtask
HEAVY_GATE = $(HEAVY_GATE_BIN) heavy-gate --

## heavy-gate-build - build the semaphore wrapper. Runs from packages/ so the
## workspace cargo config (and its rustc wrapper) applies. The build target dir
## is forced to the same absolute HEAVY_GATE_TARGET_DIR the wrapper is executed
## from, so a relative CARGO_TARGET_DIR cannot split the two.
heavy-gate-build:
	@cd packages && CARGO_TARGET_DIR='$(HEAVY_GATE_TARGET_DIR)' cargo build --quiet -p xtask

## heavy-gate-provision - create or repair the protected slot namespace for the
## current numeric uid without resolving a user name through NSS. This is the
## explicit post-login path for network-backed users and the developer setup
## path on hosts that do not consume the NixOS module. Because /run is a tmpfs,
## run it once per boot when the gate reports missing provisioning. It never
## creates a fallback under a user-owned root.
heavy-gate-provision:
	@target_uid="$$(id -u)"; \
	sudo -- sh -eu -c '\
	  target_uid="$$1"; root=/run/d2b-heavy-gates; \
	  case "$$target_uid" in ""|*[!0-9]*) echo "heavy-gate provisioning: invalid target uid" >&2; exit 1;; esac; \
	  if [ -L "$$root" ] || { [ -e "$$root" ] && [ ! -d "$$root" ]; }; then echo "heavy-gate provisioning: refusing an unsafe runtime root" >&2; exit 1; fi; \
	  install -d -m 0755 -o root -g root "$$root"; \
	  uid_dir="$$root/uid-$$target_uid"; \
	  if [ -L "$$uid_dir" ] || { [ -e "$$uid_dir" ] && [ ! -d "$$uid_dir" ]; }; then echo "heavy-gate provisioning: refusing an unsafe per-user directory" >&2; exit 1; fi; \
	  install -d -m 0755 -o root -g root "$$uid_dir"; \
	  for index in 0 1; do \
	    slot="$$uid_dir/slot-$$index"; \
	    if [ -L "$$slot" ] || { [ -e "$$slot" ] && [ ! -f "$$slot" ]; }; then echo "heavy-gate provisioning: refusing an unsafe slot file" >&2; exit 1; fi; \
	    if [ ! -e "$$slot" ]; then install -m 0600 -o "$$target_uid" -g root /dev/null "$$slot"; else chown "$$target_uid":root "$$slot"; chmod 0600 "$$slot"; fi; \
	  done; \
	  echo "heavy-gate provisioning: protected slots are ready for this boot"' \
	  sh "$$target_uid"

## heavy-check - the Layer-1 PR-equivalent gate under the heavy-lane semaphore.
heavy-check: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) check

## heavy-test-integration / -host-integration / -hardware - explicit aliases for
## the public heavy lanes, kept for muscle memory and scripts. The public lanes
## now acquire the semaphore themselves; a redundant outer gate here is safe
## because the inner invocation verifies and reuses the inherited slot.
heavy-test-integration: test-integration
heavy-test-host-integration: test-host-integration
heavy-test-hardware: test-hardware

## heavy-cargo-test - the Rust workspace test suite under the semaphore.
##                    Override the selector with HEAVY_CARGO_TEST_ARGS.
HEAVY_CARGO_TEST_ARGS ?= --workspace --all-targets
heavy-cargo-test: heavy-gate-build
	cd packages && $(HEAVY_GATE) cargo test $(HEAVY_CARGO_TEST_ARGS)

## heavy-flake-check - the building `nix flake check` under the semaphore.
##                     `make test-flake` is the cheap --no-build sibling.
heavy-flake-check: heavy-gate-build
	$(HEAVY_GATE) $(NIX_FLAKE) flake check --print-build-logs

# --- pre-existing maintainer targets ---------------------------------------

## i3-check - verify no v1.3 deferrals authored (ADR 0022 I3 invariant).
##            Wired into pre-tag and tests/static.sh per panel-docs R1 MF-1.
i3-check:
	bash tests/unit/meta/no-new-deferral.sh

## pre-tag - run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, d2b active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates $${TMPDIR:-/tmp}/d2b-smoke-run-log.txt.
##           ALSO runs the I3 invariant grep gate (ADR 0022 + panel-docs R1).
##           Public heavy lane: acquires a slot, then runs the raw live work behind
##           the gate - the live smoke suite is the most destructive, stateful lane
##           in the tree and must never bypass the sole-use semaphore.
pre-tag: i3-check heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-pre-tag

## heavy-lane-pre-tag - the raw full live-VM smoke work. Internal: reachable only
## from inside the gate (see heavy-lane-guard).
heavy-lane-pre-tag: heavy-lane-guard
	bash tests/integration/live/live-vm-smoke.sh --full

## smoke-lite - run the single-VM lite smoke gate (≤5 min).
##              Used at every panel-round HEAD per I5.
##              Public heavy lane: acquires a slot, then runs the raw live work
##              behind the gate.
smoke-lite: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-smoke-lite

## heavy-lane-smoke-lite - the raw lite live-VM smoke work. Internal: reachable
## only from inside the gate (see heavy-lane-guard).
heavy-lane-smoke-lite: heavy-lane-guard
	bash tests/integration/live/live-vm-smoke.sh --lite

.PHONY: test-changelog changelog-fold

## test-changelog - the changelog policy gate (also the CI test-changelog job).
##                  Requires code changes to ship release notes as either a
##                  CHANGELOG.md entry or a changelog.d/ fragment, and validates
##                  the structure of every fragment present.
test-changelog:
	bash scripts/changelog-check.sh

## changelog-fold - fold every changelog.d/ fragment into the CHANGELOG.md
##                  '## [Unreleased]' block and delete the consumed fragments.
##                  Run at merge time; see changelog.d/README.md.
changelog-fold:
	cd packages && cargo run -q -p xtask -- changelog-fold
# --- hermetic execution-budget gate ----------------------------------------

.PHONY: test-runtime-ledger runtime-ledger-pin

## test-runtime-ledger - hermetic execution-budget gate. Reads the pinned
##   closed census (D2B_RUNTIME_CENSUS) of crates, warm-builds those census
##   crates with the same compiler flags and selectors used by measurement, then
##   collects repeated samples at two granularities: per-test libtest wall-clock
##   values are advisory diagnostics below the hard 60-second per-test ceiling,
##   while each crate is enforced against the
##   process CPU consumed by its complete cargo test invocation (`time -p`
##   user+sys). CPU time excludes time descheduled behind unrelated machine
##   load, so concurrent work cannot manufacture a test regression. The
##   deterministic ledger labels both timing bases and their enforcement mode,
##   then enforces the crate CPU budget, the complete-census audit (non-empty
##   scopes, matching repetition counts, one sample per repetition), and the
##   closed-census audit (the crate and exact test sets reproduce the pin). It
##   also runs the hermetic placement lint over integration tests.
##
##   This is an absolute aggregate CPU budget gate. It holds no baseline and
##   makes no historical regression claim. A per-test sample above 60 seconds
##   fails; lower per-test threshold breaches are emitted to stderr as
##   non-failing advisories so CI and operators can enumerate likely
##   contributors to a crate CPU increase. Libtest exposes per-test wall time,
##   not per-test CPU time.
##   Measuring the latter would require a custom harness or one timed process per
##   test per repetition, whose startup cost and census-sized process fan-out
##   would materially change this gate.
##
##   Deferred follow-up (tracked as runtime-ledger-full-census-and-real-shards):
##   grow the census beyond the single pinned crate to a real multi-crate shard
##   inventory with a per-shard budget, and add a genuine cross-machine
##   reference baseline so a true historical-regression gate can be built on top
##   of these budgets. Until that lands, there is no shard dimension, no
##   baseline is recorded, and none is required.
##
##   All cargo invocations run from packages/ (not the repo root via
##   --manifest-path) so packages/.cargo/config.toml - and its sccache
##   rustc-wrapper - is discovered; the ledger and census paths are passed
##   root-absolute so the working-directory change cannot misplace them.
D2B_RUNTIME_RUNNER      ?= local
D2B_RUNTIME_REPETITIONS ?= 3
D2B_RUNTIME_LEDGER      ?= packages/target/test-runtime-ledger.json
D2B_RUNTIME_CENSUS      ?= tests/runtime-ledger-census.json
D2B_RUNTIME_CRATES      ?=
D2B_RUNTIME_UPDATE_CENSUS ?= 0
D2B_LEDGER_XTASK         = cargo run --quiet -p xtask -- test-runtime-ledger

## Classified hermetic tests that legitimately exceed the normal 50 ms
## per-test wall-clock diagnostic threshold. Per-test wall-clock data is
## advisory because machine contention makes it unsuitable for enforcement;
## these overrides keep the complete advisory-breach report useful without
## changing the enforced aggregate process-CPU crate budget:
##   * the *_bounded_byte_inputs_do_not_panic property harnesses each replay a
##     committed corpus and drive RUNS=10000 generated inputs through a parser,
##     so hundreds of milliseconds of pure execution is the intended workload;
##   * the rejects_tampered_*_artifact cases build and hash a full launcher /
##     unsafe-local artifact tree and prove every tamper is refused.
D2B_RUNTIME_ADVISORY_THRESHOLDS = \
	--advisory-threshold d2b-core::manifest_v04_bounded_byte_inputs_do_not_panic=1000 \
	--advisory-threshold d2b-core::bundle_bounded_byte_inputs_do_not_panic=1000 \
	--advisory-threshold d2b-core::host_json_bounded_byte_inputs_do_not_panic=1000 \
	--advisory-threshold d2b-core::privileges_json_bounded_byte_inputs_do_not_panic=1000 \
	--advisory-threshold d2b-core::rejects_tampered_realm_workloads_launcher_v2_artifact=300 \
	--advisory-threshold d2b-core::rejects_tampered_unsafe_local_workloads_artifact=300


## runtime-ledger-pin - measure the pinned crates and regenerate the exact
##   runtime test census after adding or removing a test. To add or remove a
##   whole crate, pass the complete intended set as D2B_RUNTIME_CRATES.
runtime-ledger-pin:
	@$(MAKE) --no-print-directory test-runtime-ledger D2B_RUNTIME_UPDATE_CENSUS=1

## test-runtime-ledger - emit and check the hermetic execution-budget ledger.
test-runtime-ledger:
	@set -eu; \
	started_at="$$(date +%s)"; \
	ledger='$(abspath $(D2B_RUNTIME_LEDGER))'; \
	census='$(abspath $(D2B_RUNTIME_CENSUS))'; \
	work='$(abspath packages/target/test-runtime-ledger.work)'; \
	reps='$(D2B_RUNTIME_REPETITIONS)'; \
	rm -rf "$$work"; mkdir -p "$$work"; \
	if [ -n '$(strip $(D2B_RUNTIME_CRATES))' ]; then \
	  crates='$(strip $(D2B_RUNTIME_CRATES))'; \
	else \
	  crates="$$(cd packages && $(D2B_LEDGER_XTASK) census --expected-census "$$census" --field crates)"; \
	fi; \
	if [ -z "$$crates" ]; then \
	  echo "test-runtime-ledger: the pinned census names no crates" >&2; exit 1; fi; \
	echo "test-runtime-ledger: linting hermetic placement across the census crates' integration tests"; \
	lint_files=""; \
	for crate in $$crates; do \
	  for f in packages/$$crate/tests/*.rs; do \
	    [ -e "$$f" ] && lint_files="$$lint_files $$f"; \
	  done; \
	done; \
	if [ -n "$$lint_files" ]; then \
	  ( cd packages && $(D2B_LEDGER_XTASK) lint $$(for f in $$lint_files; do echo "$(abspath .)/$$f"; done) ); \
	fi; \
	echo "test-runtime-ledger: selecting libtest-harness targets per census crate (harness=false custom-main binaries emit no libtest JSON and cannot be timed)"; \
	meta="$$work/cargo-metadata.json"; \
	( cd packages && cargo metadata --format-version 1 --no-deps ) > "$$meta" 2>/dev/null; \
	redactor="$$(jq -r '.target_directory + "/debug/xtask"' "$$meta")"; \
	for crate in $$crates; do \
	  manifest="$$(jq -r --arg pkg "$$crate" '.packages[] | select(.name==$$pkg) | .manifest_path' "$$meta")"; \
	  if [ -z "$$manifest" ] || [ ! -f "$$manifest" ]; then \
	    echo "test-runtime-ledger: cannot resolve the Cargo.toml for census crate $$crate; refusing to guess its harness set" >&2; exit 1; fi; \
	  harnessless="$$(awk '/^\[\[test\]\]/{if(t&&h&&n!="")print n;t=1;n="";h=0;next} /^\[/{if(t&&h&&n!="")print n;t=0} t&&/^[[:space:]]*name[[:space:]]*=/{l=$$0;sub(/.*=[[:space:]]*"/,"",l);sub(/".*/,"",l);n=l} t&&/^[[:space:]]*harness[[:space:]]*=[[:space:]]*false/{h=1} END{if(t&&h&&n!="")print n}' "$$manifest")"; \
	  tflags="$$(jq -r --arg pkg "$$crate" '.packages[] | select(.name==$$pkg) | .targets[] | select(.kind|index("test")) | select((.["required-features"]//[])|length==0) | .name' "$$meta" | \
	    while read -r tname; do \
	      [ -n "$$tname" ] || continue; \
	      drop=0; for x in $$harnessless; do [ "$$x" = "$$tname" ] && drop=1; done; \
	      [ "$$drop" -eq 0 ] && printf ' --test %s' "$$tname"; \
	    done)"; \
	  sel="--lib$$tflags"; \
	  printf '%s\n' "$$sel" > "$$work/$$crate.testargs"; \
	  echo "test-runtime-ledger: $$crate timed selector: $$sel"; \
	done; \
	echo "test-runtime-ledger: warm-building the exact timed selectors so compilation is excluded from CPU measurements"; \
	for crate in $$crates; do \
	  sel="$$(cat "$$work/$$crate.testargs")"; \
	  ( cd packages && RUSTC_BOOTSTRAP=1 cargo test -p "$$crate" $$sel --no-run --quiet ); \
	done; \
	args=""; \
	rep=0; \
	while [ "$$rep" -lt "$$reps" ]; do \
	  rep=$$((rep + 1)); \
	  echo "test-runtime-ledger: repetition $$rep/$$reps"; \
	  for crate in $$crates; do \
	    json="$$work/$$crate-$$rep.json"; \
	    err="$$work/$$crate-$$rep.err"; \
	    timing="$$work/$$crate-$$rep.time"; \
	    sel="$$(cat "$$work/$$crate.testargs")"; \
	    status=0; \
	    ( cd packages && \
	      D2B_LEDGER_JSON="$$json" D2B_LEDGER_ERR="$$err" D2B_LEDGER_TIMING="$$timing" \
	      /bin/bash -c '{ time -p "$$@" > "$$D2B_LEDGER_JSON" 2> "$$D2B_LEDGER_ERR"; } 2> "$$D2B_LEDGER_TIMING"' \
	        d2b-runtime-ledger env RUSTC_BOOTSTRAP=1 cargo test -p "$$crate" $$sel --quiet -- \
	        -Z unstable-options --format=json --report-time ) || status=$$?; \
	    if [ "$$status" -ne 0 ]; then \
	      echo "test-runtime-ledger: cargo test exited $$status for census crate $$crate; failing closed before recording any measurement" >&2; \
	      redact_status=0; \
	      if [ ! -x "$$redactor" ]; then \
	        echo "test-runtime-ledger: diagnostic redactor unavailable; raw cargo output suppressed" >&2; \
	      elif [ -n "$${HOME:-}" ]; then \
	        "$$redactor" redact-diagnostics --repo-root "$(abspath .)" --home "$$HOME" --tail-lines 20 < "$$err" >&2 || redact_status=$$?; \
	      else \
	        "$$redactor" redact-diagnostics --repo-root "$(abspath .)" --tail-lines 20 < "$$err" >&2 || redact_status=$$?; \
	      fi; \
	      if [ "$$redact_status" -ne 0 ]; then \
	        echo "test-runtime-ledger: diagnostic redaction failed; raw cargo output suppressed" >&2; \
	      fi; \
	      exit 1; \
	    fi; \
	    if grep -q '"event": "failed"' "$$json"; then \
	      echo "test-runtime-ledger: a hermetic test reported failure while timing census crate $$crate" >&2; exit 1; fi; \
	    timed_events="$$(awk '/"type": "test"/ && /"exec_time"/ { count++ } END { print count + 0 }' "$$json")"; \
	    if [ "$$timed_events" -lt 1 ]; then \
	      echo "test-runtime-ledger: census crate $$crate emitted no timed test events; refusing to record an empty measurement set" >&2; exit 1; fi; \
	    started=$$(grep '"type": "suite"' "$$json" | grep -c '"event": "started"' || true); \
	    passed=$$(grep '"type": "suite"' "$$json" | grep -c '"event": "ok"' || true); \
	    if [ "$$started" -lt 1 ] || [ "$$started" -ne "$$passed" ]; then \
	      echo "test-runtime-ledger: census crate $$crate did not emit a matching successful suite-completion event ($$passed/$$started ok); refusing to record a partial stream" >&2; exit 1; fi; \
	    if ! cdur="$$(awk '\
	        $$1 == "user" { user = $$2; saw_user = 1 } \
	        $$1 == "sys" { sys = $$2; saw_sys = 1 } \
	        END { if (!saw_user || !saw_sys) exit 1; printf "%d", ((user + sys) * 1000) + 0.5 }' \
	        "$$timing")"; then \
	      echo "test-runtime-ledger: could not parse process CPU timing for census crate $$crate; refusing to record" >&2; exit 1; \
	    fi; \
	    args="$$args --crate $$crate=$$cdur --crate-libtest-json $$crate=$$json"; \
	  done; \
	done; \
	( cd packages && $(D2B_LEDGER_XTASK) record \
	    --runner '$(D2B_RUNTIME_RUNNER)' --repetitions "$$reps" \
	    --output "$$ledger" $(D2B_RUNTIME_ADVISORY_THRESHOLDS) $$args ); \
	if [ '$(D2B_RUNTIME_UPDATE_CENSUS)' = 1 ]; then \
	  ( cd packages && $(D2B_LEDGER_XTASK) pin --ledger "$$ledger" --output "$$census" ); \
	fi; \
	( cd packages && $(D2B_LEDGER_XTASK) check \
	    --ledger "$$ledger" --expected-census "$$census" ); \
	finished_at="$$(date +%s)"; \
	echo "test-runtime-ledger: complete (duration: $$((finished_at - started_at))s)"

# ===========================================================================
# Disk hygiene.
#
#   make clean   Remove this worktree's cargo target directories and scratch
#                tree, then collect unreferenced Nix store paths. The shared
#                sccache directory is deliberately kept, so the next build
#                re-links rather than recompiling from scratch.
#
# Knobs: D2B_CLEAN_DRY_RUN=1, D2B_CLEAN_SKIP_GC=1, D2B_CLEAN_KEEP_SCRATCH=1.
clean:
	bash tests/tools/clean-worktree.sh
