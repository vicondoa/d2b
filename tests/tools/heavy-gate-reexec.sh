# shellcheck shell=bash
# tests/tools/heavy-gate-reexec.sh - shared heavy-gate self-guard (ADR 0046).
#
# Sourced by every heavy entrypoint (the container lane, the live lanes, the
# hardware smoke, the performance budgets, and the aggregating runners). Those
# lanes must never bypass the sole-use, two-slot per-uid heavy-gate semaphore
# that serialises heavy work against the shared Nix store, Bazel output tree,
# and KVM device.
#
# The mere presence of D2B_HEAVY_GATE proves nothing: any process can export
# it, which is exactly the forgeable-marker bypass this helper closes. Instead
# of trusting the variable, we ask the wrapper to VERIFY that this process
# genuinely holds a slot - `xtask heavy-gate verify-slot` re-runs the inode,
# ownership, and atomic F_OFD_SETLK ownership proof through the inherited slot
# descriptor and reports its verdict purely through the exit status:
#
#   0  the caller genuinely holds a slot          -> proceed
#   3  no slot is held                            -> acquire one (bounded re-exec)
#   *  the verifier itself malfunctioned          -> propagate and fail closed
#
# A malfunction (a stale binary with no verify-slot subcommand, an unsupported
# lock, a broken environment) is NOT treated as "unheld": re-execing through a
# broken gate is exactly the infinite loop this helper must not create, so any
# non-0, non-3 status is propagated unchanged and the caller fails closed.
#
# Trust model. This helper derives the canonical checkout - and therefore the
# default xtask runfile - from its OWN on-disk location (BASH_SOURCE), never
# from the caller-supplied ROOT. Bazel owns freshness and dependency inputs;
# this helper only consumes the prebuilt `//packages/xtask:xtask` artifact.
# Callers may provide an equivalent Bazel runfile through D2B_XTASK_BIN.
#
# Usage, from a heavy entrypoint after computing ROOT:
#
#   # shellcheck source=tests/tools/heavy-gate-reexec.sh
#   . "$ROOT/tests/tools/heavy-gate-reexec.sh"
#   d2b_heavy_gate_reexec "$ROOT" "$0" "$@"

# Proceed only when this process genuinely holds a heavy-gate slot; otherwise
# re-exec the caller through the gate to acquire a real slot, bounded so a
# stale or broken binary can never loop forever.
d2b_heavy_gate_reexec() {
  # Bash imports exported functions before this helper runs, and function
  # resolution precedes PATH lookup. Developers commonly export toolchain
  # wrapper functions, so explicitly invoke Bash's cleanup builtin rather than
  # another function that happens to be named `unset`.
  builtin unset -f cargo rustc 2>/dev/null || true

  # $1 is the caller-supplied ROOT; it is deliberately ignored for locating
  # xtask (see the trust model above). $2 is the entrypoint to re-run.
  local self="$2"
  shift 2

  # Canonical checkout root: derived from THIS trusted helper's own location
  # (<root>/tests/tools/heavy-gate-reexec.sh), so it cannot be redirected by a
  # caller-controlled variable.
  local helper_dir root
  helper_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd -P) || {
    echo "heavy-gate self-guard: cannot resolve the helper directory; failing closed" >&2
    return 70
  }
  root=$(cd -- "$helper_dir/../.." >/dev/null 2>&1 && pwd -P) || {
    echo "heavy-gate self-guard: cannot resolve the checkout root; failing closed" >&2
    return 70
  }
  # Fail-closed re-exec depth limit. A healthy acquisition needs exactly one
  # re-exec: unheld -> acquire a slot -> re-run the entrypoint -> now held.
  # Anything deeper means the gate handed back a slot the verifier still
  # rejects (a stale or broken binary), so we STOP rather than spin forever.
  local depth="${D2B_HEAVY_GATE_REEXEC_DEPTH:-0}"
  case "$depth" in
    '' | *[!0-9]*) depth=0 ;;
  esac

  local xtask="${D2B_XTASK_BIN:-$root/bazel-bin/packages/xtask/xtask}"
  if [ ! -x "$xtask" ]; then
    echo "heavy-gate self-guard: Bazel xtask artifact is unavailable; run 'bazel build //packages/xtask:xtask' first" >&2
    return 70
  fi

  local rc=0
  "$xtask" heavy-gate verify-slot || rc=$?
  if [ "$rc" -eq 0 ]; then
    # Genuinely holds a slot. Drop the depth counter so it cannot leak into a
    # nested heavy invocation's environment.
    unset D2B_HEAVY_GATE_REEXEC_DEPTH
    return 0
  fi
  if [ "$rc" -ne 3 ]; then
    echo "heavy-gate self-guard: verify-slot failed closed (exit $rc); refusing to run heavy work unsynchronised" >&2
    return "$rc"
  fi
  if [ "$depth" -ge 1 ]; then
    echo "heavy-gate self-guard: still no verified slot after re-exec (depth $depth); refusing to loop" >&2
    return 70
  fi

  export D2B_HEAVY_GATE_REEXEC_DEPTH=$((depth + 1))
  exec "$xtask" heavy-gate -- bash "$self" "$@"
}
