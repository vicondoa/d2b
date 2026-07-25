# Makefile - d2b repository top-level convenience targets.
#
# Maintainer-facing targets only; CI converges on this stable make-target
# interface incrementally during the test rearchitecture.

.PHONY: pre-tag smoke-lite i3-check \
        check check-static check-ci check-all check-fast check-tier0 \
        test test-unit \
        test-lint test-rust test-proofs test-flake test-nix-unit \
        test-flake-list \
        test-drift test-policy test-integration test-host-integration test-hardware perf \
        heavy-lane-guard heavy-lane-integration heavy-lane-host-integration \
        heavy-lane-hardware heavy-lane-perf \
        heavy-lane-pre-tag heavy-lane-smoke-lite \
        heavy-gate-build heavy-check heavy-cargo-test heavy-flake-check \
        heavy-test-integration heavy-test-host-integration heavy-test-hardware \
        layer1-workflow layer1-workflow-check \
        ledger-regen check-inventory pr-checklist-gate nix-unit-pin flake-matrix-pin

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

## test-rust - the comprehensive Rust gate (fmt, clippy, cargo test, contract
## tests with D2B_FIXTURES, CLI-contract layer, no-bash-ast-walker, broker
## workspace ×3 feature passes, schema-gen reproducibility, cargo-deny/audit,
## stub-no-socket, assert-pinned-tests).
test-rust:
	bash tests/test-rust.sh

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

## test-nix-unit - build all sharded nix-unit corpus checks (focused convenience
## target; already covered by test-flake, so NOT in test-unit to avoid double work).
test-nix-unit:
	bash tests/test-nix-unit.sh

## test-drift - generated-artifact drift gates (xtask gen-*, vms-json parity).
test-drift:
	bash tests/test-drift.sh

## test-policy - meta gates that guard the test architecture + cross-cutting
## invariants (ci-coverage, adr-index, deliverable-gate, etc.).
test-policy:
	bash tests/test-policy.sh

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
## A missing slot means someone ran the raw target directly, which would
## bypass the sole-use semaphore.
heavy-lane-guard: heavy-gate-build
	@if ! $(HEAVY_GATE_BIN) heavy-gate verify-slot; then \
	  echo "heavy lane invoked outside the heavy-gate semaphore." >&2; \
	  echo "Run the public lane (e.g. 'make test-integration'), which acquires a slot," >&2; \
	  echo "or 'cargo xtask heavy-gate -- make <lane>'; do not run the internal target directly." >&2; \
	  exit 2; \
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

.PHONY: test-runtime-ledger

## test-runtime-ledger - hermetic execution-budget gate. Reads the pinned
##   closed census (D2B_RUNTIME_CENSUS) of crates, warm-builds those census
##   crates so compilation never lands in the timings, then collects repeated
##   *execution-only* samples at two granularities - per test (from a
##   crate-qualified libtest JSON stream) and per crate (summed from that
##   stream's per-test exec_time, so cargo's dependency-freshness overhead never
##   lands in the timing) - across D2B_RUNTIME_REPETITIONS runs. It records them
##   into a deterministic, portable ledger carrying an operator-supplied runner
##   label instead of a hostname, then enforces the absolute per-test /
##   per-crate budgets, the complete-census audit (non-empty scopes, matching
##   repetition counts, one sample per repetition), and the closed-census audit
##   (the crate set reproduces the pin exactly). It also runs the hermetic
##   placement lint over the census crates' integration tests.
##
##   This is an absolute budget gate: every recorded p95 is judged only against
##   its own frozen budget. It holds no baseline and makes no historical
##   regression claim - a slower run that still fits the budget passes.
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
D2B_LEDGER_XTASK         = cargo run --quiet -p xtask -- test-runtime-ledger

## test-runtime-ledger - emit and check the hermetic execution-budget ledger.
test-runtime-ledger:
	@set -eu; \
	ledger='$(abspath $(D2B_RUNTIME_LEDGER))'; \
	census='$(abspath $(D2B_RUNTIME_CENSUS))'; \
	work='$(abspath packages/target/test-runtime-ledger.work)'; \
	reps='$(D2B_RUNTIME_REPETITIONS)'; \
	rm -rf "$$work"; mkdir -p "$$work"; \
	crates="$$(cd packages && $(D2B_LEDGER_XTASK) census --expected-census "$$census" --field crates)"; \
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
	echo "test-runtime-ledger: warm-building the census crates so compilation is excluded from timings"; \
	for crate in $$crates; do ( cd packages && cargo test -p "$$crate" --lib --tests --no-run --quiet ); done; \
	args=""; \
	rep=0; \
	while [ "$$rep" -lt "$$reps" ]; do \
	  rep=$$((rep + 1)); \
	  echo "test-runtime-ledger: repetition $$rep/$$reps"; \
	  for crate in $$crates; do \
	    json="$$work/$$crate-$$rep.json"; \
	    ( cd packages && RUSTC_BOOTSTRAP=1 cargo test -p "$$crate" --lib --tests --quiet -- \
	        -Z unstable-options --format=json --report-time ) > "$$json" 2>/dev/null || true; \
	    if grep -q '"event": "failed"' "$$json"; then \
	      echo "test-runtime-ledger: hermetic test failure while timing $$crate" >&2; exit 1; fi; \
	    cdur="$$(grep '"type": "test"' "$$json" | grep '"exec_time"' | \
	        sed -E 's/.*"exec_time": ([0-9.]+).*/\1/' | \
	        awk '{ s += $$1 } END { printf "%d", (s * 1000) + 0.5 }')"; \
	    args="$$args --crate $$crate=$$cdur --crate-libtest-json $$crate=$$json"; \
	  done; \
	done; \
	( cd packages && $(D2B_LEDGER_XTASK) record \
	    --runner '$(D2B_RUNTIME_RUNNER)' --repetitions "$$reps" \
	    --output "$$ledger" $$args ); \
	( cd packages && $(D2B_LEDGER_XTASK) check \
	    --ledger "$$ledger" --expected-census "$$census" )

