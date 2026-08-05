#!/usr/bin/env bash
# Headless autopilot runner.
#
# Interactive work does NOT need this script. Per-lane model, effort, and
# context binding is carried by the skill dispatch tables, so an ordinary toad
# or `copilot` session already binds every role correctly. This exists only for
# an unattended run with no terminal attached: it pins the session binding, the
# ceilings, and the log destination so a run cannot quietly drift or bill
# without bound.
#
# Usage:
#   scripts/copilot/autopilot.sh [--resume <id>] [--auto-merge] [-- <extra copilot args>]
#
# Environment overrides, all optional:
#   D2B_AUTOPILOT_MODEL      session model      (default gpt-5.6-sol)
#   D2B_AUTOPILOT_EFFORT     session effort     (default xhigh)
#   D2B_AUTOPILOT_CONTINUES  continuation cap   (default 40)
#   D2B_AUTOPILOT_CREDITS    AI credit ceiling  (default 200)
#   D2B_AUTOPILOT_LOG_DIR    log directory      (default .scratch/autopilot/logs)
#   D2B_AUTOPILOT_LOG_KEEP   run logs retained  (default 20, 0 disables pruning)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

MODEL="${D2B_AUTOPILOT_MODEL:-gpt-5.6-sol}"
EFFORT="${D2B_AUTOPILOT_EFFORT:-xhigh}"
CONTINUES="${D2B_AUTOPILOT_CONTINUES:-40}"
CREDITS="${D2B_AUTOPILOT_CREDITS:-200}"
LOG_DIR="${D2B_AUTOPILOT_LOG_DIR:-$ROOT/.scratch/autopilot/logs}"

RESUME=""
AUTO_MERGE=0
EXTRA=()

while [ $# -gt 0 ]; do
  case "$1" in
    --resume)
      [ $# -ge 2 ] || { echo "autopilot: --resume needs a session id" >&2; exit 2; }
      RESUME="$2"
      shift 2
      ;;
    --auto-merge)
      AUTO_MERGE=1
      shift
      ;;
    --)
      shift
      EXTRA=("$@")
      break
      ;;
    -h|--help)
      # Print the header comment block, however long it grows. A pinned line
      # range silently truncates the help the moment the header changes.
      sed -n '2,${/^#/!q;p;}' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "autopilot: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# Fail closed on a dirty tree. An unattended run that starts on top of somebody
# else's uncommitted work cannot tell its own diff from theirs, and the panel
# round would review both.
if [ -n "$(git status --porcelain)" ]; then
  echo "autopilot: refusing to start on a dirty worktree." >&2
  echo "  Commit or stash first; a run cannot distinguish its own diff from pre-existing edits." >&2
  git status --short >&2
  exit 3
fi

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
for protected in main v3; do
  if [ "$BRANCH" = "$protected" ]; then
    echo "autopilot: refusing to run directly on the protected branch '$BRANCH'." >&2
    echo "  Open a worktree first: git worktree add -b <branch> ../d2b-<name> $protected" >&2
    exit 3
  fi
done

# The binding table must agree with the agents before a lane is ever dispatched.
# A mispinned effort is a silent downgrade that produces a plausible-looking
# panel record, so this runs first and hard-fails.
node scripts/copilot/check-bindings.mjs

mkdir -p "$LOG_DIR"

# Bound the log directory. A multi-day unattended run is the whole point of
# this launcher, and an unbounded log directory inside the worktree is how
# that run ends: not on a stopping condition, but on a full disk. Keeping a
# fixed number of the most recent runs is enough to diagnose the last
# failure, which is all these logs are for.
LOG_KEEP="${D2B_AUTOPILOT_LOG_KEEP:-20}"
if [ "$LOG_KEEP" -gt 0 ] 2>/dev/null; then
  find "$LOG_DIR" -mindepth 1 -maxdepth 1 -printf '%T@\t%p\0' 2>/dev/null \
    | sort -z -rn \
    | tail -z -n "+$((LOG_KEEP + 1))" \
    | cut -z -f2- \
    | while IFS= read -r -d '' stale; do rm -rf -- "$stale"; done
fi

PROMPT="/d2b-autopilot"
if [ "$AUTO_MERGE" -eq 1 ]; then
  PROMPT="/d2b-autopilot --auto-merge"
fi

ARGS=(
  --mode autopilot
  --model "$MODEL"
  --effort "$EFFORT"
  --no-ask-user
  --allow-all-tools
  --max-autopilot-continues "$CONTINUES"
  --max-ai-credits "$CREDITS"
  --log-dir "$LOG_DIR"
  --log-level info
)

if [ -n "$RESUME" ]; then
  ARGS+=(--resume="$RESUME")
else
  ARGS+=(-p "$PROMPT")
fi

if [ ${#EXTRA[@]} -gt 0 ]; then
  ARGS+=("${EXTRA[@]}")
fi

echo "autopilot: branch=$BRANCH model=$MODEL effort=$EFFORT continues=$CONTINUES credits=$CREDITS"
echo "autopilot: logs -> $LOG_DIR"
exec copilot "${ARGS[@]}"
