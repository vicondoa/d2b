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
	test-rust-guest-shell-runner test-rust-local \
	test-rust-schema test-rust-supply-chain test-rust-leaf-main-workspace \
	test-rust-leaf-schema test-rust-leaf-fixture-contracts test-rust-leaf-broker \
	test-rust-leaf-guest-shell-runner \
	test-rust-leaf-supply-chain test-fixture-contracts test-proofs test-flake \
	test-flake-realized test-flake-aarch64 test-flake-x86 test-nix-unit \
	test-performance-budgets test-drift test-policy test-changelog
D2B_MAKE_LOCAL_TARGETS := \
	check-ci test-integration test-host-integration perf pre-tag smoke-lite
# Meta helpers that invoke tooling directly but are not Layer-1 test aliases.
D2B_MAKE_UTILITY_TARGETS := changelog-fold

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
        check check-ci check-fast check-tier0 \
        bazel-check \
        test-unit \
        test-lint test-rust test-rust-main \
        test-rust-broker test-rust-guest-shell-runner test-rust-local \
        test-rust-schema test-rust-supply-chain \
        test-rust-leaf-main-workspace \
        test-rust-leaf-schema \
        test-rust-leaf-fixture-contracts test-rust-leaf-broker \
        test-rust-leaf-guest-shell-runner \
        test-rust-leaf-supply-chain \
        test-fixture-contracts test-proofs test-flake test-flake-realized \
        test-flake-aarch64 test-flake-x86 test-nix-unit \
        test-performance-budgets \
        test-drift test-policy test-changelog \
        test-integration test-host-integration perf \
        clean

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)

# ===========================================================================
# Test interface. Every Bazel-backed target below dispatches to the matching
# public suite in bazel/checks/BUILD.bazel.
#
#   make check          complete Bazel Layer-1 gate.
#   make check-ci       check + test-integration for local/manual compatibility.
#   make test-<layer>   focused Bazel suite.
#   make test-integration  type-9 container integration; local host/manual pre-PR.
#   make test-host-integration  type-10 runNixOSTest; local NixOS/KVM pre-PR.
# ===========================================================================

## check-ci - run the Layer-1 gate, then the conditional container lane.
check-ci:
	$(BAZEL_RUN) //bazel/checks:check
	$(MAKE) test-integration

## check-fast - compatibility alias for check; check-tier0 is the fast subset.

## bazel-check - complete Bazel graph. Locally this defaults to the
## BuildBuddy profile; the facade falls back to local execution when the
## credential is unavailable. CI sets D2B_BAZEL_PROFILE=local.
D2B_BAZEL_PROFILE ?= remote
D2B_BAZEL_TEST_TAG_FILTERS ?= -manual,-gpu,-kvm

BAZEL_RUN = \
	env \
	D2B_BAZEL_JOB="$@" \
	D2B_BAZEL_TEST_TAG_FILTERS="$(D2B_BAZEL_TEST_TAG_FILTERS)" \
	tests/tools/bazel-check --profile "$(D2B_BAZEL_PROFILE)" --
export D2B_BAZEL_PROFILE D2B_BAZEL_TEST_TAG_FILTERS

check-tier0: D2B_BAZEL_TEST_TAG_FILTERS := -gpu,-kvm
test-rust-main: D2B_BAZEL_TEST_TAG_FILTERS := -local,-no-remote-exec,-manual,-exclusive,-gpu,-kvm

$(D2B_MAKE_BAZEL_TARGETS):
	$(BAZEL_RUN) //bazel/checks:$@

# ===========================================================================
# Sub-targets. Each target is a thin alias over one public Bazel suite.
# ===========================================================================

## test-integration - L2 podman container integration tests.
test-integration:
	bash tests/test-integration.sh

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## test-host-integration - G-host: runNixOSTest VM integration tests (the
## `vmChecks` flake output, NOT swept by `nix flake check`). Each test boots a
## real NixOS VM with the d2b daemon surface and asserts live broker /
## daemon / host-posture behaviour (socket activation, bridge isolation,
## state-dir ACLs, broker privilege posture) - the hermetic, non-destructive
## successor to the `D2B_LIVE`-against-the-real-host scripts. Needs KVM (a local
## NixOS host; TCG software emulation is the slow fallback when /dev/kvm is
## absent). x86_64-linux only (a same-system VM builder is required).
## Set D2B_VM_CHECK=<name> to build one named vmChecks entry.
test-host-integration:
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
	if [ -n "$${D2B_VM_CHECK:-}" ]; then \
	names="$$D2B_VM_CHECK"; \
	else \
	names="$$(nix eval --raw --impure --no-warn-dirty --expr "builtins.concatStringsSep \" \" (builtins.attrNames (builtins.getFlake \"git+file://$$root\").vmChecks.$$system)")"; \
	fi; \
	requested="$${D2B_HOST_VM_CHECK:-}"; \
	if [ -n "$$requested" ]; then \
	case "$$requested" in \
	*[!A-Za-z0-9._-]*) \
	echo "test-host-integration: invalid D2B_HOST_VM_CHECK (use one discovered vmCheck name): $$requested" >&2; \
	exit 1;; \
	esac; \
	fi; \
	if [ -z "$$names" ]; then \
	if [ -n "$$requested" ]; then \
	echo "test-host-integration: unknown vmCheck '$$requested' (available: none)" >&2; \
	exit 1; \
	fi; \
	echo "test-host-integration: no vmChecks present"; \
	exit 0; \
	fi; \
	if [ -n "$$requested" ]; then \
	case " $$names " in \
	*" $$requested "*) names="$$requested";; \
	*) \
	echo "test-host-integration: unknown vmCheck '$$requested' (available: $$names)" >&2; \
	exit 1;; \
	esac; \
	fi; \
	fail_sccache_preflight() { \
	echo "test-host-integration: sccache preflight failed: $$1" >&2; \
	echo "Remediation: enable d2b.site.hostSccache.enable = true in the NixOS host configuration, then run:" >&2; \
	echo "  sudo nixos-rebuild switch --flake /path/to/host#<host>" >&2; \
	echo "The activated host must expose /var/cache/d2b-sccache in /etc/nix/nix.conf and keep it root:nixbld mode 2770." >&2; \
	exit 1; \
	}; \
	set --; \
	for name in $$names; do \
	set -- "$$@" ".#vmChecks.$$system.$$name"; \
	done; \
	case "$${D2B_HOST_SCCACHE:-}" in \
	1|yes|true) \
	cache_dir=/var/cache/d2b-sccache; \
	if [ ! -r /etc/nix/nix.conf ]; then \
	fail_sccache_preflight "/etc/nix/nix.conf is missing or unreadable"; \
	fi; \
	if ! awk '\
		/^[[:space:]]*#/ { next } \
		/^[[:space:]]*extra-sandbox-paths[[:space:]]*=/ { \
			line = $$0; \
			sub(/^[^=]*=/, "", line); \
			found = 0; \
			count = split(line, fields, /[[:space:]]+/); \
			for (i = 1; i <= count; i++) if (fields[i] == "/var/cache/d2b-sccache") found = 1; \
		} \
		END { exit(found ? 0 : 1) } \
	' /etc/nix/nix.conf; then \
	fail_sccache_preflight "/etc/nix/nix.conf does not expose extra-sandbox-paths = /var/cache/d2b-sccache"; \
	fi; \
	if [ ! -d "$$cache_dir" ]; then \
	fail_sccache_preflight "$$cache_dir does not exist"; \
	fi; \
	cache_owner="$$(stat -c '%U' "$$cache_dir")"; \
	cache_group="$$(stat -c '%G' "$$cache_dir")"; \
	cache_mode="$$(stat -c '%a' "$$cache_dir")"; \
	if [ "$$cache_owner" != root ] || [ "$$cache_group" != nixbld ] || [ "$$cache_mode" != 2770 ]; then \
	fail_sccache_preflight "$$cache_dir must be owned by root:nixbld with mode 2770 (found $$cache_owner:$$cache_group mode $$cache_mode)"; \
	fi; \
	if ! getent group nixbld >/dev/null 2>&1; then \
	fail_sccache_preflight "the nixbld daemon build-user group is unavailable"; \
	fi; \
	build_users_group="$$(nix show-config 2>/dev/null | awk '$$1 == \"build-users-group\" { print $$3; exit }')"; \
	if [ "$$build_users_group" != nixbld ]; then \
	fail_sccache_preflight "the Nix daemon build-users-group is not nixbld (found '$$build_users_group')"; \
	fi; \
	echo "test-host-integration: sccache preflight passed ($$cache_dir root:nixbld 2770, daemon build group nixbld)";; \
	*) \
	echo "test-host-integration: sccache disabled (set D2B_HOST_SCCACHE=1 to enable)"; \
	;; \
	esac; \
	echo "test-host-integration: building vmChecks: $$names"; \
	echo "==> nix build $$*"; \
	nix build --no-link --print-build-logs "$$@"

perf:
	$(BAZEL_RUN) //bazel/checks:test-performance-budgets

BAZEL_BIN ?= $(if $(D2B_BAZEL_BIN),$(D2B_BAZEL_BIN),bazel)

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
