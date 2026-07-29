#!/usr/bin/env bash
# demo/input.sh — prove keyboard and pointer actually reach the application,
# including after a detach/attach cycle.
#
# Input is injected into the session host's own stream rather than through a
# virtual-input tool, because tools like wtype install their own keymap and so
# send keycodes that mean nothing under ours — which looks exactly like a broken
# input path when it isn't.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$HERE/target/release/d2b-wlattach"
[ -x "$BIN" ] || BIN="$HERE/target/debug/d2b-wlattach"
SESSION="input-$$"
RT="${XDG_RUNTIME_DIR:-/tmp}/d2b-wlattach/$SESSION"
KEYOUT="/tmp/wlattach-key-$$"
LOG="/tmp/wlattach-input-host-$$.log"

pass=0; fail=0
check() {
  if [ "$2" = "$3" ]; then
    printf '  \033[32mPASS\033[0m  %s (%s)\n' "$1" "$3"; pass=$((pass + 1))
  else
    printf '  \033[31mFAIL\033[0m  %s: expected %s, got %s\n' "$1" "$2" "$3"; fail=$((fail + 1))
  fi
}
cleanup() {
  "$BIN" kill -s "$SESSION" >/dev/null 2>&1
  sleep 0.4
  for p in $(pgrep -f "d2b-wlattach (serve|present) --session $SESSION" 2>/dev/null); do
    kill -9 "$p" 2>/dev/null
  done
  rm -rf "$RT" "$KEYOUT"
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "build first: cargo build"; exit 1; }
rm -rf "$RT" "$KEYOUT"

# A terminal in raw mode with mouse reporting on. One keystroke, then one click,
# each land as bytes on the pty.
WAYLAND_DEBUG=1 "$BIN" serve --session "$SESSION" -- \
  foot sh -c "printf '\033[?1000h'; stty -icanon -echo min 1 time 0; dd bs=1 count=1 of=$KEYOUT 2>/dev/null; dd bs=1 count=6 of=$KEYOUT.mouse 2>/dev/null; sleep 30" \
  >"$LOG" 2>&1 &

for _ in $(seq 60); do [ -S "$RT/ctl.sock" ] && break; sleep 0.2; done
[ -S "$RT/ctl.sock" ] || { echo "host did not start"; exit 1; }
sleep 2
"$BIN" attach -s "$SESSION" >/dev/null; sleep 3

echo "== keyboard =="
"$BIN" inject --session "$SESSION" --key 22   # KEY_U
sleep 1.5
check "application received a keystroke" "u" "$(cat "$KEYOUT" 2>/dev/null)"

echo
echo "== pointer =="
"$BIN" inject --session "$SESSION" --click 100 100
sleep 1.5
check "app saw wl_pointer.enter"  1 "$(grep -c 'wl_pointer@[0-9]*\.enter'  "$LOG")"
check "app saw wl_pointer.motion" 1 "$(grep -c 'wl_pointer@[0-9]*\.motion' "$LOG")"
check "app saw wl_pointer.button" 2 "$(grep -c 'wl_pointer@[0-9]*\.button' "$LOG")"
# foot turns the click into an X10 mouse report: ESC [ M ...
check "terminal emitted a mouse report" "yes" \
  "$(grep -q $'\033\[M' "$KEYOUT.mouse" 2>/dev/null && echo yes || echo no)"

echo
echo "== input still works after detach/attach =="
"$BIN" detach -s "$SESSION" >/dev/null; sleep 1.5
"$BIN" attach -s "$SESSION" >/dev/null; sleep 3
rm -f "$KEYOUT.after"
# The shell above has moved on; start a fresh reader through a new key.
before=$(grep -c 'wl_keyboard@[0-9]*\.key' "$LOG")
"$BIN" inject --session "$SESSION" --key 23
sleep 1.5
after=$(grep -c 'wl_keyboard@[0-9]*\.key' "$LOG")
check "keys still delivered on the new generation" "yes" \
  "$([ "$after" -gt "$before" ] && echo yes || echo no)"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
rm -f "$KEYOUT.mouse"
[ "$fail" -eq 0 ]
