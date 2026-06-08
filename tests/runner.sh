#!/usr/bin/env bash
# Aggregating wrapper for the nixling test layers.
#
# Runs in a deterministic order:
#   static.sh -> nixling-store.sh -> audio.sh -> every security-*.sh (sorted)
#
# This is the single gate that the `security/hardening` branch's daily
# smoke is checked against. Sub-scripts have varying dispatcher
# vocabularies; this wrapper only forwards a flag (--quick / --only /
# --list) to a sub-script that actually advertises it (detected by
# grepping the script's source). Scripts that don't speak the flag are
# either run bare (--quick / --full) or rendered as a single synthetic
# row (--list) or skipped with a SKIP line (--only).
#
# Modes:
#   runner.sh                  # default: --quick (fast subset of every layer)
#   runner.sh --full           # full mode (no --quick passed down)
#   runner.sh --only <pattern> # forwarded to every layer that supports --only
#   runner.sh --list           # list every available test across every layer
#
# --only matching rules:
#   * Scripts that advertise --only are invoked with --only <pattern>;
#     their per-script "Summary: X passed, Y failed" line is inspected
#     to learn whether the pattern matched any of their tests.
#   * A script that does NOT advertise --only (e.g. static.sh,
#     security-baseline.sh) is invoked BARE when <pattern> matches its
#     synthetic name (the basename minus `.sh`, e.g. `security-baseline`).
#   * If no script ended up matching the pattern, the runner exits
#     non-zero with `no test matched pattern '<pattern>'` -- otherwise
#     a typo in the pattern would silently exit 0.
#
# Each invocation mints its own output dir so concurrent runs don't
# clobber each other:
#   RUN_ID   = <UTC timestamp>-<pid>
#   RUN_ROOT = $NL_RUN_ROOT (if set, e.g. for CI)
#              else /run/nixling-runner                (when EUID == 0)
#              else $HOME/.local/state/nixling-runner  (otherwise, per XDG)
#   RUN_DIR  = $RUN_ROOT/<RUN_ID>
#     aggregate.log    — the shared lib.sh log (NL_LOG) for this run
#     <scriptname>.log — per-script stdout+stderr
# RUN_ROOT is created mode 0700 owned by $EUID and validated on every
# run; the prior /tmp/nixling-runner path is no longer used (it was
# world-writable and let an unprivileged user influence root's log
# path via symlink races). Validation is fail-closed: if RUN_ROOT
# already exists with the wrong owner, wrong mode, or as a symlink,
# the runner exits non-zero and asks the user to fix it manually --
# the runner NEVER `rm -rf`s an existing RUN_ROOT (the previous
# behaviour was unsafe with overrides like NL_RUN_ROOT=/home/<user>).
# On a fully-green run $RUN_ROOT/latest is symlinked to the new
# RUN_DIR and older successful runs are pruned (newest 10 kept).
# Failed runs are always retained for diagnostics.
#
# Honors NL_VMS and NL_RUN_ROOT — NL_VMS is passed through to children
# via the environment so `nixling-store.sh` and the security-* scripts
# target the same VM set; NL_RUN_ROOT pins the log root (useful for CI).

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")

# Fixed-order layers. static.sh has no --quick flag and is cheap, so it
# always runs as-is. nixling-store.sh and audio.sh both speak the
# --quick / --only / --list dispatcher contract.
SCRIPTS=( static.sh nixling-store.sh audio.sh )

# Append every security-*.sh in lexical order. We use shell glob
# expansion sorted by LC_ALL=C so phases can drop new files in without
# touching this runner.
shopt -s nullglob
SEC=( "$HERE"/security-*.sh )
shopt -u nullglob
if [ "${#SEC[@]}" -gt 0 ]; then
  mapfile -t SEC_SORTED < <(printf '%s\n' "${SEC[@]}" | LC_ALL=C sort)
  for s in "${SEC_SORTED[@]}"; do
    SCRIPTS+=( "$(basename "$s")" )
  done
fi

mode="quick"
only=""
while [ $# -gt 0 ]; do
  case "$1" in
    --full)  mode="full" ;;
    --quick) mode="quick" ;;
    --only)  mode="only"; only="${2:-}"; shift ;;
    --list)  mode="list" ;;
    -h|--help)
      sed -n '2,52p' "$0"
      exit 0
      ;;
    *)
      echo "runner.sh: unknown arg: $1" >&2
      exit 2
      ;;
  esac
  shift
done

# ---------- per-run output dir ----------
# Mint a unique id per invocation so two concurrent runs do not stomp
# each other's logs (the shared NL_LOG used by lib.sh in particular).
# RUN_ROOT is chosen to avoid the world-writable /tmp directory: it is
# root-only /run/nixling-runner when invoked as root, the invoking
# user's XDG state dir otherwise. $NL_RUN_ROOT overrides both.
NL_RUN_ROOT_IS_OVERRIDE=0
if [ -n "${NL_RUN_ROOT:-}" ]; then
  RUN_ROOT="$NL_RUN_ROOT"
  NL_RUN_ROOT_IS_OVERRIDE=1
elif [ "$EUID" -eq 0 ]; then
  RUN_ROOT="/run/nixling-runner"
else
  RUN_ROOT="${HOME:?HOME must be set when not running as root}/.local/state/nixling-runner"
fi

# Critical system mount points / top-level dirs. An NL_RUN_ROOT
# override must neither equal one of these nor be a direct child of
# one (the latter catches /home/<user>, /etc/<anything>, /var/<anything>,
# …). The list is intentionally non-exhaustive but covers the obvious
# ones the spec calls out.
RUN_ROOT_CRITICAL_PATHS=( / /etc /var /home /run /srv /opt /usr /boot /sys /proc /dev /root /nix /lib /lib64 /bin /sbin /mnt /media )

# Verify an NL_RUN_ROOT override path is structurally sensible. Bails
# with a clear error to stderr on any violation; does NOT touch the
# path. Default RUN_ROOT values skip these structural checks because
# they are well-known canonical paths (e.g. /run/nixling-runner has a
# critical parent /run by construction).
validate_run_root_override() {
  local d="$1"
  if [[ "$d" != /* ]]; then
    printf 'runner.sh: NL_RUN_ROOT must be an absolute path; got: %s\n' "$d" >&2
    exit 4
  fi
  local c
  for c in "${RUN_ROOT_CRITICAL_PATHS[@]}"; do
    if [ "$d" = "$c" ]; then
      printf 'runner.sh: NL_RUN_ROOT refuses critical system path: %s\n' "$d" >&2
      exit 4
    fi
  done
  local parent
  parent=$(dirname -- "$d")
  for c in "${RUN_ROOT_CRITICAL_PATHS[@]}"; do
    if [ "$parent" = "$c" ]; then
      printf 'runner.sh: NL_RUN_ROOT refuses path whose immediate parent is a critical mount: %s (parent=%s)\n' "$d" "$parent" >&2
      exit 4
    fi
  done
  if [ ! -d "$parent" ]; then
    printf 'runner.sh: NL_RUN_ROOT parent directory does not exist: %s (for %s)\n' "$parent" "$d" >&2
    exit 4
  fi
  if [ ! -w "$parent" ]; then
    printf 'runner.sh: NL_RUN_ROOT parent directory is not writable by uid %s: %s (for %s)\n' "$EUID" "$parent" "$d" >&2
    exit 4
  fi
}

# Verify an existing RUN_ROOT is safe to reuse. Fails closed (exits
# non-zero with a clear message) if it is a symlink, not a directory,
# owned by another user, or has wrong mode. Does NOT delete or modify
# the path -- the user must fix it manually. Returns 0 cleanly if the
# path is absent (caller will create it) or fully compliant.
validate_existing_run_root() {
  local d="$1"
  if [ -L "$d" ]; then
    printf 'runner.sh: RUN_ROOT %s is a symlink; refusing to follow it (fix manually)\n' "$d" >&2
    exit 4
  fi
  if [ ! -e "$d" ]; then
    return 0
  fi
  if [ ! -d "$d" ]; then
    printf 'runner.sh: RUN_ROOT %s exists but is not a directory; refusing (fix manually)\n' "$d" >&2
    exit 4
  fi
  local owner mode
  owner=$(stat -c '%u' "$d" 2>/dev/null || printf '')
  mode=$(stat -c '%a' "$d" 2>/dev/null || printf '')
  if [ "$owner" != "$EUID" ]; then
    printf 'runner.sh: RUN_ROOT %s has wrong owner uid %s (expected %s); fix manually or remove\n' "$d" "$owner" "$EUID" >&2
    exit 4
  fi
  if [ "$mode" != "700" ]; then
    printf 'runner.sh: RUN_ROOT %s has wrong mode 0%s (expected 0700); fix manually or remove\n' "$d" "$mode" >&2
    exit 4
  fi
}

# Ensure RUN_ROOT is present and compliant. Validates override
# structure (for NL_RUN_ROOT paths) and existing-dir state (always);
# only creates the directory when it is fully absent, with strict
# 0700 perms. Never destructive: an existing non-compliant RUN_ROOT
# is reported and the script exits -- the previous behaviour of
# `rm -rf -- "$d"` could nuke arbitrary user data (e.g.
# NL_RUN_ROOT=/home/<user>).
#
# security-r4-1: RUN_ROOT is CANONICALIZED before any other validation
# so the critical-path block-list cannot be bypassed via traversal
# segments (e.g. /tmp/foo/../etc/nixos) or via paths that traverse
# through an attacker-created symlink above the leaf (e.g.
# /tmp/symlinktest/parent/safe where parent -> /etc). The canonical
# path is then written back into the global RUN_ROOT so every
# subsequent operation (RUN_DIR, install -d, latest symlink, prune)
# operates on the canonical path -- never on the raw user-supplied one.
ensure_run_root() {
  local d="$1"
  local is_override="$2"

  # Refuse a raw RUN_ROOT that is itself a symlink (round-3 behaviour):
  # we want the user to fix the symlink manually, not silently chase it
  # to wherever it points.
  if [ -L "$d" ]; then
    printf 'runner.sh: RUN_ROOT %s is a symlink; refusing to follow it (fix manually)\n' "$d" >&2
    exit 4
  fi

  # Canonicalize BEFORE any other validation. `realpath -e` requires
  # every component to exist; for a not-yet-created leaf we canonicalize
  # the parent (which MUST exist for the dir to be creatable) and append
  # the leaf so we still have a fully real prefix to validate against.
  local canonical
  if [ -e "$d" ]; then
    if ! canonical=$(realpath -e -- "$d" 2>/dev/null); then
      printf 'runner.sh: RUN_ROOT %s cannot be canonicalized (realpath -e failed)\n' "$d" >&2
      exit 4
    fi
  else
    local leaf parent parent_canon
    leaf=$(basename -- "$d")
    parent=$(dirname -- "$d")
    if ! parent_canon=$(realpath -e -- "$parent" 2>/dev/null); then
      printf 'runner.sh: RUN_ROOT parent does not exist: %s (for %s)\n' "$parent" "$d" >&2
      exit 4
    fi
    canonical="$parent_canon/$leaf"
  fi

  # Defense in depth: if the override path contained literal `..` AND
  # canonicalization changed it, refuse the traversal even if it would
  # have landed somewhere safe. Catches NL_RUN_ROOT=/tmp/foo/../etc/nixos
  # whether or not /tmp/foo happens to exist.
  if [ "$is_override" = "1" ] && [ "$canonical" != "$d" ] && [[ "$d" == *..* ]]; then
    printf 'runner.sh: NL_RUN_ROOT contains traversal segments (..); refusing: %s (resolves to %s)\n' "$d" "$canonical" >&2
    exit 4
  fi

  # Walk the canonical path; every component must be a real directory
  # entry, not a symlink. After `realpath -e` this is true by
  # construction -- belt-and-braces defense.
  local walk="$canonical"
  while [ -n "$walk" ] && [ "$walk" != "/" ]; do
    if [ -L "$walk" ]; then
      printf 'runner.sh: RUN_ROOT canonical path component is a symlink: %s (component=%s)\n' "$canonical" "$walk" >&2
      exit 4
    fi
    walk=$(dirname -- "$walk")
  done

  # Apply the existing critical-path block-list AGAINST THE CANONICAL
  # path, not the raw user-supplied path. This catches paths that
  # resolve to /etc/nixos (or any other critical mount) via traversal
  # segments or via a symlink anywhere above the leaf.
  if [ "$is_override" = "1" ]; then
    validate_run_root_override "$canonical"
  fi
  validate_existing_run_root "$canonical"

  if [ ! -e "$canonical" ]; then
    local pdir
    pdir=$(dirname -- "$canonical")
    mkdir -p -- "$pdir"
    if [ "$EUID" -eq 0 ]; then
      install -d -m 0700 -o root -g root -- "$canonical"
    else
      install -d -m 0700 -- "$canonical"
    fi
  fi

  # Publish the canonical path back to the caller via the global
  # RUN_ROOT so all subsequent operations use it.
  RUN_ROOT="$canonical"
}

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
# Validate (and create-if-absent) RUN_ROOT in every mode -- including
# --list -- so dangerous NL_RUN_ROOT overrides fail closed before any
# I/O. ensure_run_root canonicalizes RUN_ROOT in place (security-r4-1),
# so RUN_DIR is derived AFTER the call to ensure it uses the canonical
# path. Only the RUN_DIR creation and aggregate log are gated on
# non-list mode.
ensure_run_root "$RUN_ROOT" "$NL_RUN_ROOT_IS_OVERRIDE"
RUN_DIR="$RUN_ROOT/$RUN_ID"
if [ "$mode" != "list" ]; then
  mkdir -p -- "$RUN_DIR"
  export NL_LOG="$RUN_DIR/aggregate.log"
  : > "$NL_LOG"
  printf 'runner.sh: RUN_ROOT=%s  RUN_ID=%s\n' "$RUN_ROOT" "$RUN_ID"
fi

# ---------- helpers ----------

# Does <script> advertise <flag>? We grep its source for the literal
# string. Intentionally simple and string-based; if a script wants to
# participate in --quick / --only / --list, it has to mention the flag
# in its case statement (or at least its usage banner).
script_supports() {
  local path="$1" flag="$2"
  grep -q -- "$flag" "$path"
}

# Did the script's per-script log report that at least one of its own
# tests ran under --only? We look at the last "Summary: X passed, Y
# failed" line (printed by lib.sh log()). Returns 0 (matched) /
# 1 (no match or no summary).
script_matched_tests() {
  local log="$1"
  local summary
  summary=$(grep -E 'Summary: [0-9]+ passed, [0-9]+ failed' "$log" 2>/dev/null | tail -n1)
  [ -n "$summary" ] || return 1
  case "$summary" in
    *"Summary: 0 passed, 0 failed"*) return 1 ;;
    *)                               return 0 ;;
  esac
}

# Set by run_one when a script either ran with --only and the script
# reported a non-zero test count, or was invoked bare because its
# synthetic name matched the pattern. Consulted at the end so we can
# fail loudly when --only matched nothing at all.
MATCHED_ANY=0

# Run one script in the chosen mode. Returns:
#   0 -> ran and passed (or was a no-op SKIP)
#   1 -> ran and failed (one or more tests failed)
#   2 -> script missing (counted separately, NOT a failure)
run_one() {
  local script="$1"
  local path="$HERE/$script"
  local base="${script%.sh}"
  local log="$RUN_DIR/${base}.log"

  if [ ! -f "$path" ]; then
    printf '  MISSING: %s (skipped, not yet implemented)\n' "$script"
    return 2
  fi

  local has_quick=0 has_only=0 has_list=0
  script_supports "$path" '--quick' && has_quick=1
  script_supports "$path" '--only'  && has_only=1
  script_supports "$path" '--list'  && has_list=1

  local -a args=()
  case "$mode" in
    list)
      if [ "$has_list" = 1 ]; then
        printf '%s:\n' "$script"
        bash "$path" --list 2>/dev/null | sed 's/^/  /'
      else
        # Surface scripts with no internal --list as a single synthetic
        # row so the pattern is discoverable for --only.
        printf '%s:\n  %s\n' "$script" "$base"
      fi
      return 0
      ;;
    only)
      if [ "$has_only" = 1 ]; then
        args=( --only "$only" )
      elif [ "$only" = "$base" ]; then
        # Script doesn't speak --only, but its synthetic name matches
        # the pattern: run it bare and credit the match so the runner
        # doesn't fall through to "no test matched".
        args=()
        MATCHED_ANY=1
      else
        printf '  SKIP: %s does not support --only and synthetic name %q does not match %q\n' \
          "$script" "$base" "$only"
        return 0
      fi
      ;;
    quick)
      if [ "$has_quick" = 1 ]; then
        args=( --quick )
      fi
      ;;
    full)
      args=()
      ;;
  esac

  printf '\n========== %s %s ==========\n' "$script" "${args[*]:-(no args)}"
  local rc=0
  bash "$path" "${args[@]}" >"$log" 2>&1 || rc=$?

  # When the script speaks --only we can ask its summary line whether
  # the pattern hit anything; bare invocations are credited above.
  if [ "$mode" = "only" ] && [ "$has_only" = 1 ] && script_matched_tests "$log"; then
    MATCHED_ANY=1
  fi

  if [ "$rc" -eq 0 ]; then
    printf '  RESULT: PASS  (log: %s)\n' "$log"
    tail -n 3 "$log" | sed 's/^/    | /'
    return 0
  fi
  printf '  RESULT: FAIL  (log: %s)\n' "$log"
  tail -n 3 "$log" | sed 's/^/    | /'
  return 1
}

# ---------- main ----------

if [ "$mode" = "list" ]; then
  printf 'nixling test runner — available tests\n\n'
fi

pass=0
fail=0
missing=0
ran=0

for s in "${SCRIPTS[@]}"; do
  rc=0
  run_one "$s" || rc=$?
  case "$rc" in
    0) pass=$((pass+1));    ran=$((ran+1)) ;;
    1) fail=$((fail+1));    ran=$((ran+1)) ;;
    2) missing=$((missing+1)) ;;
    *) fail=$((fail+1));    ran=$((ran+1)) ;;
  esac
done

if [ "$mode" = "list" ]; then
  exit 0
fi

printf '\n==========================================\n'
if [ -n "$only" ]; then
  printf 'runner.sh summary (mode: %s / only=%s)\n' "$mode" "$only"
else
  printf 'runner.sh summary (mode: %s)\n' "$mode"
fi
printf '  run id          : %s\n'  "$RUN_ID"
printf '  run dir         : %s\n'  "$RUN_DIR"
printf '  scripts run     : %d\n'  "$ran"
printf '  scripts passed  : %d\n'  "$pass"
printf '  scripts failed  : %d\n'  "$fail"
printf '  scripts missing : %d\n'  "$missing"
printf '==========================================\n'

# --only with a pattern that matched nothing anywhere is itself a
# failure -- otherwise a typo in <pattern> would silently exit 0
# while reporting 0 failed.
if [ "$mode" = "only" ] && [ "$MATCHED_ANY" -eq 0 ] && [ "$fail" -eq 0 ]; then
  printf "runner.sh: no test matched pattern '%s'\n" "$only" >&2
  exit 3
fi

# On a fully-green run, update the convenience symlink and prune older
# successful runs (newest 10 kept). Failed runs are always retained.
# The "latest" symlink is swapped atomically via a tmp+rename. The
# prune only ever deletes immediate children of RUN_ROOT whose leaf
# name strictly matches the RUN_ID regex; nothing else under RUN_ROOT
# is touched, even if some unrelated file or directory has been
# placed there.
if [ "$fail" -eq 0 ]; then
  ln -sfT -- "$RUN_ID" "$RUN_ROOT/latest.tmp"
  mv -T -- "$RUN_ROOT/latest.tmp" "$RUN_ROOT/latest"
  if [ -d "$RUN_ROOT" ]; then
    find "$RUN_ROOT" -mindepth 1 -maxdepth 1 -type d \
         -regextype posix-extended \
         -regex '.*/[0-9]{8}T[0-9]{6}Z-[0-9]+$' \
         -printf '%f\n' 2>/dev/null \
      | sort -r \
      | tail -n +11 \
      | while read -r old; do
          # Belt-and-braces: re-check the leaf name matches the
          # RUN_ID format before deleting. Never rm anything that
          # doesn't fit the pattern, regardless of what find returned.
          if [[ "$old" =~ ^[0-9]{8}T[0-9]{6}Z-[0-9]+$ ]]; then
            rm -rf -- "${RUN_ROOT:?}/$old"
          fi
        done
  fi
fi

[ "$fail" -eq 0 ]
