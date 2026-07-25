# shellcheck shell=bash
# tests/tools/heavy-gate-reexec.sh - shared heavy-gate self-guard (ADR 0046).
#
# Sourced by every heavy entrypoint (the live lanes, the hardware smoke, the
# performance budgets, and the aggregating runners). Those lanes must never
# bypass the sole-use, two-slot per-uid heavy-gate semaphore that serialises
# heavy work against the shared Nix store, cargo target directory, and KVM
# device.
#
# The mere presence of D2B_HEAVY_GATE proves nothing: any process can export
# it, which is exactly the forgeable-marker bypass this helper closes. Instead
# of trusting the variable, we ask the wrapper to VERIFY that this process
# genuinely holds a slot - `xtask heavy-gate verify-slot` re-runs the inode,
# ownership, and atomic F_OFD_SETLK ownership proof through the inherited slot
# descriptor and exits nonzero unless a real slot is held. Only a verified
# slot proceeds; anything else re-execs through the gate exactly once to
# acquire a real slot. The re-exec inherits the locked descriptor, so the
# second pass verifies and proceeds without acquiring a second slot.
#
# Usage, from a heavy entrypoint after computing ROOT:
#
#   # shellcheck source=tests/tools/heavy-gate-reexec.sh
#   . "$ROOT/tests/tools/heavy-gate-reexec.sh"
#   d2b_heavy_gate_reexec "$ROOT" "$0" "$@"

# Proceed only when this process genuinely holds a heavy-gate slot; otherwise
# re-exec the caller through the gate exactly once to acquire a real slot.
d2b_heavy_gate_reexec() {
  local root="$1" self="$2"
  shift 2
  local xtask="${CARGO_TARGET_DIR:-$root/packages/target}/debug/xtask"
  if [ ! -x "$xtask" ]; then
    ( cd "$root/packages" && cargo build --quiet -p xtask ) >&2
  fi
  if "$xtask" heavy-gate verify-slot; then
    return 0
  fi
  exec "$xtask" heavy-gate -- bash "$self" "$@"
}
