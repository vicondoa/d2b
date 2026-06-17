# Makefile — nixling repository top-level convenience targets.
#
# Maintainer-facing targets only; CI converges on this stable make-target
# interface incrementally during the test rearchitecture.

.PHONY: pre-tag smoke-lite i3-check \
        check check-ci check-all check-fast check-tier0 \
        test test-unit \
        test-lint test-rust test-proofs test-flake test-nix-unit \
        test-drift test-policy test-integration test-host-integration test-hardware perf \
        ledger ledger-regen check-inventory pr-checklist-gate ci-uses-make nix-unit-pin

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)
NIX_FLAKE := nix --extra-experimental-features 'nix-command flakes'

# ===========================================================================
# Test-rearchitecture interface (plan §2.9). The targets are the stable
# contract; the tools behind them change per wave. W0: `check` wraps today's
# static.sh; per-layer targets route through tests/migration-ledger.toml.
#
#   make check          L1 PR gate (A-F,H) — Ubuntu+Nix, any runner. Done-gate.
#   make check-ci       W0: check, then the test-integration placeholder.
#   make check-all      check-ci + test-hardware + perf — full local NixOS gate.
#   make test-<layer>   focused per-layer run (ledger-driven).
#   make test-integration  W0 placeholder: legacy G-ci only on NixOS+KVM;
#                          runNixOSTest CI harness lands W4.
#   make test-hardware     G-hw real GPU/YubiKey/TPM passthrough — NixOS host only.
# ===========================================================================

## check — the Layer-1 PR done-gate. W0: the authoritative static.sh gate.
check:
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
	bash tests/static-fast-tier0.sh

# ===========================================================================
# Umbrella test targets (local / agent development).
#
#   make test-unit        L1 gate sub-targets (lint, rust, proofs, flake, drift,
#                         policy). The full local-dev fast gate. Use in place of
#                         the old check-fast.
#   make test             test-unit + test-integration (full local gate).
#   make test-integration L2 podman container integration tests.
#
# CI runs the individual sub-targets in parallel; locally, `make test-unit`
# runs them serially (or `make -j test-unit` for parallelism, but beware
# /nix/store contention).
# ===========================================================================

test: test-unit test-integration

test-unit: test-lint test-rust test-proofs test-flake test-drift test-policy

# ===========================================================================
# Sub-targets. Each has a corresponding tests/test-<name>.sh driver.
# ===========================================================================

## test-lint — preflight + nix-instantiate --parse + shellcheck (no eval, no cargo).
test-lint:
	bash tests/test-lint.sh

## test-rust — the comprehensive Rust gate (fmt, clippy, cargo test, contract
## tests with NL_FIXTURES, CLI-contract layer, no-bash-ast-walker, broker
## workspace ×3 feature passes, schema-gen reproducibility, cargo-deny/audit,
## stub-no-socket, assert-pinned-tests).
test-rust:
	bash tests/test-rust.sh

## test-proofs — standalone proof crates under proofs/ (not members of packages/).
test-proofs:
	bash tests/test-proofs.sh

## test-flake — `nix flake check --no-build` for the native system (bounded
## memory). CI runs this as a 2-arch matrix (x86_64 + aarch64).
## Set NL_FLAKE_ALL_SYSTEMS=1 to cross-evaluate both (like `make check`/static.sh).
test-flake:
	bash tests/test-flake.sh

## test-nix-unit — build the nix-unit corpus check (focused convenience target;
## already covered by test-flake, so NOT in test-unit to avoid double work).
test-nix-unit:
	bash tests/test-nix-unit.sh

## test-drift — generated-artifact drift gates (xtask gen-*, vms-json parity).
test-drift:
	bash tests/test-drift.sh

## test-policy — meta gates that guard the test architecture + cross-cutting
## invariants (ci-coverage, ci-uses-make, adr-index, deliverable-gate, etc.).
test-policy:
	bash tests/test-policy.sh

## test-integration — L2 podman container integration tests.
test-integration:
	bash tests/test-integration.sh

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## check-inventory — fail-closed ledger drift check for CI.
check-inventory:
	bash tests/tools/gen-migration-ledger.sh --check

## ledger — compatibility alias for the fail-closed check.
ledger: check-inventory

## ledger-regen — regenerate tests/migration-ledger.toml in place for humans.
ledger-regen:
	bash tests/tools/gen-migration-ledger.sh

## nix-unit-pin — regenerate the fail-closed nix-unit case-presence pins
## (tests/unit/nix/pinned/*.txt) after adding or removing cases.
nix-unit-pin:
	bash tests/tools/gen-nix-unit-pins.sh

## W0 policy gates (also run by test-policy).
pr-checklist-gate:
	bash tests/unit/meta/pr-checklist-gate.sh .github/PULL_REQUEST_TEMPLATE.md
ci-uses-make:
	bash tests/unit/meta/ci-uses-make.sh

## test-host-integration — G-host: runNixOSTest VM integration tests (the
## `vmChecks` flake output, NOT swept by `nix flake check`). Each test boots a
## real NixOS VM with the nixling daemon surface and asserts live broker /
## daemon / host-posture behaviour (socket activation, bridge isolation,
## state-dir ACLs, broker privilege posture) — the hermetic, non-destructive
## successor to the `NL_LIVE`-against-the-real-host scripts. Needs KVM (a local
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
##           Requires: KVM, nixling active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates $${TMPDIR:-/tmp}/nixling-smoke-run-log.txt.
##           ALSO runs the I3 invariant grep gate (ADR 0022 + panel-docs R1).
pre-tag: i3-check
	bash tests/integration/live/live-vm-smoke.sh --full

## smoke-lite — run the single-VM lite smoke gate (≤5 min).
##              Used at every panel-round HEAD per I5.
smoke-lite:
	bash tests/integration/live/live-vm-smoke.sh --lite
