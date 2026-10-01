# Makefile - d2b repository top-level convenience targets.
#
# Public compatibility targets. Bazel owns Layer-1 target selection,
# dependency ordering, parallelism, caching, and aggregation.

.DEFAULT_GOAL := check

# The dispatcher is deliberately explicit. A target is listed in exactly one
# environment class so a new public lane cannot silently inherit host-local or
# Bazel behavior from a name pattern.
D2B_MAKE_BAZEL_TARGETS := \
	check check-fast check-tier0 bazel-check test-unit \
	test-lint test-rust test-rust-main test-rust-broker \
	test-rust-schema test-rust-supply-chain test-rust-leaf-main-workspace \
	test-rust-leaf-schema test-rust-leaf-fixture-contracts test-rust-leaf-broker \
	test-rust-leaf-supply-chain test-fixture-contracts test-proofs test-flake \
	test-flake-realized test-flake-aarch64 test-flake-x86 test-nix-unit \
	test-performance-budgets test-drift test-policy test-changelog
D2B_MAKE_LOCAL_TARGETS := \
	check-clippy check-ci check-integration \
	test-integration test-host-integration perf \
	pre-tag smoke-lite heavy-check heavy-flake-check check-async-gate check-census \
	check-dead-code
# Meta helpers that invoke Bazel directly but are not Layer-1 test aliases.
D2B_MAKE_UTILITY_TARGETS := changelog-fold generate

D2B_MAKE_GOALS := $(if $(strip $(MAKECMDGOALS)),$(MAKECMDGOALS),$(.DEFAULT_GOAL))
D2B_MAKE_CLASSIFIED_GOALS := $(filter \
	$(D2B_MAKE_BAZEL_TARGETS) $(D2B_MAKE_LOCAL_TARGETS) \
	$(D2B_MAKE_UTILITY_TARGETS),$(D2B_MAKE_GOALS))
D2B_MAKE_RECURSIVE := $(MAKE)
D2B_MAKE_REENTRY ?= 0
NIX_FLAKE := nix --extra-experimental-features 'nix-command flakes'
D2B_MAKE_SHELL_READY := $(shell \
	if [ "$${D2B_PROJECT_SHELL:-}" = d2b ] && \
	   [ -n "$${D2B_BAZEL_BIN:-}" ] && [ -x "$${D2B_BAZEL_BIN}" ]; then \
		printf 1; \
	else \
		printf 0; \
	fi)

ifneq ($(strip $(D2B_MAKE_CLASSIFIED_GOALS)),)
ifneq ($(D2B_MAKE_SHELL_READY),1)
ifeq ($(D2B_MAKE_REENTRY),0)
D2B_MAKE_DISPATCH_REQUIRED := 1
else
$(error d2b Make dispatcher: re-entry marker is set but the d2b shell contract is incomplete (D2B_PROJECT_SHELL=d2b and executable D2B_BAZEL_BIN are required))
endif
endif
endif

ifeq ($(D2B_MAKE_DISPATCH_REQUIRED),1)
.PHONY: __d2b_make_dispatch $(D2B_MAKE_GOALS)

$(D2B_MAKE_GOALS): __d2b_make_dispatch

__d2b_make_dispatch:
	@set -eu; \
	if ! command -v nix >/dev/null 2>&1; then \
		echo "d2b Make dispatcher: Nix is required for $(D2B_MAKE_GOALS); enter the d2b shell or install Nix" >&2; \
		exit 127; \
	fi; \
	exec $(NIX_FLAKE) \
		develop --no-write-lock-file .#bazel -c \
		env D2B_MAKE_REENTRY=1 $(D2B_MAKE_RECURSIVE) --no-print-directory \
		D2B_MAKE_REENTRY=1 $(D2B_MAKE_GOALS)
else

# Recipe shells must not inherit exported Bash functions from their caller.
# Function resolution precedes PATH lookup, so an inherited cargo/nix/jq
# function could silently redirect a gate that intends to execute a binary.
SHELL := $(CURDIR)/tests/tools/scrub-shell-environment

.PHONY: pre-tag smoke-lite \
        check check-clippy check-ci check-integration check-fast check-tier0 \
        bazel-check \
        test-unit \
        test-lint test-rust test-rust-main \
        test-rust-broker \
        test-rust-schema test-rust-supply-chain \
        test-rust-leaf-main-workspace \
        test-rust-leaf-schema \
        test-rust-leaf-fixture-contracts test-rust-leaf-broker \
        test-rust-leaf-supply-chain \
        test-fixture-contracts test-proofs test-flake test-flake-realized \
        test-flake-aarch64 test-flake-x86 test-nix-unit \
        test-performance-budgets \
        test-drift test-policy test-changelog \
        check-integration \
        test-integration test-host-integration perf \
        heavy-check heavy-flake-check check-async-gate check-census \
        check-dead-code \
        generate \
        clean

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)

# ===========================================================================
# Test interface. Every Bazel-backed target below dispatches to the matching
# public suite in bazel/checks/BUILD.bazel.
#
#   make check          complete Bazel Layer-1 gate. Hermetic by design: it
#                       runs no integration lane, so it stays runnable on a
#                       hosted CI runner and on a host with neither a
#                       container runtime nor /dev/kvm.
#   make check-integration  check + both integration lanes; the local
#                       NixOS/KVM pre-PR aggregate that `check` excludes.
#   make check-ci       check + test-integration for local/manual compatibility.
#   make test-<layer>   focused Bazel suite.
#   make test-integration  type-9 container integration; local host/manual pre-PR.
#   make test-host-integration  type-10 Bazel host lane; local NixOS/KVM pre-PR.
#   make heavy-check     full Layer-1 check.
#   make heavy-flake-check  full flake realization.
# ===========================================================================

## Public Bazel aliases invoke `bazel test` directly. The .bazelrc default is
## BuildBuddy `remote`; PR/CI sets D2B_BAZEL_PROFILE=local (no wrapper).
D2B_BAZEL_PROFILE_ARG = $(if $(strip $(D2B_BAZEL_PROFILE)),--config=$(D2B_BAZEL_PROFILE))
D2B_BAZEL_LOCAL_TEST_JOBS ?=
D2B_BAZEL_JOBS ?=
D2B_BAZEL_TEST_OUTPUT ?=
BAZEL_BIN ?= $(if $(D2B_BAZEL_BIN),$(D2B_BAZEL_BIN),bazel)
D2B_BAZEL_TEST = $(BAZEL_BIN) test $(D2B_BAZEL_PROFILE_ARG) $(if $(strip $(D2B_BAZEL_JOBS)),--jobs=$(D2B_BAZEL_JOBS)) $(if $(strip $(D2B_BAZEL_LOCAL_TEST_JOBS)),--local_test_jobs=$(D2B_BAZEL_LOCAL_TEST_JOBS)) $(if $(strip $(D2B_BAZEL_TEST_OUTPUT)),--test_output=$(D2B_BAZEL_TEST_OUTPUT)) --test_env=D2B_REPO_ROOT="$(CURDIR)"
export D2B_BAZEL_PROFILE D2B_BAZEL_LOCAL_TEST_JOBS D2B_BAZEL_JOBS D2B_BAZEL_TEST_OUTPUT

## check - Layer-1 Bazel gate. Every crate's clippy runs inside the Bazel suite
## via the d2b_rust_rules clippy tests; the workspace per-crate `-Dwarnings`
## rustc flag carries into the clippy action, so any clippy warning in any
## crate fails here - the same strictness the rustc builds already enforce.
## The recipe comes from the shared $(D2B_MAKE_BAZEL_TARGETS) rule below.
check:

## check-clippy - standalone cargo clippy over the workspace with the workspace
## lint table's levels as the failure condition; kept as a fast local lane and
## used by check-ci. `.cargo/config.toml` sets `-D warnings` (this worktree's
## copy and the enclosing checkout's both apply, and cargo merges their
## rustflags), which would turn the pre-existing `clippy::all` corpus (833
## diagnostics at 040896e9f) into gate failures. `RUSTFLAGS=` replaces the
## config's rustflags instead of merging with them, so the manifest decides:
## only lints denied by `[workspace.lints]` fail the build, everything else
## stays a warning. `disallowed_methods` is allowed there while its 4,948-site
## backlog is converted - see the removal condition beside the allowance in
## Cargo.toml; `await_holding_lock` and `await_holding_refcell_ref` are denied
## and enforced by this target.
check-clippy:
	RUSTFLAGS= cargo clippy --workspace --all-targets --locked --keep-going

## check-dead-code - workspace dead-code/visibility/unused-dependency gate
## (cargo-hawk + cargo-shear + rustc dead_code) via the xtask. Runs in the
## d2b dev shell; cargo-shear is provisioned there, cargo-hawk needs a
## nightly rustc_private toolchain on PATH.
check-dead-code:
	cd $(CURDIR) && cargo run -p xtask -- deadcode-check

## check-ci - run the Layer-1 gate, then the conditional container lane.
check-ci: check-clippy
	$(D2B_BAZEL_TEST) //bazel/checks:check
	$(MAKE) test-integration

## check-integration - run the integration lanes, which `check` deliberately
## leaves out. `check` stays the hermetic Layer-1 gate: the two integration
## lanes need a container runtime (test-integration) and a KVM-capable NixOS
## host (test-host-integration), so folding them in would make the fast gate
## unrunnable on CI's hosted runners and on a laptop without /dev/kvm. This
## is the pre-PR aggregate: the Layer-1 gate, then both lanes.
check-integration: check
	$(MAKE) test-integration
	$(MAKE) test-host-integration

## check-fast - compatibility alias for check; check-tier0 is the fast subset.

$(D2B_MAKE_BAZEL_TARGETS):
	$(D2B_BAZEL_TEST) //bazel/checks:$@

# ===========================================================================
# Sub-targets. Each target is a thin alias over one public Bazel suite.
# ===========================================================================

## test-integration - L2 podman container integration tests.
test-integration:
	bash tests/test-integration.sh

## generate - regenerate committed schemas, docs, bindings, completions, and
## policy inputs through the one Bazel-owned generator aggregate.
generate:
	$(BAZEL_BIN) run --config=local //packages/xtask:generate

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## test-host-integration - G-host: the Bazel-owned host integration lane.
##
## Runs the eleven VM integration checks through the lane test target. Each
## check boots its own NixOS guest with the d2b daemon surface and asserts
## live broker / daemon / host-posture behaviour (socket activation, bridge
## isolation, state-dir ACLs, broker privilege posture) - the hermetic,
## non-destructive successor to the `D2B_LIVE`-against-the-real-host
## scripts. Every assertion is Rust. The guest images are graph outputs
## keyed on declared inputs, and the lane restores a pooled guest per check
## rather than booting one guest per check.
##
## This is a local, contributor-run lane and not a CI gate: it is a
## pre-PR surface, which is what lets the virtualization precondition be
## asserted rather than negotiated.
##
## Needs KVM. The lane declares virtualization as a precondition and has no
## silent emulation fallback, so on a host without it this target stops with
## a message rather than turning very slow. x86_64-linux only (it needs a
## same-system VM builder).
##
## Set D2B_VM_CHECK=<name> to run one named check. `bazel test
## --test_filter=<name>` works too: the lane reads Bazel's own filter as
## well as this variable, and the first of the two that names anything wins.
##
## The host tools and the guest are built under the committed `guest` profile
## from .bazelrc, so an exported Bazel profile cannot change the guest
## closure. The guest-image action declares its own substituters and
## preflights them itself, so the Attic preflight and closure upload that
## used to live here are gone with the nix recipe that needed them.
test-host-integration:
	@set -eu; \
	system="$$(nix eval --raw --impure --expr builtins.currentSystem)"; \
	if [ "$$system" != "x86_64-linux" ]; then \
	echo "test-host-integration: the lane is x86_64-linux only (it needs a same-system VM builder); skipping on $$system"; \
	exit 0; \
	fi; \
	if [ ! -e /dev/kvm ]; then \
	echo "test-host-integration: /dev/kvm is absent, and the lane declares virtualization as a precondition with no emulation fallback" >&2; \
	exit 1; \
	fi; \
	$(D2B_BAZEL_TEST) --config=guest \
	--test_env=D2B_VM_CHECK="$${D2B_VM_CHECK:-}" \
	$(if $(strip $(D2B_VM_CHECK)),//bazel/checks/vm:host_integration_lane_run_$(D2B_VM_CHECK),//bazel/checks/vm:host_integration_lane_run)

## perf - run the advisory performance budget suite.
perf:
	$(D2B_BAZEL_TEST) //bazel/checks:test-performance-budgets

## check-async-gate - the async-gate source hygiene gate (Layer-1 policy):
## scans the broker, the daemon, and every provider crate for denied blocking
## calls inside async contexts.
check-async-gate:
	$(D2B_BAZEL_TEST) //bazel/checks/policy:check-async-gate

## check-census - blocking-API census gate: runs the per-crate census (lexical
## meter for free functions, clippy-derived counts for instance-method
## classes) and fails when any covered crate exceeds its committed baseline in
## packages/xtask/data/blocking-census-baseline.json on EITHER axis: the
## per-entry deny-list count (including the no-new-spawn_blocking guard) or
## the crate's clippy::disallowed_methods allow/expect count. The second axis
## is what stops a new allow attribute from lowering the count baseline
## instead of removing a call.
## Re-baseline with `cargo xtask blocking-census --json <path>` after a
## conversion lands; the check always runs before the write, so a
## `--json <committed> --check <committed>` re-baseline is judged against the
## committed bytes, and the baseline change ships in the same commit.
check-census:
	cd $(CURDIR) && cargo run -p xtask -- blocking-census --check packages/xtask/data/blocking-census-baseline.json

## heavy-check - the complete Layer-1 check.
heavy-check:
	$(D2B_BAZEL_TEST) //bazel/checks:check

## heavy-flake-check - the building `nix flake check`; `make test-flake` is the
## cheap --no-build sibling.
heavy-flake-check:
	$(NIX_FLAKE) flake check --print-build-logs

# --- pre-existing maintainer targets ---------------------------------------

## pre-tag - run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, d2b active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates $${TMPDIR:-/tmp}/d2b-smoke-run-log.txt.
pre-tag:
	bash tests/integration/live/live-vm-smoke.sh --full

## smoke-lite - run the single-VM lite smoke gate (≤5 min).
smoke-lite:
	bash tests/integration/live/live-vm-smoke.sh --lite

.PHONY: changelog-fold

## test-changelog - the changelog policy gate (also the CI test-changelog job).
##                  Requires code changes to ship release notes as either a
##                  CHANGELOG.md entry or a changelog.d/ fragment, and validates
##                  the structure of every fragment present.
## changelog-fold - fold every changelog.d/ fragment into the CHANGELOG.md
##                  '## [Unreleased]' block and delete the consumed fragments.
##                  Run at merge time; see changelog.d/README.md.
changelog-fold:
	'$(BAZEL_BIN)' run --config=local //packages/xtask:xtask -- changelog-fold

.PHONY: update-agent-skills

## update-agent-skills - refresh the vendored agent-skill trees and the
##                      .agents/skills and .claude/skills links agent
##                      sessions load, so a fresh clone is fully configured
##                      without any install step.
##                      Sources: the Compound Engineering plugin, the
##                      caveman suite, and the rewrite-rs Rust skills;
##                      ponytail stays on its vendored copy. Commit the
##                      result afterwards.
update-agent-skills:
	bash tests/tools/update-agent-skills.sh

# ===========================================================================
# Disk hygiene.
#
#   make clean   Remove this worktree's build output directories and scratch
#                tree, then collect unreferenced Nix store paths. The shared
#                sccache directory is deliberately kept, so the next build
#                re-links rather than recompiling from scratch.
#
# Knobs: D2B_CLEAN_DRY_RUN=1, D2B_CLEAN_SKIP_GC=1, D2B_CLEAN_KEEP_SCRATCH=1.
clean:
	bash tests/tools/clean-worktree.sh

endif
