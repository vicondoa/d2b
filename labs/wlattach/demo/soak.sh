#!/usr/bin/env bash
# demo/soak.sh - leak check: FD count and RSS across many attach/detach cycles.
set -uo pipefail
N="${1:-25}"
APP=("${@:2}"); [ ${#APP[@]} -eq 0 ] && APP=(foot)
SESSION="soak-$$"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$HERE/target/release/d2b-wlattach"
[ -x "$BIN" ] || BIN="$HERE/target/debug/d2b-wlattach"
RT="${XDG_RUNTIME_DIR:-/tmp}/d2b-wlattach/$SESSION"

cleanup() {
  "$BIN" kill -s "$SESSION" >/dev/null 2>&1
  sleep 0.5
  for p in $(pgrep -f "d2b-wlattach (serve|present) --session $SESSION" 2>/dev/null); do
    kill -9 "$p" 2>/dev/null
  done
  rm -rf "$RT"
}
trap cleanup EXIT

rm -rf "$RT"
"$BIN" serve --session "$SESSION" -- "${APP[@]}" >/tmp/wlattach-soak-host.log 2>&1 &
HOST=$!
for _ in $(seq 60); do [ -S "$RT/ctl.sock" ] && break; sleep 0.2; done
[ -S "$RT/ctl.sock" ] || { echo "host did not start"; exit 1; }
sleep 2
APPPID=$(pgrep -P "$HOST" | head -1)

fds()  { ls "/proc/$1/fd" 2>/dev/null | wc -l; }
rss()  { awk '/VmRSS/{print $2}' "/proc/$1/status" 2>/dev/null; }

"$BIN" attach -s "$SESSION" >/dev/null; sleep 3
base_fd=$(fds "$HOST"); base_rss=$(rss "$HOST")
printf 'baseline   host fd=%s rss=%skB   app=%s\n' "$base_fd" "$base_rss" "$APPPID"

for i in $(seq "$N"); do
  "$BIN" detach -s "$SESSION" >/dev/null; sleep 0.6
  "$BIN" attach -s "$SESSION" >/dev/null; sleep 1.2
  if [ $((i % 5)) -eq 0 ]; then
    printf 'cycle %-3s host fd=%s rss=%skB app_alive=%s\n' \
      "$i" "$(fds "$HOST")" "$(rss "$HOST")" \
      "$([ -d /proc/$APPPID ] && echo yes || echo NO)"
  fi
done

end_fd=$(fds "$HOST"); end_rss=$(rss "$HOST")
printf '\nfd    baseline=%s final=%s  delta=%s\n' "$base_fd" "$end_fd" "$((end_fd - base_fd))"
printf 'rss   baseline=%skB final=%skB  delta=%skB\n' "$base_rss" "$end_rss" "$((end_rss - base_rss))"
printf 'app   pid=%s alive=%s (unchanged throughout)\n' "$APPPID" "$([ -d /proc/$APPPID ] && echo yes || echo NO)"

rc=0
[ "$((end_fd - base_fd))" -le 2 ] || { echo "FAIL: fd leak"; rc=1; }
[ "$((end_rss - base_rss))" -le 32768 ] || { echo "FAIL: rss growth > 32MiB"; rc=1; }
[ -d "/proc/$APPPID" ] || { echo "FAIL: application died"; rc=1; }
[ $rc -eq 0 ] && echo "SOAK OK"
exit $rc
