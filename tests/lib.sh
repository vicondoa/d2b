# shellcheck shell=bash
# Shared helpers for the nixling test suite.
#
# Each helper is small, dependency-free, and assumes:
#   - We are running on a host with nixling installed and the
#     framework activated (i.e. `nixos-rebuild switch` has happened).
#   - sudo -A works without prompting (the invoking user is in
#     `wheel` and an askPass helper is configured) for tests that
#     touch root-owned state.
#   - `nixling` is on PATH (it's in system.environment, installed by the
#     framework's cli.nix).
#   - jq, ip, ssh are installed (nixpkgs default).
#
# Configurable via env:
#   FLAKE — consumer flake root (default: derived from this lib's
#           location, i.e. the repo containing tests/).
#   NL_OPERATOR_SSH_KEY — host operator's SSH private key for the
#           net-VM root login (default: $HOME/.ssh/id_ed25519).
#
# All output goes to stderr so test functions can `echo` their actual
# return value to stdout if they're producing data.

set -u

NL_LOG=${NL_LOG:-/tmp/nixling-test.log}
# Derive FLAKE from lib.sh's own location (tests/lib.sh → ../).
# Override with FLAKE=/path/to/clone when running against an alien tree.
_LIB_HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
FLAKE=${FLAKE:-$(dirname "$_LIB_HERE")}
# shellcheck disable=SC2034  # STATE_ROOT used by scripts that source this lib
STATE_ROOT=/var/lib/nixling/vms

# ---------- logging ----------

log() { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" | tee -a "$NL_LOG" >&2; }
ok()  { log "  PASS: $*"; }
fail() {
  log "  FAIL: $*"
  return 1
}

# ---------- assertions ----------

assert_eq() {
  local actual="$1" expected="$2" msg="${3:-}"
  if [ "$actual" = "$expected" ]; then
    ok "${msg:-assert_eq} ('$actual')"
  else
    fail "${msg:-assert_eq}: got '$actual', expected '$expected'"
  fi
}

assert_lt() {
  local actual="$1" threshold="$2" msg="${3:-}"
  if [ "$actual" -lt "$threshold" ]; then
    ok "${msg:-assert_lt} ($actual < $threshold)"
  else
    fail "${msg:-assert_lt}: $actual not < $threshold"
  fi
}

assert_ge() {
  local actual="$1" threshold="$2" msg="${3:-}"
  if [ "$actual" -ge "$threshold" ]; then
    ok "${msg:-assert_ge} ($actual >= $threshold)"
  else
    fail "${msg:-assert_ge}: $actual not >= $threshold"
  fi
}

assert_file_exists() {
  local p="$1"
  if [ -e "$p" ]; then
    ok "exists: $p"
  else
    fail "missing file: $p"
  fi
}

assert_contains() {
  local haystack="$1" needle="$2" msg="${3:-}"
  case "$haystack" in
    *"$needle"*) ok "${msg:-assert_contains} ('...$needle...')" ;;
    *)           fail "${msg:-assert_contains}: '$needle' not in output" ;;
  esac
}

assert_not_contains() {
  local haystack="$1" needle="$2" msg="${3:-}"
  case "$haystack" in
    *"$needle"*) fail "${msg:-assert_not_contains}: '$needle' WAS in output" ;;
    *)           ok "${msg:-assert_not_contains} ('$needle' absent)" ;;
  esac
}

# ---------- host helpers ----------

host_run() {
  log "  \$ $*"
  "$@"
}

vm_running() {
  # Mirrors cli.nix:vm_pids. Returns 0 if any cloud-hypervisor/qemu
  # process is associated with the named VM via its supervisord
  # cmdline or socket path.
  local vm="$1"
  systemctl is-active --quiet "microvm@${vm}.service" 2>/dev/null && return 0
  pgrep -f "microvm@${vm}\\b|nixos-system-${vm}-" >/dev/null 2>&1
}

# Read the on-disk manifest baked into the nixling derivation. Avoids
# duplicating SSH credential discovery across tests.
vm_ssh_user() {
  jq -r --arg v "$1" '.[$v].sshUser // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null
}
vm_ssh_key() {
  jq -r --arg v "$1" '.[$v].sshKeyPath // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null
}
vm_ssh_ip() {
  jq -r --arg v "$1" '.[$v].staticIp // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null
}

ssh_vm() {
  local vm="$1"; shift
  local user key ip
  user=$(vm_ssh_user "$vm")
  key=$(vm_ssh_key  "$vm")
  ip=$(vm_ssh_ip    "$vm")
  if [ -z "$user" ] || [ -z "$key" ] || [ -z "$ip" ]; then
    fail "ssh_vm: $vm missing ssh.user/ssh.keyPath/staticIp in manifest"
    return 1
  fi
  local kh=/var/lib/nixling/known_hosts.nixling
  ssh -o StrictHostKeyChecking=yes \
      -o UserKnownHostsFile="$kh" \
      -o ConnectTimeout=10 \
      -i "$key" "$user@$ip" "$@"
  local rc=$?
  # security-r8-audio-7: VMs frequently rotate host keys across
  # nixos-rebuilds (the SSH host-key on disk is regenerated in the
  # microvm root). When that happens, ssh fails with 255 + "HOST
  # IDENTIFICATION HAS CHANGED". The L1 known_hosts-refresh service
  # refuses to overwrite a pinned key (security-r7) — operators are
  # expected to rotate manually. For the AUTOMATED TEST suite, we
  # accept that rotation is the norm: remove the stale pin and
  # re-pin via the refresh service, then retry once. This keeps the
  # test resilient to legitimate VM rebuilds without lowering the
  # interactive-shell security posture.
  if [ "$rc" -eq 255 ] && [ -w "$kh" ] || sudo -n -A -- true 2>/dev/null; then
    local ssh_err
    ssh_err=$(ssh -o StrictHostKeyChecking=yes \
                  -o UserKnownHostsFile="$kh" \
                  -o ConnectTimeout=2 -o BatchMode=yes \
                  -i "$key" "$user@$ip" : 2>&1) || true
    if printf '%s' "$ssh_err" | grep -q "HOST IDENTIFICATION HAS CHANGED"; then
      sudo -A ssh-keygen -R "$ip" -f "$kh" >/dev/null 2>&1 || true
      sudo -A systemctl reset-failed "nixling-known-hosts-refresh@${vm}.service" >/dev/null 2>&1 || true
      sudo -A systemctl start "nixling-known-hosts-refresh@${vm}.service" >/dev/null 2>&1 || true
      sleep 1
      ssh -o StrictHostKeyChecking=yes \
          -o UserKnownHostsFile="$kh" \
          -o ConnectTimeout=10 \
          -i "$key" "$user@$ip" "$@"
      return $?
    fi
  fi
  return $rc
}

# ssh_net_vm: reach a net VM (which has no admin user/key in the
# manifest) via root + the host operator's id_ed25519 key, routed over
# the net VM's *uplink* interface IP. The net VM's entry in the
# manifest stores the uplink IP in `.staticIp` (it has no workload-LAN
# address; the workload-side address is the LAN gateway baked into
# net.nix). The net VM's root account accepts the host operator's
# id_ed25519 from the host per net.nix.
#
# Return codes (callers use these to distinguish "infra missing" from
# "SSH failed" so they can SKIP cleanly rather than mis-FAIL):
#   2 — net VM not in manifest / no staticIp
#   3 — operator host key not on disk (net VM build evaluated
#       `lib.optionals (builtins.pathExists ...)` to []; nothing to
#       authenticate with)
#   * — whatever ssh itself returned (255 transport, command exit)
ssh_net_vm() {
  local vm="$1"; shift
  local ip key=${NL_OPERATOR_SSH_KEY:-$HOME/.ssh/id_ed25519}
  ip=$(jq -r --arg v "$vm" \
    '.[$v] | select(.isNetVm == true) | .staticIp // empty' \
    /run/current-system/sw/share/nixling/vms.json 2>/dev/null)
  if [ -z "$ip" ]; then
    return 2
  fi
  if [ ! -r "$key" ]; then
    return 3
  fi
  ssh -i "$key" \
      -o BatchMode=yes \
      -o ConnectTimeout=5 \
      -o StrictHostKeyChecking=no \
      -o UserKnownHostsFile=/dev/null \
      "root@$ip" "$@"
}

# DEPRECATED back-compat alias. Older test scripts and ad-hoc tooling
# call `ssh_router <vm>`; new code should call `ssh_net_vm`. The W2
# rename of the per-env auto-VM (\`<env>-router\` → \`sys-<env>-net\`)
# and of the manifest field (\`isRouter\` → \`isNetVm\`) means the
# old name is misleading, but a hard rename in this commit would
# break any caller carried over from the pre-W2 era. Remove after
# all in-tree call sites have switched (Phase 7a).
ssh_router() { ssh_net_vm "$@"; }

# ---------- cleanup ----------

NL_CLEANUPS=()
add_cleanup() { NL_CLEANUPS+=( "$*" ); }
run_cleanups() {
  local i
  for ((i=${#NL_CLEANUPS[@]}-1; i>=0; i--)); do
    log "cleanup: ${NL_CLEANUPS[$i]}"
    eval "${NL_CLEANUPS[$i]}" || log "  (cleanup failed, continuing)"
  done
}
trap run_cleanups EXIT
