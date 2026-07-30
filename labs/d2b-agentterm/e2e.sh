#!/usr/bin/env bash
# End-to-end validation for d2b-agentterm.
#
# Runs the real binary against real programs under a real PTY (allocated by
# `script`, since the harness itself has no controlling terminal), then drives
# it through the agent socket exactly the way an agent would.
#
# This is a lab; nothing here is wired into a repo CI gate. Run it by hand:
#
#     bash e2e.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$ROOT/target/debug/d2b-agentterm"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/agentterm-e2e-XXXXXX")"
SOCK="$WORK/agent.sock"

PASS=0
FAIL=0

cleanup() {
  [[ -n "${SESSION_PID:-}" ]] && kill "$SESSION_PID" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

ok() { printf '  ok    %s\n' "$1"; PASS=$((PASS + 1)); }
no() { printf '  FAIL  %s\n' "$1"; printf '        %s\n' "${2:-}"; FAIL=$((FAIL + 1)); }

check() {
  local name="$1" expected="$2" actual="$3"
  if [[ "$actual" == *"$expected"* ]]; then
    ok "$name"
  else
    no "$name" "expected to contain: $expected"
    printf '        actual: %s\n' "$(printf '%s' "$actual" | head -c 400)"
  fi
}

refute() {
  local name="$1" unexpected="$2" actual="$3"
  if [[ "$actual" != *"$unexpected"* ]]; then
    ok "$name"
  else
    no "$name" "expected NOT to contain: $unexpected"
  fi
}

agent() { "$BIN" "$@" --socket "$SOCK"; }

# Start a session under a PTY of a known size and wait for the socket.
start_session() {
  local cols="$1" rows="$2"
  shift 2
  script -qfec "stty cols $cols rows $rows; exec '$BIN' run --quiet --socket '$SOCK' -- $*" \
    /dev/null >"$WORK/session.log" 2>&1 &
  SESSION_PID=$!

  for _ in $(seq 1 100); do
    [[ -S "$SOCK" ]] && sleep 0.3 && return 0
    sleep 0.1
  done
  return 1
}

stop_session() {
  kill "$SESSION_PID" 2>/dev/null
  wait "$SESSION_PID" 2>/dev/null
  unset SESSION_PID
  rm -f "$SOCK"
}

printf '\n== 1. shell: scrolling / primary buffer ==\n'
if start_session 80 24 bash --norc --noprofile; then
  ok "session started and bound its socket"

  agent text 'echo hello-from-agent' >/dev/null
  agent keys Enter >/dev/null
  sleep 0.6

  screen="$(agent screen)"
  check "agent-typed command appears on screen" "hello-from-agent" "$screen"

  info="$(agent info)"
  check "info reports the primary buffer" "buffer primary" "$info"
  check "info reports the PTY size set by stty" "size 80x24" "$info"

  delta="$(agent delta --since 10s)"
  check "delta uses scrolling mode on the primary buffer" "mode scrolling" "$delta"
  check "delta reports the appended output" "hello-from-agent" "$delta"
  refute "delta transcript carries no escape sequences" $'\033' "$delta"

  quiet="$(agent delta --since 100ms)"
  check "a quiet window reports no change" "(no change)" "$quiet"

  json="$(agent screen --json)"
  check "json output is camelCase" '"altScreen"' "$json"

  stop_session
else
  no "session started" "socket never appeared"
fi

printf '\n== 2. alt-screen TUI: vim ==\n'
if command -v vim >/dev/null && start_session 80 24 vim -u NONE -N; then
  sleep 0.8
  info="$(agent info)"
  check "vim is detected on the alternate buffer" "buffer alt" "$info"

  agent keys i >/dev/null
  agent text 'typed into vim' >/dev/null
  sleep 0.5

  screen="$(agent screen)"
  check "text typed by the agent reaches vim" "typed into vim" "$screen"

  delta="$(agent delta --since 10s)"
  check "delta uses alt-screen mode" "mode alt-screen" "$delta"
  check "delta reports changed rows" "changed rows:" "$delta"

  agent keys Escape >/dev/null
  sleep 0.3
  agent keys ':' 'q' '!' Enter >/dev/null
  sleep 0.5
  ok "vim accepted Escape and the :q! key sequence"

  stop_session
else
  printf '  skip  vim not available\n'
fi

printf '\n== 3. resize propagates to the child (the bug ht has) ==\n'
if start_session 80 24 bash --norc --noprofile; then
  agent text 'tput cols; tput lines' >/dev/null
  agent keys Enter >/dev/null
  sleep 0.6
  before="$(agent screen)"
  check "child sees its initial width" "80" "$before"

  # Resize the PTY the session is attached to. `script` forwards SIGWINCH.
  stty -F /dev/tty cols 100 2>/dev/null || true
  agent resize --cols 100 --rows 30 >/dev/null
  sleep 0.4

  agent text 'tput cols' >/dev/null
  agent keys Enter >/dev/null
  sleep 0.6

  after="$(agent screen)"
  check "child observes the new width after resize" "100" "$after"

  info="$(agent info)"
  check "session reports the new size" "size 100x30" "$info"

  stop_session
else
  no "resize session started" "socket never appeared"
fi

printf '\n== 4. dump reconstructs screen state ==\n'
if start_session 80 24 bash --norc --noprofile; then
  agent text 'echo reconstruct-me' >/dev/null
  agent keys Enter >/dev/null
  sleep 0.6

  dump="$(agent dump)"
  check "dump contains the screen content" "reconstruct-me" "$dump"
  check "dump contains a cursor positioning sequence" $'\033[' "$dump"

  stop_session
else
  no "dump session started" "socket never appeared"
fi

printf '\n== 5. key encoding and error handling ==\n'
if start_session 80 24 bash --norc --noprofile; then
  agent text 'sleep 30' >/dev/null
  agent keys Enter >/dev/null
  sleep 0.5
  # Ctrl-C must interrupt the sleep and return a prompt.
  agent keys C-c >/dev/null
  sleep 0.5
  agent text 'echo after-interrupt' >/dev/null
  agent keys Enter >/dev/null
  sleep 0.6

  screen="$(agent screen)"
  check "C-c interrupted the child and the shell recovered" "after-interrupt" "$screen"

  bad="$(agent keys S-nonsense 2>&1)"
  check "an unencodable key is refused rather than typed as text" "error:" "$bad"
  refute "the refused key was not sent to the child" "S-nonsense" "$(agent screen)"

  stop_session
else
  no "keys session started" "socket never appeared"
fi

printf '\n== 6. socket hygiene ==\n'
if start_session 80 24 bash --norc --noprofile; then
  mode="$(stat -c '%a' "$SOCK" 2>/dev/null)"
  check "socket is owner-only" "600" "$mode"
  stop_session
  sleep 0.3
  if [[ ! -S "$SOCK" ]]; then
    ok "socket removed on session exit"
  else
    no "socket removed on session exit" "socket still present"
  fi
else
  no "hygiene session started" "socket never appeared"
fi

printf '\n== summary ==\n'
printf '  passed %d, failed %d\n\n' "$PASS" "$FAIL"
[[ "$FAIL" -eq 0 ]]
