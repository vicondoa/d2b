# Makefile — d2b repository top-level convenience targets.
#
# Maintainer-facing targets only; CI converges on this stable make-target
# interface incrementally during the test rearchitecture.

.PHONY: pre-tag smoke-lite i3-check \
        check check-static check-ci check-all check-fast check-tier0 \
        test test-unit \
        test-lint test-rust test-proofs test-flake test-nix-unit \
        test-flake-list \
        test-drift test-policy test-integration test-host-integration test-hardware perf \
        layer1-workflow layer1-workflow-check \
        ledger-regen check-inventory pr-checklist-gate nix-unit-pin flake-matrix-pin

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)
NIX_FLAKE := nix --extra-experimental-features 'nix-command flakes'
CARGO_XTASK := cd packages && RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER= cargo xtask

# ===========================================================================
# Test-rearchitecture interface. The targets are the stable contract; the
# local/CI Layer-1 gate graph lives in tests/layer1-jobs.json.
#
#   make check          L1 PR-equivalent gate, locally parallelized.
#   make check-static   Legacy monolithic tests/static.sh full-static gate.
#   make check-ci       check + test-integration for local/manual compatibility.
#   make check-all      check-ci + test-hardware + perf — full local NixOS gate.
#   make test-<layer>   focused per-layer run (ledger-driven).
#   make test-integration  type-9 container integration; local host/manual pre-PR.
#   make test-host-integration  type-10 runNixOSTest; local NixOS/KVM pre-PR.
#   make test-hardware     G-hw real GPU/YubiKey/TPM passthrough — NixOS host only.
# ===========================================================================

## check — the Layer-1 PR-equivalent done-gate. The manifest runner executes
##          check-tier0 first, then safe L1 sub-targets in parallel, then
##          drift after the parallel phase. Tune with D2B_CHECK_JOBS and
##          D2B_FLAKE_JOBS.
check:
	$(CARGO_XTASK) layer1 run-local

## check-static — legacy/full-static monolithic gate retained for explicit use.
check-static:
	bash tests/static.sh

## check-ci — W0: run check, then skip or run legacy G-ci on a suitable host.
check-ci:
	$(MAKE) check
	$(MAKE) test-integration

## check-all — the full local gate on a NixOS host with devices.
check-all:
	$(MAKE) check-ci
	$(MAKE) test-hardware
	$(MAKE) perf

## check-fast / check-tier0 — fast PR-loop subsets.
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
	$(CARGO_XTASK) layer1 run-local --skip-preflight

# ===========================================================================
# Sub-targets. Test gates have corresponding tests/test-<name>.sh drivers;
# manifest/check-discovery plumbing is owned by Rust xtask.
# ===========================================================================

## test-lint — preflight + nix-instantiate --parse + shellcheck (no eval, no cargo).
test-lint:
	bash tests/test-lint.sh

## test-rust — the comprehensive Rust gate (fmt, clippy, cargo test, contract
## tests with D2B_FIXTURES, CLI-contract layer, no-bash-ast-walker, broker
## workspace ×3 feature passes, schema-gen reproducibility, cargo-deny/audit,
## stub-no-socket, assert-pinned-tests).
test-rust:
	bash tests/test-rust.sh

## test-proofs — standalone proof crates under proofs/ (not members of packages/).
test-proofs:
	bash tests/test-proofs.sh

## test-flake — `nix flake check --no-build` for the native system (bounded
## memory). CI shards the x86_64 leg one-job-per-check via a dynamic matrix:
## set D2B_FLAKE_CHECK=<name> to instantiate just that one check (the matrix
## enumerates names with `make test-flake-list`); the aarch64 PR leg runs only a
## lightweight smoke eval. Set D2B_FLAKE_ALL_SYSTEMS=1 to cross-evaluate every
## system locally (like `make check`/static.sh).
test-flake:
	bash tests/test-flake.sh

## test-flake-list — emit the native-system flake check names as a JSON array on
## stdout (CI dynamic-matrix plumbing for the sharded test-flake; see
## .github/workflows/pr-l1-static-fast.yml). Invoke as `make -s test-flake-list`.
test-flake-list:
	@$(CARGO_XTASK) layer1 checks list

## test-nix-unit — build all sharded nix-unit corpus checks (focused convenience
## target; already covered by test-flake, so NOT in test-unit to avoid double work).
test-nix-unit:
	bash tests/test-nix-unit.sh

## test-drift — generated-artifact drift gates (xtask gen-*, vms-json parity).
test-drift:
	bash tests/test-drift.sh

## test-policy — meta gates that guard the test architecture + cross-cutting
## invariants (ci-coverage, adr-index, deliverable-gate, etc.).
test-policy:
	bash tests/test-policy.sh

## test-integration — L2 podman container integration tests.
test-integration:
	bash tests/test-integration.sh

## layer1-workflow — regenerate the Layer-1 PR workflow from tests/layer1-jobs.json.
layer1-workflow:
	$(CARGO_XTASK) layer1 workflow write

## layer1-workflow-check — fail if the generated Layer-1 PR workflow is stale.
layer1-workflow-check:
	$(CARGO_XTASK) layer1 workflow check

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## check-inventory — fail-closed ledger drift check for CI.
check-inventory:
	bash tests/tools/gen-migration-ledger.sh --check

## ledger-regen — regenerate tests/migration-ledger.toml in place for humans.
ledger-regen:
	bash tests/tools/gen-migration-ledger.sh

## nix-unit-pin — regenerate the fail-closed nix-unit case-presence pins
## (tests/unit/nix/pinned/*.txt) after adding or removing cases.
nix-unit-pin:
	bash tests/tools/gen-nix-unit-pins.sh

## flake-matrix-pin — regenerate the fail-closed CI flake-check-matrix pin
## (tests/golden/flake-check-matrix/<system>.txt) after adding/removing a flake
## check. The drift gate (run by `make test-drift`) fails closed until this is
## rerun, so the sharded x86 CI matrix can't silently change coverage.
flake-matrix-pin:
	bash tests/tools/gen-flake-check-matrix-pin.sh

## W0 policy gate (also run by test-policy).
pr-checklist-gate:
	bash tests/unit/meta/pr-checklist-gate.sh .github/PULL_REQUEST_TEMPLATE.md

## test-host-integration — G-host: runNixOSTest VM integration tests (the
## `vmChecks` flake output, NOT swept by `nix flake check`). Each test boots a
## real NixOS VM with the d2b daemon surface and asserts live broker /
## daemon / host-posture behaviour (socket activation, bridge isolation,
## state-dir ACLs, broker privilege posture) — the hermetic, non-destructive
## successor to the `D2B_LIVE`-against-the-real-host scripts. Needs KVM (a local
## NixOS host; TCG software emulation is the slow fallback when /dev/kvm is
## absent). x86_64-linux only (a same-system VM builder is required).
test-host-integration:
	@set -eu; \
	system="$$(nix eval --raw --impure --expr builtins.currentSystem)"; \
	if [ "$$system" != "x86_64-linux" ]; then \
	echo "test-host-integration: vmChecks are x86_64-linux only (need a same-system VM builder); skipping on $$system"; \
	exit 0; \
	fi; \
	if [ ! -e /dev/kvm ]; then \
	echo "test-host-integration: /dev/kvm absent — runNixOSTest will fall back to slow TCG emulation"; \
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
## test-hardware — G-hw: real GPU/YubiKey/hardware-TPM passthrough + full
## microVM boot. NixOS host WITH the devices only; CI cannot run this.
test-hardware:    ; bash tests/tools/run-layer.sh test-hardware
perf:             ; bash tests/tools/run-layer.sh perf

# --- pre-existing maintainer targets ---------------------------------------

## i3-check — verify no v1.3 deferrals authored (ADR 0022 I3 invariant).
##            Wired into pre-tag and tests/static.sh per panel-docs R1 MF-1.
i3-check:
	bash tests/unit/meta/no-new-deferral.sh

## pre-tag — run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, d2b active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates $${TMPDIR:-/tmp}/d2b-smoke-run-log.txt.
##           ALSO runs the I3 invariant grep gate (ADR 0022 + panel-docs R1).
pre-tag: i3-check
	bash tests/integration/live/live-vm-smoke.sh --full

## smoke-lite — run the single-VM lite smoke gate (≤5 min).
##              Used at every panel-round HEAD per I5.
smoke-lite:
	bash tests/integration/live/live-vm-smoke.sh --lite
