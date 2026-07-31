#!/usr/bin/env bash
# Run a command in the lab guest over the passt-forwarded SSH port.
#
# The guest is reachable only on host loopback (run-lab-vm.sh passes
# `-t 127.0.0.1/$SSH_PORT:22` to passt), so this is not a network-exposed
# surface. Password auth against the lab account is deliberate: the password is
# already a literal in guest/configuration.nix and therefore in the Nix store,
# so a key pair would add key management without adding confidentiality.
#
# Usage:
#   lab-ssh.sh                       # interactive shell
#   lab-ssh.sh vulkaninfo            # run a command, stream output
#   lab-ssh.sh --wait                # block until sshd answers, then exit
#   lab-ssh.sh --stdin 'python3 -'   # run a command with this script's stdin
#                                    # piped to it, so a probe can be iterated
#                                    # without rebuilding the guest image
set -euo pipefail

SSH_PORT="${VENUS_LAB_SSH_PORT:-2222}"
SSH_USER="${VENUS_LAB_SSH_USER:-lab}"
SSH_PASS="${VENUS_LAB_SSH_PASS:-lab}"
WAIT_SECS="${VENUS_LAB_SSH_WAIT:-180}"

# A lab VM is reinstalled constantly and always answers on the same loopback
# port, so a known-hosts entry would be wrong more often than right and would
# turn every rebuild into a manual key-removal step.
SSH_OPTS=(
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o GlobalKnownHostsFile=/dev/null
  -o LogLevel=ERROR
  -o ConnectTimeout=5
  -p "$SSH_PORT"
)

log() { printf '[lab-ssh] %s\n' "$*" >&2; }

wait_for_ssh() {
  local deadline=$(( SECONDS + WAIT_SECS ))
  log "waiting up to ${WAIT_SECS}s for guest sshd on 127.0.0.1:$SSH_PORT"
  while (( SECONDS < deadline )); do
    if sshpass -p "$SSH_PASS" ssh "${SSH_OPTS[@]}" \
         "$SSH_USER@127.0.0.1" true 2>/dev/null; then
      log "guest is up"
      return 0
    fi
    sleep 2
  done
  log "ERROR: guest sshd did not answer within ${WAIT_SECS}s"
  return 1
}

if [ "${1:-}" = "--wait" ]; then
  wait_for_ssh
  exit $?
fi

# --stdin: forward this script's stdin to the remote command. Used to pipe a
# probe script in rather than baking it into the guest image, so iterating on
# the probe costs an SSH round trip instead of an image rebuild and reboot.
STDIN_MODE=0
if [ "${1:-}" = "--stdin" ]; then
  STDIN_MODE=1
  shift
fi

# Always wait first. Calling this immediately after starting the VM is the
# normal case, and failing with "connection refused" because the guest is still
# booting is a misleading error that looks like a networking bug.
#
# The wait probe must not consume stdin, or the payload would be eaten before
# the real command runs.
wait_for_ssh < /dev/null || exit 1

if [ "$STDIN_MODE" = "1" ]; then
  exec sshpass -p "$SSH_PASS" ssh "${SSH_OPTS[@]}" "$SSH_USER@127.0.0.1" -- "$@"
fi

if [ $# -eq 0 ]; then
  exec sshpass -p "$SSH_PASS" ssh "${SSH_OPTS[@]}" -t "$SSH_USER@127.0.0.1"
fi

exec sshpass -p "$SSH_PASS" ssh "${SSH_OPTS[@]}" "$SSH_USER@127.0.0.1" -- "$@"
