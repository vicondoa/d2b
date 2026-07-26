# shellcheck shell=bash
# tests/tools/heavy-gate-reexec.sh - shared heavy-gate self-guard (ADR 0046).
#
# Sourced by every heavy entrypoint (the container lane, the live lanes, the
# hardware smoke, the performance budgets, and the aggregating runners). Those
# lanes must never bypass the sole-use, two-slot per-uid heavy-gate semaphore
# that serialises heavy work against the shared Nix store, cargo target
# directory, and KVM device.
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
# xtask it builds and runs - from its OWN on-disk location (BASH_SOURCE), never
# from the caller-supplied ROOT or from CARGO_TARGET_DIR. The build target is
# likewise pinned to this checkout's packages/target: the caller's
# CARGO_TARGET_DIR is deliberately ignored, so an absolute or relative value
# pointing at a planted xtask cannot make verify-slot succeed without a slot.
# The build also runs with the caller's build-affecting Cargo/Rust environment
# stripped, so a hostile rustc, rustc-wrapper, cargo config override, or
# RUSTFLAGS cannot substitute the binary; only the in-checkout, trusted
# packages/.cargo/config.toml governs the real toolchain wrapper.
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
  local helper_dir root packages
  helper_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd -P) || {
    echo "heavy-gate self-guard: cannot resolve the helper directory; failing closed" >&2
    return 70
  }
  root=$(cd -- "$helper_dir/../.." >/dev/null 2>&1 && pwd -P) || {
    echo "heavy-gate self-guard: cannot resolve the checkout root; failing closed" >&2
    return 70
  }
  packages="$root/packages"

  # Fail-closed re-exec depth limit. A healthy acquisition needs exactly one
  # re-exec: unheld -> acquire a slot -> re-run the entrypoint -> now held.
  # Anything deeper means the gate handed back a slot the verifier still
  # rejects (a stale or broken binary), so we STOP rather than spin forever.
  local depth="${D2B_HEAVY_GATE_REEXEC_DEPTH:-0}"
  case "$depth" in
    '' | *[!0-9]*) depth=0 ;;
  esac

  # Canonical, checkout-anchored build target. The caller's CARGO_TARGET_DIR is
  # NOT consulted (see the trust model): honouring it - even when absolute -
  # would let a hostile environment point the target at a planted xtask whose
  # verify-slot returns success. The wrapper is always built into, and executed
  # from, the target directory that belongs to THIS checkout.
  local target="$packages/target"
  local xtask="$target/debug/xtask"

  # Ensure freshness: ALWAYS rebuild from the canonical packages dir. Rebuilding
  # only when the binary is absent would run a stale pre-verify-slot xtask as-is;
  # it lacks the subcommand, fails, and the script re-execs through that same
  # stale gate whose child repeats - the unbounded loop this guard must avoid.
  #
  # The build runs in a subshell with the caller's build-affecting Cargo/Rust
  # environment stripped so a planted toolchain cannot inject a substitute
  # binary. Cargo stderr is retained in a private checkout-local directory and
  # passed through xtask's path-safe diagnostic filter if the build fails. A
  # first build with no usable filter suppresses the raw text explicitly.
  local rc=0 build_diag_dir build_err redactor_status=0
  mkdir -p -- "$target" || {
    echo "heavy-gate self-guard: cannot prepare the pinned build target; failing closed" >&2
    return 70
  }
  build_diag_dir="$target/.heavy-gate-build-$BASHPID-$RANDOM"
  (umask 077 && mkdir -- "$build_diag_dir") || {
    echo "heavy-gate self-guard: cannot create private build diagnostics; failing closed" >&2
    return 70
  }
  build_err="$build_diag_dir/cargo.stderr"
  (
    cd -- "$packages" 2>/dev/null \
      && builtin unset CARGO_TARGET_DIR CARGO_BUILD_TARGET_DIR \
               RUSTC RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER \
               CARGO_BUILD_RUSTC CARGO_BUILD_RUSTC_WRAPPER \
               RUSTFLAGS RUSTDOCFLAGS CARGO_ENCODED_RUSTFLAGS CARGO_BUILD_RUSTFLAGS \
      && CARGO_TARGET_DIR="$target" builtin command cargo build --quiet -p xtask
  ) >/dev/null 2>"$build_err" || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "heavy-gate self-guard: xtask build failed under the pinned toolchain (exit $rc); failing closed" >&2
    if [ ! -x "$xtask" ]; then
      echo "heavy-gate self-guard: diagnostic redactor unavailable; raw cargo output suppressed" >&2
    elif [ -n "${HOME:-}" ]; then
      "$xtask" redact-diagnostics --repo-root "$root" --home "$HOME" \
        --tail-lines 20 <"$build_err" >&2 || redactor_status=$?
    else
      "$xtask" redact-diagnostics --repo-root "$root" \
        --tail-lines 20 <"$build_err" >&2 || redactor_status=$?
    fi
    if [ "$redactor_status" -ne 0 ]; then
      echo "heavy-gate self-guard: diagnostic redaction failed; raw cargo output suppressed" >&2
    fi
    rm -rf -- "$build_diag_dir"
    return 70
  fi
  rm -rf -- "$build_diag_dir"

  if [ ! -x "$xtask" ]; then
    echo "heavy-gate self-guard: the freshly built xtask is unavailable; failing closed" >&2
    return 70
  fi

  rc=0
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
