#!/usr/bin/env bash
# demo/demo.sh - the scripted attach/detach demonstration.
#
# Proves the one claim the prototype exists to make: a real Wayland application
# survives the death of the process that is showing its window, and gets its
# window back - with the content that was on screen - on a brand-new compositor
# connection.
#
# Usage:  bash demo/demo.sh [app...]        (default: foot)
set -uo pipefail

APP=("${@:-foot}")
SESSION="demo-$$"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$HERE/target/release/d2b-wlattach"
[ -x "$BIN" ] || BIN="$HERE/target/debug/d2b-wlattach"
RT="${XDG_RUNTIME_DIR:-/tmp}/d2b-wlattach/$SESSION"

pass=0
fail=0
check() { # check <description> <expected> <actual>
  if [ "$2" = "$3" ]; then
    printf '  \033[32mPASS\033[0m  %s (%s)\n' "$1" "$3"
    pass=$((pass + 1))
  else
    printf '  \033[31mFAIL\033[0m  %s: expected %s, got %s\n' "$1" "$2" "$3"
    fail=$((fail + 1))
  fi
}

# Count windows the given pid owns in niri. Requires niri IPC; if unavailable we
# skip the window-visibility checks rather than reporting a false pass.
have_niri=0
command -v niri >/dev/null 2>&1 && niri msg --json windows >/dev/null 2>&1 && have_niri=1
windows_of() {
  [ "$have_niri" = 1 ] || { echo skip; return; }
  niri msg --json windows 2>/dev/null | grep -c "\"pid\":$1" || true
}

alive() { [ -d "/proc/$1" ] && echo yes || echo no; }
frontend_pid() { pgrep -f "d2b-wlattach present --session $SESSION" | head -1; }

cleanup() {
  "$BIN" kill -s "$SESSION" >/dev/null 2>&1
  sleep 0.5
  for p in $(pgrep -f "d2b-wlattach (serve|present) --session $SESSION" 2>/dev/null); do
    kill -9 "$p" 2>/dev/null
  done
  rm -rf "$RT"
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "build first: cargo build"; exit 1; }

echo "== d2b-wlattach demo: ${APP[*]} =="
rm -rf "$RT"

"$BIN" serve --session "$SESSION" -- "${APP[@]}" >/tmp/wlattach-demo-host.log 2>&1 &
HOST=$!

for _ in $(seq 60); do [ -S "$RT/ctl.sock" ] && break; sleep 0.2; done
[ -S "$RT/ctl.sock" ] || { echo "host did not start; see /tmp/wlattach-demo-host.log"; exit 1; }

# The application is a child of the session host, not of the frontend.
sleep 2
APPPID=$(pgrep -P "$HOST" | head -1)
echo "  session host  pid=$HOST"
echo "  application   pid=$APPPID"

echo
echo "-- 1. attach: the window should appear"
"$BIN" attach -s "$SESSION" >/dev/null
sleep 4
F1=$(frontend_pid)
check "window is on screen" 1 "$(windows_of "$F1")"
check "application running" yes "$(alive "$APPPID")"

echo
echo "-- 2. detach: window goes away, application must NOT"
"$BIN" detach -s "$SESSION" >/dev/null
sleep 2
check "window is gone" 0 "$(windows_of "$F1")"
check "application STILL running" yes "$(alive "$APPPID")"
check "frontend process gone" no "$(alive "$F1")"
check "session retains content" 1 \
  "$("$BIN" status -s "$SESSION" --json | grep -o '"retained":[0-9]*' | cut -d: -f2)"

echo
echo "-- 3. attach again: a NEW frontend, same application"
"$BIN" attach -s "$SESSION" >/dev/null
sleep 4
F2=$(frontend_pid)
check "window is back" 1 "$(windows_of "$F2")"
check "frontend is a different process" different \
  "$([ "$F1" != "$F2" ] && echo different || echo same)"
check "application never restarted" yes "$(alive "$APPPID")"
check "same application pid" "$APPPID" "$(pgrep -P "$HOST" | head -1)"

echo
echo "-- 4. repeat 5 more cycles"
for i in $(seq 5); do
  "$BIN" detach -s "$SESSION" >/dev/null
  sleep 1
  "$BIN" attach -s "$SESSION" >/dev/null
  sleep 2
done
check "application survived 6 detach/attach cycles" yes "$(alive "$APPPID")"
check "still exactly one window" 1 "$(windows_of "$(frontend_pid)")"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
