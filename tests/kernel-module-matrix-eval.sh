#!/usr/bin/env bash
# tests/kernel-module-matrix-eval.sh — P3 ph3-p3-kernel-module-check
# matrix-drift regression. Asserts that the REQUIRED / OPTIONAL
# module constants in packages/nixlingd/src/kernel_module_check.rs
# stay in sync with the operator-reference matrix in
# docs/reference/kernel-module-check.md.
#
# The pure check function and the operator reference both encode
# the same matrix; this gate fails if they drift.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/kernel-module-matrix-eval.sh"

SRC="$ROOT/packages/nixlingd/src/kernel_module_check.rs"
DOC="$ROOT/docs/reference/kernel-module-check.md"

[ -f "$SRC" ] || fail "source not found: $SRC"
[ -f "$DOC" ] || fail "operator reference not found: $DOC"

# ------------------------------------------------------------------
# Required modules (always, plus KVM alternatives).
# ------------------------------------------------------------------
expected_required_always=(
  vhost_net
  tun
  virtio_net
  virtio_blk
  virtio_pci
  virtio_console
)

expected_required_kvm=(
  kvm_intel
  kvm_amd
)

# Required-when-feature.
expected_required_conditional_virtiofs="virtiofs"
expected_required_conditional_graphics=(
  udmabuf
  drm_virtgpu
)

# Optional rows.
expected_optional_nvidia=(
  nvidia
  nvidia_uvm
)
expected_optional_usbip="usbip_host"
expected_optional_tpm="tpm_vtpm_proxy"

check_in_file() {
  local label="$1" path="$2" needle="$3"
  if ! grep -qF -- "$needle" "$path"; then
    fail "$label: missing '$needle' in $(basename "$path")"
  fi
  ok "$label: '$needle' present in $(basename "$path")"
}

# Source-side assertions.
for m in "${expected_required_always[@]}"; do
  check_in_file "src REQUIRED_ALWAYS" "$SRC" "\"$m\""
done

for m in "${expected_required_kvm[@]}"; do
  check_in_file "src REQUIRED_KVM_ALTERNATIVES" "$SRC" "\"$m\""
done

check_in_file "src REQUIRED_IF_VIRTIOFS" "$SRC" "\"$expected_required_conditional_virtiofs\""

for m in "${expected_required_conditional_graphics[@]}"; do
  check_in_file "src REQUIRED_IF_GRAPHICS" "$SRC" "\"$m\""
done

for m in "${expected_optional_nvidia[@]}"; do
  check_in_file "src OPTIONAL_GRAPHICS_NVIDIA" "$SRC" "\"$m\""
done

check_in_file "src OPTIONAL_USBIP" "$SRC" "\"$expected_optional_usbip\""
check_in_file "src OPTIONAL_TPM"   "$SRC" "\"$expected_optional_tpm\""

# Doc-side assertions: the operator reference table cites every
# module name backticked. We assert backticked occurrences so an
# accidental rename of one side surfaces immediately.
doc_check() {
  local label="$1" needle="$2"
  if ! grep -qF -- "\`$needle\`" "$DOC"; then
    fail "$label: missing backticked '$needle' in $(basename "$DOC")"
  fi
  ok "$label: '\`$needle\`' present in $(basename "$DOC")"
}

for m in "${expected_required_always[@]}" "${expected_required_kvm[@]}" \
         "$expected_required_conditional_virtiofs" \
         "${expected_required_conditional_graphics[@]}" \
         "${expected_optional_nvidia[@]}" \
         "$expected_optional_usbip" \
         "$expected_optional_tpm"; do
  doc_check "doc matrix" "$m"
done

# Constants in the source must EXACTLY name the canonical idents
# (not the doc-pretty form) so the gate catches a stealth refactor.
for ident in REQUIRED_ALWAYS REQUIRED_KVM_ALTERNATIVES \
             REQUIRED_IF_VIRTIOFS REQUIRED_IF_GRAPHICS \
             OPTIONAL_GRAPHICS_NVIDIA OPTIONAL_USBIP OPTIONAL_TPM; do
  if ! grep -qE "pub const $ident" "$SRC"; then
    fail "src missing public constant: $ident"
  fi
  ok "src declares $ident"
done

# Fatal-typed-error contract: typed_error must carry
# HostKernelModulesMissing at exit code 64 with kind
# host-kernel-modules-missing.
TYPED="$ROOT/packages/nixlingd/src/typed_error.rs"
[ -f "$TYPED" ] || fail "typed_error.rs not found: $TYPED"
grep -qF "HostKernelModulesMissing" "$TYPED" || fail "typed_error: missing HostKernelModulesMissing variant"
grep -qF '"host-kernel-modules-missing"' "$TYPED" || fail "typed_error: missing kind 'host-kernel-modules-missing'"
grep -qE "HostKernelModulesMissing \{ \.\. \} => 64" "$TYPED" || fail "typed_error: missing exit code 64 for HostKernelModulesMissing"
ok "typed_error: HostKernelModulesMissing wired (kind + exit 64)"

log "tests/kernel-module-matrix-eval.sh: matrix is in sync"
