#!/usr/bin/env bash
# demo/visual.sh - verify what is actually ON SCREEN, not just what crossed the
# wire.
#
# Every other test here asserts protocol events or process liveness. Those can
# all pass while the window shows a frozen first frame, which is exactly the bug
# this test exists to catch: reusing a wl_buffer without re-attaching it let the
# compositor keep its uploaded texture forever.
#
# Needs: niri (for screenshot-window) and a GTK app.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$HERE/target/release/d2b-wlattach"
[ -x "$BIN" ] || BIN="$HERE/target/debug/d2b-wlattach"
APP="${1:-gtk4-demo}"
SESSION="visual-$$"
RT="${XDG_RUNTIME_DIR:-/tmp}/d2b-wlattach/$SESSION"
WORK="$(mktemp -d)"

pass=0; fail=0
check() {
  if [ "$2" = "$3" ]; then
    printf '  \033[32mPASS\033[0m  %s\n' "$1"; pass=$((pass + 1))
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
  rm -rf "$RT" "$WORK"
}
trap cleanup EXIT

command -v niri >/dev/null 2>&1 || { echo "SKIP: needs niri"; exit 0; }
command -v "$APP" >/dev/null 2>&1 || { echo "SKIP: $APP not on PATH"; exit 0; }

# Screenshot our window into $WORK, by niri window id.
shoot() {
  local out="$1"
  niri msg action screenshot-window --id "$WID" -d true >/dev/null 2>&1
  sleep 1
  local latest
  latest=$(ls -t "$HOME"/Pictures/Screenshots/* 2>/dev/null | head -1)
  [ -n "$latest" ] && mv "$latest" "$out" 2>/dev/null
}
hash_of() { sha256sum "$1" 2>/dev/null | cut -c1-32; }

rm -rf "$RT"
"$BIN" serve --session "$SESSION" -- "$APP" >/tmp/wlattach-visual-host.log 2>&1 &
for _ in $(seq 60); do [ -S "$RT/ctl.sock" ] && break; sleep 0.2; done
[ -S "$RT/ctl.sock" ] || { echo "host did not start"; exit 1; }
sleep 3
"$BIN" attach -s "$SESSION" >/dev/null; sleep 5

FPID=$(pgrep -f "d2b-wlattach present --session $SESSION" | head -1)
WID=$(niri msg --json windows 2>/dev/null \
  | grep -o "{[^{]*\"pid\":$FPID[^}]*}" | grep -o '"id":[0-9]*' | head -1 | cut -d: -f2)
[ -n "$WID" ] || { echo "our window is not on screen"; exit 1; }
PX=$(ls "$RT"/px-*.raw 2>/dev/null | head -1)

echo "== the application redraws when clicked =="
a=$(hash_of "$PX")
"$BIN" inject --session "$SESSION" --click 200 300 >/dev/null 2>&1
sleep 1.5
b=$(hash_of "$PX")
check "rendered content changed" "changed" \
  "$([ "$a" != "$b" ] && echo changed || echo same)"

echo
echo "== that change reaches the screen =="
shoot "$WORK/s1.png"
"$BIN" inject --session "$SESSION" --click 260 360 >/dev/null 2>&1
sleep 1.5
shoot "$WORK/s2.png"
check "on-screen pixels changed" "changed" \
  "$([ "$(hash_of "$WORK/s1.png")" != "$(hash_of "$WORK/s2.png")" ] && echo changed || echo same)"

echo
echo "== the restored frame matches what was on screen before detach =="
# Let the application settle first: sampling mid-redraw would compare two
# different frames and tell us nothing.
settle() {
  local last="" now="" stable=0
  for _ in $(seq 40); do
    now=$(hash_of "$PX")
    if [ "$now" = "$last" ]; then
      stable=$((stable + 1))
      [ "$stable" -ge 3 ] && break
    else
      stable=0
    fi
    last="$now"
    sleep 0.25
  done
}
settle
before=$(hash_of "$PX")

"$BIN" detach -s "$SESSION" >/dev/null; sleep 2.5
# Frame callbacks are withheld while detached, so a settled application should
# not have drawn anything new.
check "content retained while detached" "$before" "$(hash_of "$PX")"

"$BIN" attach -s "$SESSION" >/dev/null; sleep 3
check "same frame restored on attach" "$before" "$(hash_of "$PX")"

echo
printf 'passed %d, failed %d\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
