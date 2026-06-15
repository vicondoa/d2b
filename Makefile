# Makefile — nixling repository top-level convenience targets.
#
# Maintainer-facing targets only; CI converges on this stable make-target
# interface incrementally during the test rearchitecture.

.PHONY: pre-tag smoke-lite i3-check \
	        check check-ci check-all check-fast check-tier0 \
	        test-rust test-drift test-fixtures test-contract test-nix-unit \
	        test-flake test-policy test-mutation test-integration test-hardware perf \
	        ledger ledger-regen check-inventory pr-checklist-gate ci-uses-make

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
check-fast:
	bash tests/static-fast.sh
check-tier0:
	bash tests/static-fast-tier0.sh

## check-inventory — fail-closed ledger drift check for CI.
check-inventory:
	bash tests/tools/gen-migration-ledger.sh --check

## ledger — compatibility alias for the fail-closed check.
ledger: check-inventory

## ledger-regen — regenerate tests/migration-ledger.toml in place for humans.
ledger-regen:
	bash tests/tools/gen-migration-ledger.sh

## W0 policy gates. Warn-only in aggregate CI today; CI wiring lands later.
pr-checklist-gate:
	bash tests/pr-checklist-gate.sh .github/PULL_REQUEST_TEMPLATE.md
ci-uses-make:
	bash tests/ci-uses-make.sh

## Per-layer targets — W0: run the group's not-yet-ported legacy scripts via
## the ledger. W1+ repoints each to its successor (nextest/nix-unit/VM).
test-rust:
	bash tests/tools/run-layer.sh test-rust
	bash tests/tools/assert-pinned-tests.sh
	set -eu; \
	if ! command -v cargo >/dev/null 2>&1; then \
	  for candidate in "$$HOME"/.rustup/toolchains/1.94.1-*/bin; do \
	    if [ -x "$$candidate/cargo" ]; then PATH="$$candidate:$$PATH"; export PATH; break; fi; \
	  done; \
	fi; \
	CARGO_BUILD_RUSTC_WRAPPER='' RUSTC_WRAPPER='' nix shell --quiet --inputs-from . nixpkgs#cargo-nextest nixpkgs#gcc --command bash -c 'cd packages && cargo nextest run --workspace --exclude nixling-contract-tests'; \
	cd packages; \
	CARGO_BUILD_RUSTC_WRAPPER='' RUSTC_WRAPPER='' cargo test --doc --workspace --exclude nixling-contract-tests
	bash tests/tools/assert-pinned-tests.sh
test-drift:       ; bash tests/tools/run-layer.sh test-drift
test-contract:
	bash tests/tools/run-layer.sh test-contract
	@set -eu; \
	system="$$(nix eval --raw --impure --expr builtins.currentSystem)"; \
	fixtures="$$(nix build --no-link --print-out-paths ".#checks.$$system.fixture-smoke")"; \
	printf 'NL_FIXTURES=%s\n' "$$fixtures"; \
	cd packages; \
	if command -v cargo-nextest >/dev/null 2>&1; then \
	  NL_FIXTURES="$$fixtures" cargo nextest run -p nixling-contract-tests; \
	elif command -v nix >/dev/null 2>&1; then \
	  NL_FIXTURES="$$fixtures" nix shell --inputs-from .. nixpkgs#cargo-nextest -c cargo nextest run -p nixling-contract-tests; \
	else \
	  NL_FIXTURES="$$fixtures" cargo test -p nixling-contract-tests; \
	fi
## test-nix-unit — W2: legacy D/E eval-gates still on bash, then the
## migrated nix-unit value/throw corpus (flake.checks.<sys>.nix-unit).
test-nix-unit:
	bash tests/tools/run-layer.sh test-nix-unit
	$(NIX_FLAKE) build --no-link --print-out-paths '.#checks.$(SYSTEM).nix-unit'
test-flake:       ; bash tests/tools/run-layer.sh test-flake
test-policy:      ; bash tests/tools/run-layer.sh test-policy
test-fixtures:
	@set -eu; \
	system="$$(nix eval --raw --impure --expr builtins.currentSystem)"; \
	nix build --no-link --print-out-paths ".#checks.$$system.fixture-smoke"
test-mutation:    ; @echo "test-mutation: standing mutation gate lands W1+ (plan §3.7)"
## test-integration — W0 placeholder: run legacy G-ci only on a local NixOS host
## with KVM. The runNixOSTest CI job lands in W4; do not run live-host scripts
## on generic CI runners.
test-integration:
	@if [ -e /dev/kvm ] && [ -f /etc/NIXOS ]; then \
	bash tests/tools/run-layer.sh test-integration; \
	else \
	echo "G-ci legacy tests need a NixOS host; runNixOSTest harness lands W4"; \
	fi
## test-hardware — G-hw: real GPU/YubiKey/hardware-TPM passthrough + full
## microVM boot. NixOS host WITH the devices only; CI cannot run this.
test-hardware:    ; bash tests/tools/run-layer.sh test-hardware
perf:             ; bash tests/tools/run-layer.sh perf

# --- pre-existing maintainer targets ---------------------------------------

## i3-check — verify no v1.3 deferrals authored (ADR 0022 I3 invariant).
##            Wired into pre-tag and tests/static.sh per panel-docs R1 MF-1.
i3-check:
	bash tests/no-new-deferral.sh

## pre-tag — run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, nixling active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates tests/smoke-run-log.txt.
##           ALSO runs the I3 invariant grep gate (ADR 0022 + panel-docs R1).
pre-tag: i3-check
	bash tests/live-vm-smoke.sh --full

## smoke-lite — run the single-VM lite smoke gate (≤5 min).
##              Used at every panel-round HEAD per I5.
smoke-lite:
	bash tests/live-vm-smoke.sh --lite
