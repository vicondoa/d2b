#!/usr/bin/env bash
# tests/tools/gen-migration-ledger.sh — W0 seed for the test-rearchitecture
# assertion ledger (plan §4/§5, Appendix A).
#
# Emits tests/migration-ledger.toml: one row PER SCRIPT (W0 seed granularity;
# W1+ refine to one row PER ASSERTION). Each row records the target group
# (A-H / G-ci / G-hw / perf), the `make` target, the CI tier, and migration
# status. ORCH/helper scripts are intentionally excluded (they are not test
# cases; they become `make`-target plumbing).
#
# CRITICAL self-check (seed of the `check-inventory` gate, plan §3.9): every
# `tests/*.sh` MUST be assigned to exactly one group or the ORCH exclude list.
# The script fails closed on any unassigned or doubly-assigned file, so a
# newly-landed test cannot escape the taxonomy/ledger/CI.

# shellcheck disable=SC2034  # group vars A..ORCH are consumed via indirect ${!g} expansion
set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
OUT=${1:-$ROOT/tests/migration-ledger.toml}

# --- group → scripts (basenames, no .sh) -----------------------------------
# A: cargo-nextest (logic/argv/DTO, fake-backed canaries, KVM-free runtime).
A="activation-helper-eval audio-argv-shape broker-default-features-build
broker-enum-disposition broker-export-audit broker-scm-rights-fd-lifecycle
broker-socket-acl broker-validate-bundle cgroup-delegation-oracle ch-argv-shape
ch-net-handoff-canary cli-json cli-rust-native-audit cli-rust-native-auth-status
cli-rust-native-host-check cli-rust-native-host-doctor cli-rust-native-list
cli-rust-native-status cli-rust-native-usb cli-vm-verbs-eval daemon-metrics-eval
daemon-socket-acl daemon-state-lock daemon-state-persistence
daemon-version-negotiation dag-topo device-node-matrix gpu-argv-shape
guest-control-proto guest-control-token-materializer guest-proto-bindings
guest-static-elf guest-ttrpc-bindings host-prepare-idempotency
host-prepare-network host-validate-verb-eval ifname-collision ioctl-negative
kernel-module-matrix l1c-privilege-oracle manifest-fuzz-bounded
manifest-v04-roundtrip minijail-version-check nft-coexistence
nft-foreign-rule-preservation nixlingd-startup-smoke otel-host-bridge-argv-shape
path-safety-violation-fs pidfd-handoff bridge-isolation-runtime
runner-shape-preflight runner-shape-snapshot rust-workspace-checks
sidecar-argv-shape ssh-host-key-preflight-eval stub-no-socket usbip-argv-shape
usbip-firewall-skeleton usbip-state-machine-eval video-argv-shape
video-binary-contract virtiofsd-argv-shape w6-argv-shape"

# B: drift vs shipped committed artifact (xtask gen + git diff).
B="bundle-drift cli-json-drift daemon-api-drift error-codes-drift
host-json-drift-gate manpage-completion-drift vms-json-parity
wave-evidence-schema-eval"

# C: contract over rendered fixture artifact (DTO + snapshot).
C="ifname-nix-rust-parity kernel-modules-parity-eval minijail-validator-audio
minijail-validator-cloud-hypervisor minijail-validator-gpu
minijail-validator-otel-host-bridge minijail-validator-swtpm
minijail-validator-usbip minijail-validator-video minijail-validator-virtiofsd
minijail-validator-vsock-relay minijail-validator-wayland-proxy
net-vm-bundle-gate-eval privileges-json-rust-vs-nix-eval
static-invariant-broad-caps static-invariant-opaque-key-ids static-invariant-uid0
static-invariant-world-readable-leak static-invariant-writable-paths
video-sidecar-hardening-eval loki-label-cardinality-eval tempo-budget-eval"

# D: pure-Nix value/option/internal-config introspection (nix-unit).
D="autostart-wiring-eval bridge-ipv6-boot-sysctl-eval broker-bundle-path-eval
broker-caps-eval broker-socket-activation-eval broker-systemd-unit-eval
daemon-autostart-eval daemon-default-compat-eval daemon-experimental-warning-eval
group-migration-fresh-install-eval group-rename-semantic-eval
guest-config-containment-eval guest-control-auth-eval guest-control-vsock-eval
guest-exec-policy-eval ipv6-off-readback kernel-module-matrix-eval
multi-env-daemon-backed net-vm-network-eval niri-vm-borders-eval
observability-eval polkit-allowlist-eval per-vm-state-ownership-eval
readiness-waves-eval restart-policy-eval state-dir-acl-eval store-overlay-emit-eval
store-sync-export-eval umask-roundtrip-eval usbip-gating-eval v1.1-kernel-floor-eval
video-contract-eval volume-mounts-eval"

# E: eval-must-fail (nix-unit Bucket-A value + Bucket-B expectedError).
E="assertions-eval principal-uid-collision-eval supervisor-option-absent-eval"

# F: build/derivation invariant + per-example evals + schema strictness.
F="cli-nix-consumers-eval examples-with-observability-eval
guest-static-consumption-eval legacy-unit-denylist-eval
static-invariant-deny-unknown-fields static-invariant-deny-unknown-fields-w3"

# H: structural/source/doc cross-reference lint (policy scanner).
H="adr-0015-presence-eval adr-index-coverage agents-md-rewrite-eval
changelog-v1-cut-eval ci-coverage cli-contract-coverage deliverable-gate-inventory
guest-control-auth-nongoals guest-control-vsock-helper-static
guest-exec-runtime-static harness-ubuntu-eval host-prep-dag-eval l3-pin-consistency
layer1-self-inventory legacy-group-name-denylist legacy-group-name-denylist-self-test
manpage-completeness-eval microvm-nix-absent-eval no-bash-exec-eval no-new-deferral
otel-acl-migration-eval privileges-doc-completeness-eval
privileges-matrix-completeness processes-json-eval release-tag-eval
static-rust-dependency-direction stop-dag-reconcile-eval tap-dag-contract-doc-eval
tracing-contract-lint vfsd-watchdog-retired-eval vm-submodule-cutover-eval
vm-submodule-eval"

# G-ci: device-free runNixOSTest VM tests (run in CI on KVM job + local NixOS).
GCI="audio audit-forwarding network-isolation nixling-store state-dir-acl-runtime
swtpm-persistence-smoke"

# G-hw: real device passthrough / full microVM boot (OFF-CI, NixOS host w/ devices).
GHW="hardware-smoke-gpu-yubikey live-vm-smoke"

# perf: scheduled timing budgets (stable runner only).
PERF="performance-budgets"

# ORCH / helpers: NOT test cases (become `make`-target plumbing); excluded.
ORCH="static static-fast static-fast-tier0 static-timing runner
preflight-disk-space cli-rust-native-common lib"

group_of() {
  local n="$1" g
  for g in A B C D E F H GCI GHW PERF ORCH; do
    case " $(echo ${!g}) " in *" $n "*) printf '%s' "$g"; return 0;; esac
  done
  printf '%s' "?"
}
target_of() { case "$1" in
  A) echo test-rust;; B) echo test-drift;; C) echo test-contract;;
  D|E) echo test-nix-unit;; F) echo test-flake;; H) echo test-policy;;
  GCI) echo test-integration;; GHW) echo test-hardware;; PERF) echo perf;;
  *) echo "";; esac; }
tier_of() { case "$1" in
  GCI) echo ci-kvm;; GHW) echo manual-hw;; PERF) echo scheduled;; *) echo ci-l1;;
  esac; }

# --- self-check: every tests/*.sh assigned exactly once --------------------
fail=0
for f in "$ROOT"/tests/*.sh; do
  n=$(basename "$f" .sh)
  hits=0
  for g in A B C D E F H GCI GHW PERF ORCH; do
    case " $(echo ${!g}) " in *" $n "*) hits=$((hits+1));; esac
  done
  if [ "$hits" -eq 0 ]; then echo "UNASSIGNED: tests/$n.sh (classify it before merge)" >&2; fail=1; fi
  if [ "$hits" -gt 1 ]; then echo "DOUBLY ASSIGNED: tests/$n.sh ($hits groups)" >&2; fail=1; fi
done
# every named script must still exist on disk (catch deletions/renames)
for g in A B C D E F H GCI GHW PERF ORCH; do
  for n in $(echo ${!g}); do
    [ -f "$ROOT/tests/$n.sh" ] || { echo "LEDGER NAMES MISSING FILE: tests/$n.sh ($g)" >&2; fail=1; }
  done
done
[ "$fail" -eq 0 ] || { echo "check-inventory: ledger does not cover tests/ 1:1 — fix before generating" >&2; exit 1; }

# --- emit -------------------------------------------------------------------
{
  echo "# tests/migration-ledger.toml — AUTO-GENERATED by tests/tools/gen-migration-ledger.sh."
  echo "# W0 seed: one row per script. W1+ refines to one row per assertion."
  echo "# Do not edit by hand; re-run the generator. Seed of the check-inventory gate."
  echo "schema_version = 0"
  echo "generated_for = \"test-rearchitecture W0\""
  echo
  for f in "$ROOT"/tests/*.sh; do
    n=$(basename "$f" .sh); g=$(group_of "$n")
    [ "$g" = "ORCH" ] && continue
    [ "$g" = "?" ] && continue
    echo "[[script]]"
    echo "name = \"tests/$n.sh\""
    echo "group = \"$g\""
    echo "make_target = \"$(target_of "$g")\""
    echo "tier = \"$(tier_of "$g")\""
    echo "status = \"legacy\"      # legacy | porting | ported"
    echo "successor_ids = []"
    echo "exercised_today = \"yes\""
    echo
  done
} > "$OUT"
echo "wrote $OUT ($(grep -c '^\[\[script\]\]' "$OUT") test rows; ORCH excluded)"
