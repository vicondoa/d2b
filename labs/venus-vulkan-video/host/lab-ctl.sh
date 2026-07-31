#!/usr/bin/env bash
# Start / stop / probe the lab VM without hand-rolled pkill.
#
# Why this exists: the host also runs the operator's real d2b microVMs, which
# are ALSO cloud-hypervisor processes. A `pkill -f cloud-hypervisor` would take
# those down with it. Every lab process instead lives under the per-run
# directory recorded here, so teardown can be scoped to this run and nothing
# else.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
export HERE   # available to anything this dispatches; not used directly here
STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab"
PIDFILE="$STATE_DIR/launcher.pid"
RUNROOT="${XDG_RUNTIME_DIR:-/tmp}/venus-lab"
LOGFILE="$STATE_DIR/launcher.log"

log() { printf '[lab-ctl] %s\n' "$*" >&2; }

usage() {
  cat >&2 <<'EOF'
usage: lab-ctl.sh {start|stop|reap|status|serial}

  start   launch the lab VM detached; records the launcher pid
  stop    terminate the recorded launcher, letting its own trap tear down
          passt/cage/crosvm/cloud-hypervisor and revoke the KVM grant
  status  report whether the recorded launcher is alive
  serial  tail the serial log

Environment:
  VENUS_LAB_FLAKE   flake ref to `nix run` (required for start)
EOF
  exit 2
}

alive() {
  [ -f "$PIDFILE" ] || return 1
  local pid; pid=$(cat "$PIDFILE" 2>/dev/null) || return 1
  [ -n "$pid" ] || return 1
  kill -0 "$pid" 2>/dev/null
}

# Every lab process carries the per-run directory in its argv, and that path
# contains "venus-lab". Reaping on THAT is what makes this safe: the operator's
# own d2b VMs are also cloud-hypervisor processes, and a match on the binary
# name would kill them too.
#
# This exists because killing the launcher is NOT sufficient. Its EXIT trap
# tears the children down only if the launcher is alive to run it; if crosvm
# crashes and takes the pipeline with it, or the launcher is killed hard, CH
# survives and spins against a dead vhost-user GPU backend at full CPU. That
# has now happened twice.
lab_pids() {
  # Two conditions, both required: the process must be one of the lab's own
  # binaries AND carry the per-run directory in its argv.
  #
  # The run dir alone is not enough. Any shell whose command line happens to
  # mention the path matches it -- including the very pipeline doing the
  # matching, which is how a self-match nearly got mistaken here for a d2b VM
  # collision. Requiring the binary name as well removes that whole class,
  # and the run dir still keeps it disjoint from the operator's d2b VMs, which
  # live under /var/lib/d2b and are also cloud-hypervisor processes.
  ps -eo pid=,comm=,args= 2>/dev/null | awk -v root="$RUNROOT" -v self="$$" '
    $1 == self { next }
    $2 !~ /^(cloud-hyperviso|crosvm|cage|passt|bwrap)/ { next }
    index($0, root) == 0 { next }
    { print $1 }
  ' || true
}

reap() {
  local what="$1" pids
  pids=$(lab_pids)
  [ -n "$pids" ] || return 0
  log "$what: reaping stray lab processes: $(echo "$pids" | tr '\n' ' ')"
  # shellcheck disable=SC2086
  kill -TERM $pids 2>/dev/null || true
  sleep 2
  pids=$(lab_pids)
  if [ -n "$pids" ]; then
    log "$what: forcing $(echo "$pids" | tr '\n' ' ')"
    # shellcheck disable=SC2086
    kill -KILL $pids 2>/dev/null || true
  fi
}

case "${1:-}" in
  start)
    if alive; then
      log "already running (pid $(cat "$PIDFILE"))"
      exit 0
    fi
    # A dead launcher with live children is exactly the state that accumulates
    # CPU-burning orphans, and it looks identical to "not running".
    reap "start"
    rm -f "$PIDFILE"
    [ -n "${VENUS_LAB_FLAKE:-}" ] || { log "set VENUS_LAB_FLAKE"; exit 2; }
    mkdir -p "$STATE_DIR"
    export VENUS_LAB_SERIAL_LOG="${VENUS_LAB_SERIAL_LOG:-$STATE_DIR/serial.log}"
    : > "$VENUS_LAB_SERIAL_LOG"
    log "starting; serial -> $VENUS_LAB_SERIAL_LOG"
    # setsid so the launcher owns its own process group and `stop` can signal
    # the whole group without reaching anything else on the host.
    setsid nohup nix run "$VENUS_LAB_FLAKE#lab-vm" \
      > "$LOGFILE" 2>&1 < /dev/null &
    echo $! > "$PIDFILE"
    log "launcher pid $(cat "$PIDFILE"); log -> $LOGFILE"
    ;;

  stop)
    if ! alive; then
      log "launcher not running; checking for orphans anyway"
      reap "stop"
      rm -f "$PIDFILE"
      exit 0
    fi
    pid=$(cat "$PIDFILE")
    log "stopping launcher pid $pid (its EXIT trap does the teardown)"
    # TERM the process GROUP, not the pid: `nix run` execs a wrapper which
    # execs the launcher, so the pid recorded here may not be the process
    # holding the trap. The group is the reliable handle, and it contains only
    # this run's children.
    kill -TERM -- "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    for _ in $(seq 1 30); do
      alive || break
      sleep 1
    done
    if alive; then
      log "did not exit after 30s; sending KILL to the group"
      kill -KILL -- "-$pid" 2>/dev/null || true
      sleep 2
    fi
    # The launcher is gone; verify its children are too rather than assuming
    # the trap did its job.
    reap "stop"
    rm -f "$PIDFILE"
    if [ -n "$(lab_pids)" ]; then
      log "WARNING: lab processes still present after reap:"
      ps -o pid,pcpu,args -p "$(lab_pids | tr '\n' ',' | sed 's/,$//')" >&2 || true
      exit 1
    fi
    log "stopped, no lab processes remain"
    ;;

  reap)
    reap "manual"
    rm -f "$PIDFILE"
    if [ -n "$(lab_pids)" ]; then
      log "still present:"; ps -o pid,pcpu,args -p "$(lab_pids | tr '\n' ',' | sed 's/,$//')" >&2 || true
      exit 1
    fi
    log "no lab processes remain"
    ;;

  status)
    if alive; then
      log "running (pid $(cat "$PIDFILE"))"
    else
      log "not running"
      exit 1
    fi
    ;;

  serial)
    exec tail -n "${2:-40}" "${VENUS_LAB_SERIAL_LOG:-$STATE_DIR/serial.log}"
    ;;

  *) usage ;;
esac
