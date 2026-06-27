#!/usr/bin/env bash
# Layer-2 optional test for live host datapath isolation.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

MANIFEST=/run/current-system/sw/share/d2b/vms.json

skip() { log "  SKIP: $*"; exit 0; }
fail_now() { fail "$*"; exit 1; }

[ -r "$MANIFEST" ] || skip "manifest missing; d2b not installed on this host"
command -v bridge >/dev/null 2>&1 || skip "bridge(8) not available"

bridge_tap_isolated() {
  local vm="$1" tap
  tap=$(jq -r --arg v "$vm" '.[$v].tap // empty' "$MANIFEST") || return 1
  [ -n "$tap" ] || return 1
  bridge link show dev "$tap" 2>/dev/null | grep -q 'isolated on'
}

probe_tcp_from_vm() {
  local src_vm="$1" dst_ip="$2" dst_port="${3:-22}"
  timeout 8 ssh_vm "$src_vm" "bash -lc '</dev/tcp/$dst_ip/$dst_port'" >/dev/null 2>&1
}

start_host_listener() {
  local bind_ip="$1" bind_port="$2"
  if command -v socat >/dev/null 2>&1; then
    socat "TCP-LISTEN:${bind_port},bind=${bind_ip},reuseaddr,fork" SYSTEM:'cat >/dev/null' >/dev/null 2>&1 &
  elif command -v nc >/dev/null 2>&1; then
    while true; do nc -l -s "$bind_ip" -p "$bind_port" >/dev/null 2>&1 || break; done &
  elif command -v ncat >/dev/null 2>&1; then
    ncat -lk "$bind_ip" "$bind_port" >/dev/null 2>&1 &
  else
    return 1
  fi
  echo $!
}

start_vm_listener() {
  local vm="$1" bind_port="$2"
  ssh_vm "$vm" "sh -c '
    if command -v socat >/dev/null 2>&1; then
      socat TCP-LISTEN:${bind_port},reuseaddr,fork SYSTEM:'\''cat >/dev/null'\'' >/dev/null 2>&1 &
    elif command -v nc >/dev/null 2>&1; then
      while true; do nc -l -p ${bind_port} >/dev/null 2>&1 || break; done &
    elif command -v ncat >/dev/null 2>&1; then
      ncat -lk ${bind_port} >/dev/null 2>&1 &
    else
      exit 77
    fi
    echo \$!
  '" 2>/dev/null
}

net_vm_ip_for_env() {
  local env="$1"
  jq -r --arg e "$env" '
    to_entries[]
    | select(.key | startswith("_") | not)
    | select(.value.isNetVm == true and .value.env == $e)
    | .value.staticIp
  ' "$MANIFEST" | head -1
}

gateway_ip_for_peer() {
  local peer_ip="$1"
  IFS=. read -r o1 o2 o3 _ <<<"$peer_ip"
  printf '%s.%s.%s.1\n' "$o1" "$o2" "$o3"
}

mapfile -t CANDIDATES < <(
  jq -r '
    to_entries[]
    | select(.key | startswith("_") | not)
    | select(.value.isNetVm != true)
    | select(.value.env != null and .value.staticIp != null)
    | .key
  ' "$MANIFEST"
)

READY=()
for vm in "${CANDIDATES[@]}"; do
  if vm_running "$vm" && [ -n "$(vm_ssh_user "$vm")" ] && [ -n "$(vm_ssh_ip "$vm")" ]; then
    if ssh_vm "$vm" true >/dev/null 2>&1; then
      READY+=("$vm")
    fi
  fi
done

[ "${#READY[@]}" -ge 2 ] || skip "need at least two running workload VMs with SSH access"

ran_any=0
same_env_allowed_done=0
cross_env_done=0
host_lan_done=0
probe_port=40223
allow_pair="${D2B_ALLOW_EASTWEST_PAIR:-}"
allow_a="${allow_pair%%:*}"
allow_b="${allow_pair#*:}"
host_lan_ip=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '/ src / { for (i = 1; i <= NF; i++) if ($i == "src") { print $(i+1); exit } }')
host_lan_gateway=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '/ via / { for (i = 1; i <= NF; i++) if ($i == "via") { print $(i+1); exit } }')
host_lan_port=""
if [ -n "$host_lan_ip" ]; then
  host_lan_port=40222
  host_listener_pid=$(start_host_listener "$host_lan_ip" "$host_lan_port" || true)
  if [ -n "${host_listener_pid:-}" ]; then
    add_cleanup "kill $host_listener_pid 2>/dev/null || true"
    sleep 1
  else
    host_lan_port=""
  fi
fi

for ((i=0; i<${#READY[@]}; i++)); do
  a=${READY[$i]}
  env_a=$(jq -r --arg v "$a" '.[$v].env' "$MANIFEST")
  for ((j=i+1; j<${#READY[@]}; j++)); do
    b=${READY[$j]}
    env_b=$(jq -r --arg v "$b" '.[$v].env' "$MANIFEST")
    ip_b=$(vm_ssh_ip "$b")

    if [ "$env_a" = "$env_b" ]; then
      listener_pid=$(start_vm_listener "$b" "$probe_port" || true)
      if [ -z "$listener_pid" ]; then
        skip "$b has no temporary listener tool (need socat/nc/ncat)"
      fi
      add_cleanup "ssh_vm '$b' 'kill $listener_pid 2>/dev/null || true' >/dev/null 2>&1 || true"
      sleep 1
      if bridge_tap_isolated "$a" && bridge_tap_isolated "$b"; then
        ran_any=1
        if probe_tcp_from_vm "$a" "$ip_b" "$probe_port"; then
          fail_now "$a unexpectedly reached same-env peer $b ($ip_b:$probe_port) while bridge taps are isolated"
        fi
        if ssh_vm "$a" 'sudo -n true' >/dev/null 2>&1; then
          gw_ip=$(gateway_ip_for_peer "$ip_b")
          ssh_vm "$a" "sudo -n sh -c 'set -eu; iface=\$(ip route get $ip_b | awk '\''/ dev / { for (i = 1; i <= NF; i++) if (\$i == \"dev\") { print \$(i+1); exit } }'\''); ip route replace $ip_b via $gw_ip dev \"\$iface\"'" >/dev/null 2>&1 || true
          add_cleanup "ssh_vm '$a' 'sudo -n ip route del $ip_b >/dev/null 2>&1 || true' >/dev/null 2>&1 || true"
          if probe_tcp_from_vm "$a" "$ip_b" "$probe_port"; then
            fail_now "$a reached same-env peer $b ($ip_b:$probe_port) after routing via the net VM gateway"
          fi
        fi
        ok "same-env east-west stays blocked when bridge taps are isolated ($a -> $b)"
      elif [ "$same_env_allowed_done" -eq 0 ]; then
        if [ -n "$allow_pair" ] && { [ "$a" = "$allow_a" ] && [ "$b" = "$allow_b" ] || [ "$a" = "$allow_b" ] && [ "$b" = "$allow_a" ]; }; then
          ran_any=1
          if [ -n "$listener_pid" ] && probe_tcp_from_vm "$a" "$ip_b" "$probe_port"; then
            ok "same-env east-west works for the explicitly allowed pair ($a -> $b:$probe_port)"
          else
            fail_now "$a could not reach explicitly allowed same-env peer $b ($ip_b:$probe_port)"
          fi
          same_env_allowed_done=1
        else
          fail_now "same-env pair $a/$b is non-isolated without being named in D2B_ALLOW_EASTWEST_PAIR"
        fi
      fi
    elif [ "$cross_env_done" -eq 0 ]; then
      ran_any=1
      peer_net_ip=$(net_vm_ip_for_env "$env_b")
      listener_pid=$(start_vm_listener "$b" "$probe_port" || true)
      if [ -z "$listener_pid" ]; then
        skip "$b has no temporary listener tool (need socat/nc/ncat)"
      fi
      add_cleanup "ssh_vm '$b' 'kill $listener_pid 2>/dev/null || true' >/dev/null 2>&1 || true"
      sleep 1
      if probe_tcp_from_vm "$a" "$ip_b" "$probe_port"; then
        fail_now "$a unexpectedly reached cross-env workload $b ($ip_b:$probe_port)"
      fi
      if [ -n "$peer_net_ip" ] && probe_tcp_from_vm "$a" "$peer_net_ip" 22; then
        fail_now "$a unexpectedly reached cross-env net VM uplink $peer_net_ip"
      fi
      ok "cross-env workload and uplink traffic stays blocked ($a / $env_a -> $b / $env_b)"
      cross_env_done=1
    fi

    if [ "$host_lan_done" -eq 0 ] && [ -n "$host_lan_ip" ] && [ -n "$host_lan_port" ]; then
      ran_any=1
      if probe_tcp_from_vm "$a" "$host_lan_ip" "$host_lan_port"; then
        fail_now "$a unexpectedly reached the host primary LAN IP $host_lan_ip on port $host_lan_port"
      fi
      if [ -n "$host_lan_gateway" ] && ssh_vm "$a" "command -v ping >/dev/null 2>&1 && ping -c 1 -W 1 $host_lan_gateway" >/dev/null 2>&1; then
        fail_now "$a unexpectedly reached the host LAN gateway $host_lan_gateway"
      fi
      ok "workloads cannot reach the host primary LAN IP or LAN gateway ($a -> $host_lan_ip:$host_lan_port, $host_lan_gateway)"
      host_lan_done=1
    fi
  done
done

[ "$ran_any" -eq 1 ] || skip "no suitable same-env or cross-env running VM pairs found"
log "==> network-isolation OK"
