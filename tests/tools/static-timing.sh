#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../.." && pwd)}
# Default timing log locations to outside $ROOT so the
# append churn doesn't race builtins.getFlake source captures during
# flake-eval gates. Operators who need them in-tree can still
# override NL_STATIC_TIMING_REPORT / NL_STATIC_TIMING_RAW.
_STATIC_TIMING_TMPDIR=${TMPDIR:-/tmp}/nixling-static-timing-report.$$
mkdir -p "$_STATIC_TIMING_TMPDIR"
REPORT_LOG=${NL_STATIC_TIMING_REPORT:-$_STATIC_TIMING_TMPDIR/report.log}
RAW_LOG=${NL_STATIC_TIMING_RAW:-$_STATIC_TIMING_TMPDIR/raw.log}

render_report() {
  local raw_log="$1" report_log="$2"
  if [ ! -s "$raw_log" ]; then
    {
      printf '# tests/static.sh timing\n'
      printf '(no timing data captured)\n'
    } > "$report_log"
    return 0
  fi

  awk -F '\t' '
    $1 == "BEGIN" {
      begin[$2] = $3
      if (first == 0 || $3 < first) {
        first = $3
      }
      labels[$2] = 1
      next
    }
    $1 == "END" {
      end[$2] = $3
      elapsed[$2] = $4
      labels[$2] = 1
    }
    END {
      for (label in labels) {
        if (label in end) {
          printf "%s\t%s\t%s\t%s\n", end[label], label, elapsed[label], first
        } else {
          printf "%s\t%s\t%s\t%s\n", begin[label], label, -1, first
        }
      }
    }
  ' "$raw_log" \
    | sort -n \
    | awk -F '\t' '
      function fmt(ms, total_seconds, minutes, hours, seconds, rem_ms) {
        if (ms < 0) {
          return "incomplete"
        }
        total_seconds = int(ms / 1000)
        rem_ms = ms % 1000
        minutes = int(total_seconds / 60)
        hours = int(minutes / 60)
        seconds = total_seconds % 60
        minutes = minutes % 60
        if (hours > 0) {
          return sprintf("%02d:%02d:%02d.%03d", hours, minutes, seconds, rem_ms)
        }
        return sprintf("%02d:%02d.%03d", minutes, seconds, rem_ms)
      }
      BEGIN {
        print "# tests/static.sh timing"
        printf "%-14s %-14s %s\n", "wall", "cumulative", "label"
      }
      {
        if (NR == 1) {
          first = $4
        }
        cumulative = $1 - first
        printf "%-14s %-14s %s\n", fmt($3), fmt(cumulative), $2
        if ($3 >= 0) {
          last_complete = $1
        }
      }
      END {
        print ""
        if (last_complete > 0) {
          printf "TOTAL %s\n", fmt(last_complete - first)
        } else {
          print "TOTAL incomplete"
        }
      }
    ' > "$report_log"
}

rm -f -- "$RAW_LOG" "$RAW_LOG.lock" "$REPORT_LOG"
set +e
env NL_STATIC_TIMING_LOG="$RAW_LOG" ROOT="$ROOT" bash "$ROOT/tests/static.sh" "$@"
rc=$?
set -e
render_report "$RAW_LOG" "$REPORT_LOG"
cat "$REPORT_LOG"
exit "$rc"
