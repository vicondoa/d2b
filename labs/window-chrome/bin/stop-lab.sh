#!/usr/bin/env bash
# Stop any running chrome-lab proxy started by bin/d2b-chrome-lab.
#
# Capture runs restart the proxy between states, so this needs to be a single
# idempotent step rather than a copy-pasted pgrep/kill pair.
set -uo pipefail

pattern="${1:-listen /run/user/1000/chromelab}"
mapfile -t pids < <(pgrep -f "$pattern" || true)

if [ "${#pids[@]}" -eq 0 ]; then
  exit 0
fi

for pid in "${pids[@]}"; do
  [ -n "$pid" ] || continue
  kill "$pid" 2>/dev/null || true
done

# Give the proxy a moment to drop its socket before the next one binds.
for _ in $(seq 1 20); do
  pgrep -f "$pattern" >/dev/null 2>&1 || exit 0
  sleep 0.2
done

# Still alive: escalate, otherwise the next run inherits a stale listener.
mapfile -t pids < <(pgrep -f "$pattern" || true)
for pid in "${pids[@]}"; do
  [ -n "$pid" ] || continue
  kill -9 "$pid" 2>/dev/null || true
done
sleep 0.4
