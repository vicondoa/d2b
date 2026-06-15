# Makefile — nixling repository top-level convenience targets.
#
# Maintainer-facing targets only; CI uses .github/workflows/*.yml directly.

.PHONY: pre-tag smoke-lite i3-check \
        check check-ci check-all check-fast check-tier0 \
        test-rust test-drift test-fixtures test-contract test-nix-unit \
        test-flake test-policy test-mutation test-integration test-hardware perf \
        ledger check-inventory

# ===========================================================================
# Test-rearchitecture interface (plan §2.9). The targets are the stable
# contract; the tools behind them change per wave. W0: `check` wraps today's
# static.sh; per-layer targets route through tests/migration-ledger.toml.
#
#   make check          L1 PR gate (A-F,H) — Ubuntu+Nix, any runner. Done-gate.
#   make check-ci       check + test-integration — exactly what CI runs.
#   make check-all      check-ci + test-hardware + perf — full local NixOS gate.
#   make test-<layer>   focused per-layer run (ledger-driven).
#   make test-integration  G-ci runNixOSTest — CI KVM job + local NixOS.
#   make test-hardware     G-hw real GPU/YubiKey/TPM passthrough — NixOS host only.
# ===========================================================================

## check — the Layer-1 PR done-gate. W0: the authoritative static.sh gate.
check:
	bash tests/static.sh

## check-ci — what CI runs: L1 + the device-free VM tier (KVM Ubuntu job).
check-ci: check test-integration

## check-all — the full local gate on a NixOS host with devices.
check-all: check-ci test-hardware perf

## check-fast / check-tier0 — fast PR-loop subsets.
check-fast:
	bash tests/static-fast.sh
check-tier0:
	bash tests/static-fast-tier0.sh

## ledger / check-inventory — (re)generate the migration ledger and assert it
## covers tests/ 1:1 (fails closed on any unclassified/renamed test).
ledger check-inventory:
	bash tests/tools/gen-migration-ledger.sh

## Per-layer targets — W0: run the group's not-yet-ported legacy scripts via
## the ledger. W1+ repoints each to its successor (nextest/nix-unit/VM).
test-rust:        ; bash tests/tools/run-layer.sh test-rust
test-drift:       ; bash tests/tools/run-layer.sh test-drift
test-contract:    ; bash tests/tools/run-layer.sh test-contract
test-nix-unit:    ; bash tests/tools/run-layer.sh test-nix-unit
test-flake:       ; bash tests/tools/run-layer.sh test-flake
test-policy:      ; bash tests/tools/run-layer.sh test-policy
test-fixtures:    ; @echo "test-fixtures: artifact-fixture derivations land in W1/W3 (plan §2.1)"
test-mutation:    ; @echo "test-mutation: standing mutation gate lands W1+ (plan §3.7)"
## test-integration — G-ci device-free runNixOSTest. Runs in CI on a KVM job
## (DeterminateSystems/determinate-nix-action) and on a local NixOS host.
test-integration: ; bash tests/tools/run-layer.sh test-integration
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
