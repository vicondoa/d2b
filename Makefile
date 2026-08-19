# Makefile - d2b repository top-level convenience targets.
#
# Public compatibility targets. Bazel owns Layer-1 target selection,
# dependency ordering, parallelism, caching, and aggregation.

# Recipe shells must not inherit exported Bash functions from their caller.
# Function resolution precedes PATH lookup, so an inherited cargo/nix/jq
# function could silently redirect a gate that intends to execute a binary.
SHELL := $(CURDIR)/tests/tools/scrub-shell-environment

.PHONY: pre-tag smoke-lite \
        check check-static check-ci check-all check-fast check-tier0 \
        bazel-check \
        test test-unit \
        test-lint test-rust test-rust-main \
        test-rust-broker test-rust-guest-shell-runner test-rust-local test-rust-no-bash-ast \
        test-rust-schema test-rust-inventory test-rust-supply-chain \
        test-cargo-compat \
        test-rust-leaf-main-workspace \
        test-rust-leaf-schema test-rust-leaf-inventory \
        test-rust-leaf-fixture-contracts test-rust-leaf-broker \
        test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast \
        test-rust-leaf-supply-chain \
        test-fixture-contracts test-proofs test-flake test-flake-realized \
        test-flake-aarch64 test-flake-x86 test-nix-unit \
        test-performance-budgets test-ci-coverage \
        test-drift test-policy test-integration test-host-integration test-hardware perf \
        heavy-lane-guard heavy-lane-integration heavy-lane-host-integration \
        heavy-lane-hardware heavy-lane-perf \
        heavy-lane-pre-tag heavy-lane-smoke-lite \
        heavy-gate-build heavy-gate-provision heavy-check heavy-cargo-test heavy-flake-check \
        heavy-test-integration heavy-test-host-integration heavy-test-hardware \
        ledger-regen check-inventory nix-unit-pin \
        runtime-ledger-pin clean

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)
NIX_FLAKE := nix --extra-experimental-features 'nix-command flakes'

# ===========================================================================
# Test interface. Every Layer-1 target below is one direct Bazel invocation;
# the fixed target lists are the only Make-side compatibility mapping.
#
#   make check          complete fixed Bazel Layer-1 gate.
#   make check-static   Legacy monolithic tests/static.sh full-static gate.
#   make check-ci       check + test-integration for local/manual compatibility.
#   make check-all      check-ci + test-hardware + perf - full local NixOS gate.
#   make test-<layer>   focused fixed Bazel suite.
#   make test-integration  type-9 container integration; local host/manual pre-PR.
#   make test-host-integration  type-10 runNixOSTest; local NixOS/KVM pre-PR.
#   make test-hardware     G-hw real GPU/YubiKey/TPM passthrough - NixOS host only.
#   make heavy-<lane>      the same lane, serialized through the two-slot
#                          per-uid heavy-gate semaphore (see "Heavy lanes").
# ===========================================================================

## check - the complete fixed Bazel Layer-1 gate.
check:
	$(BAZEL_RUN) $(D2B_BAZEL_COMPLETE_TARGETS)

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
	D2B_BAZEL_TEST_TAG_FILTERS="-gpu,-kvm" tests/tools/bazel-check --profile "$(D2B_BAZEL_PROFILE)" -- //bazel/checks/meta:tier0

## bazel-check - complete fixed Bazel graph. Locally this defaults to the
## BuildBuddy remote profile; CI sets D2B_BAZEL_PROFILE=local.
D2B_BAZEL_PROFILE ?= remote
D2B_BAZEL_TEST_TAG_FILTERS ?= -manual,-gpu,-kvm

D2B_BAZEL_MAIN_TARGETS = \
	//packages/... \
	//bazel/checks/rust/... \
	-//packages/d2b-priv-broker/... \
	-//packages/d2b-guest-shell-runner/... \
	-//packages/d2b-bus/tests/ui/... \
	-//packages/d2b-controller-toolkit/tests/ui/... \
	-//packages/d2b-resource-api/tests/ui/... \
	-//packages/xtask:policy_ci \
	-//packages/xtask:policy_workspace \
	-//bazel/checks/rust:portable_rust_broker \
	-//bazel/checks/rust:portable_rust_guest \
	-//bazel/checks/rust:d2b_priv_broker_doc_test \
	-//bazel/checks/rust:d2b_guest_shell_runner_doc_test
D2B_BAZEL_BROKER_TARGETS = //bazel/checks/rust:portable_rust_broker
D2B_BAZEL_GUEST_TARGETS = //bazel/checks/rust:portable_rust_guest
D2B_BAZEL_LOCAL_RUST_TARGETS = //bazel/checks/rust:portable_rust_local
D2B_BAZEL_POLICY_TARGETS = //bazel/checks/policy:policy_tooling
D2B_BAZEL_NIX_EVAL_TARGETS = //bazel/checks/nix:nix_evaluation
D2B_BAZEL_NIX_REALIZED_TARGETS = //bazel/checks/nix:nix_realized
D2B_BAZEL_NIX_AARCH64_TARGETS = //bazel/checks/nix:nix_aarch64
D2B_BAZEL_FIXTURE_TARGETS = //bazel/checks/fixtures:fixtures_proofs
D2B_BAZEL_COMPLETE_TARGETS = \
	$(D2B_BAZEL_MAIN_TARGETS) \
	$(D2B_BAZEL_BROKER_TARGETS) \
	$(D2B_BAZEL_GUEST_TARGETS) \
	$(D2B_BAZEL_LOCAL_RUST_TARGETS) \
	$(D2B_BAZEL_POLICY_TARGETS) \
	$(D2B_BAZEL_NIX_EVAL_TARGETS) \
	$(D2B_BAZEL_NIX_REALIZED_TARGETS) \
	$(D2B_BAZEL_NIX_AARCH64_TARGETS) \
	$(D2B_BAZEL_FIXTURE_TARGETS) \
	//bazel/checks/meta:performance_budgets

BAZEL_RUN = \
	D2B_BAZEL_TEST_TAG_FILTERS="$(D2B_BAZEL_TEST_TAG_FILTERS)" \
	tests/tools/bazel-check --profile "$(D2B_BAZEL_PROFILE)" --
export D2B_BAZEL_PROFILE D2B_BAZEL_TEST_TAG_FILTERS

# ===========================================================================
# Umbrella test targets. Layer-2 lanes remain explicit manual/local targets.
# ===========================================================================

test:
	$(MAKE) test-unit
	$(MAKE) test-integration

test-unit:
	$(BAZEL_RUN) $(D2B_BAZEL_COMPLETE_TARGETS)

bazel-check:
	$(BAZEL_RUN) $(D2B_BAZEL_COMPLETE_TARGETS)

# ===========================================================================
# Sub-targets. Each target is a thin alias over one fixed Bazel label set.
# ===========================================================================

## test-lint - fixed Bazel lint suite.
test-lint:
	$(BAZEL_RUN) //bazel/checks/policy:lint

## test-rust - fixed portable Rust, broker, and guest Bazel suites.
test-rust:
	$(BAZEL_RUN) $(D2B_BAZEL_MAIN_TARGETS) $(D2B_BAZEL_BROKER_TARGETS) $(D2B_BAZEL_GUEST_TARGETS) $(D2B_BAZEL_LOCAL_RUST_TARGETS)

test-rust-main:
	D2B_BAZEL_TEST_TAG_FILTERS="-local,-manual,-exclusive,-gpu,-kvm" tests/tools/bazel-check --profile "$(D2B_BAZEL_PROFILE)" -- $(D2B_BAZEL_MAIN_TARGETS)

test-rust-broker:
	$(BAZEL_RUN) $(D2B_BAZEL_BROKER_TARGETS)

test-rust-guest-shell-runner:
	$(BAZEL_RUN) $(D2B_BAZEL_GUEST_TARGETS)

test-rust-local:
	$(BAZEL_RUN) $(D2B_BAZEL_LOCAL_RUST_TARGETS)

test-rust-no-bash-ast:
	$(BAZEL_RUN) //tests/tools/no-bash-ast-walker:no_bash_ast_test

test-rust-schema:
	$(BAZEL_RUN) //packages/xtask:schema_reproducibility_test

test-rust-inventory:
	$(MAKE) check-tier0

test-rust-supply-chain:
	$(BAZEL_RUN) //bazel/checks/nix:flake-eval-x86-realized

## test-cargo-compat - standalone Cargo proof for the generic, serial, guest,
## doctest, harness-free, bench, and fixture-exclusion contracts. This target
## is deliberately independent of the Bazel scheduler.
test-cargo-compat:
	bash tests/tools/cargo-compat.sh

test-rust-leaf-main-workspace: test-rust-main
test-rust-leaf-schema: test-rust-schema
test-rust-leaf-inventory: test-rust-inventory
test-rust-leaf-fixture-contracts: test-fixture-contracts
test-rust-leaf-broker: test-rust-broker
test-rust-leaf-guest-shell-runner: test-rust-guest-shell-runner
test-rust-leaf-no-bash-ast: test-rust-no-bash-ast
test-rust-leaf-supply-chain: test-rust-supply-chain

## test-fixture-contracts - fixed fixture and proof Bazel suite.
test-fixture-contracts:
	$(BAZEL_RUN) $(D2B_BAZEL_FIXTURE_TARGETS)

## test-proofs - fixed fixture and proof Bazel suite.
test-proofs:
	$(BAZEL_RUN) $(D2B_BAZEL_FIXTURE_TARGETS)

## test-flake - fixed Nix evaluation Bazel suite.
test-flake:
	$(BAZEL_RUN) $(D2B_BAZEL_NIX_EVAL_TARGETS)

test-flake-x86: test-flake

test-flake-realized:
	$(BAZEL_RUN) $(D2B_BAZEL_NIX_REALIZED_TARGETS)

test-flake-aarch64:
	$(BAZEL_RUN) $(D2B_BAZEL_NIX_AARCH64_TARGETS)

## test-nix-unit - fixed Nix-unit Bazel suite.
test-nix-unit:
	$(BAZEL_RUN) //bazel/checks/nix:nix_unit

## test-drift - fixed Bazel drift suite.
test-drift:
	$(BAZEL_RUN) //bazel/checks/policy:drift

## test-policy - fixed Bazel policy/tooling suite.
test-policy:
	$(BAZEL_RUN) $(D2B_BAZEL_POLICY_TARGETS)

## test-performance-budgets - execute the self-gating performance canary.
## Hosted runners take the cheap skip path; pinned stable runners enforce it.
test-performance-budgets:
	$(BAZEL_RUN) //bazel/checks/meta:performance_budgets

test-ci-coverage:
	$(BAZEL_RUN) //bazel/checks/policy:policy_tooling

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

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## check-inventory - compatibility alias for the fixed Bazel inventory test.
check-inventory:
	$(MAKE) check-tier0

## ledger-regen - regenerate tests/migration-ledger.toml in place for humans.
ledger-regen:
	bash tests/tools/gen-migration-ledger.sh

## nix-unit-pin - regenerate the fail-closed nix-unit case-presence pins
## (tests/unit/nix/pinned/*.txt) after adding or removing cases.
nix-unit-pin:
	bash tests/tools/gen-nix-unit-pins.sh

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
	set --; \
	for name in $$names; do \
	set -- "$$@" ".#vmChecks.$$system.$$name"; \
	done; \
	case "$${D2B_HOST_SCCACHE:-}" in \
	1|yes|true) \
	cache_dir="$${SCCACHE_DIR:-$${XDG_CACHE_HOME:-$$HOME/.cache}/d2b-sccache}"; \
	mkdir -p "$$cache_dir"; \
	chmod 0700 "$$cache_dir"; \
	cache_dir="$$(cd "$$cache_dir" && pwd -P)"; \
	echo "test-host-integration: sccache cache: $$cache_dir -> /var/cache/d2b-sccache"; \
	echo "==> nix build --option extra-sandbox-paths /var/cache/d2b-sccache=$$cache_dir $$*"; \
	nix build --option extra-sandbox-paths "/var/cache/d2b-sccache=$$cache_dir" --no-link --print-build-logs "$$@";; \
	*) \
	echo "test-host-integration: sccache disabled (set D2B_HOST_SCCACHE=1 to enable)"; \
	echo "==> nix build $$*"; \
	nix build --no-link --print-build-logs "$$@";; \
	esac

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
# executed at the same location. Cargo runs the build from the repository
# root, so a relative value is resolved against the root before the binary is
# executed. Resolve it here and pass the resolved
# absolute path back to cargo, so both the build and the execution agree
# regardless of the caller's value.
ifeq ($(CARGO_TARGET_DIR),)
HEAVY_GATE_TARGET_DIR := $(CURDIR)/target
else ifeq ($(filter /%,$(CARGO_TARGET_DIR)),)
HEAVY_GATE_TARGET_DIR := $(abspath $(CURDIR)/$(CARGO_TARGET_DIR))
else
HEAVY_GATE_TARGET_DIR := $(CARGO_TARGET_DIR)
endif
HEAVY_GATE_BIN := $(HEAVY_GATE_TARGET_DIR)/debug/xtask
HEAVY_GATE = $(HEAVY_GATE_BIN) heavy-gate --

## heavy-gate-build - build the semaphore wrapper from the governed workspace
## manifest. The build target dir is forced to the same absolute
## HEAVY_GATE_TARGET_DIR the wrapper is executed from, so a relative
## CARGO_TARGET_DIR cannot split the two.
heavy-gate-build:
	@CARGO_TARGET_DIR='$(HEAVY_GATE_TARGET_DIR)' cargo build --quiet --manifest-path Cargo.toml --locked -p xtask --bin xtask

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
	$(HEAVY_GATE) cargo test $(HEAVY_CARGO_TEST_ARGS)

## heavy-flake-check - the building `nix flake check` under the semaphore.
##                     `make test-flake` is the cheap --no-build sibling.
heavy-flake-check: heavy-gate-build
	$(HEAVY_GATE) $(NIX_FLAKE) flake check --print-build-logs

# --- pre-existing maintainer targets ---------------------------------------

## pre-tag - run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, d2b active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates $${TMPDIR:-/tmp}/d2b-smoke-run-log.txt.
##           Public heavy lane: acquires a slot, then runs the raw live work behind
##           the gate - the live smoke suite is the most destructive, stateful lane
##           in the tree and must never bypass the sole-use semaphore.
pre-tag: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-pre-tag

## heavy-lane-pre-tag - the raw full live-VM smoke work. Internal: reachable only
## from inside the gate (see heavy-lane-guard).
heavy-lane-pre-tag: heavy-lane-guard
	bash tests/integration/live/live-vm-smoke.sh --full

## smoke-lite - run the single-VM lite smoke gate (≤5 min).
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
	$(BAZEL_RUN) //bazel/checks/policy:changelog

## changelog-fold - fold every changelog.d/ fragment into the CHANGELOG.md
##                  '## [Unreleased]' block and delete the consumed fragments.
##                  Run at merge time; see changelog.d/README.md.
changelog-fold:
	cargo run -q -p xtask -- changelog-fold
## test-runtime-ledger - fixed Bazel runtime-budget policy target.
test-runtime-ledger:
	$(BAZEL_RUN) //bazel/checks/policy:runtime_ledger

## runtime-ledger-pin - compatibility alias for the fixed runtime-ledger target.
runtime-ledger-pin: test-runtime-ledger

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
