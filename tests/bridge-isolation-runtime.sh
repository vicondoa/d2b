#!/usr/bin/env bash
# Hermetic runtime bridge-isolation regression test.
#
# Layer-1 already proves nixling emits the right networkd shape:
# - the net-VM port (`<env>-l1`) stays non-isolated
# - workload ports (`<env>-l*`) carry bridgeConfig.Isolated = true
#
# This script exercises the corresponding Linux bridge semantics in a
# disposable user+network namespace: one workload must still reach the
# net-VM port, but workload↔workload traffic must stay blocked even if a
# workload spoofs a peer-style MAC address.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
skip() { log "  SKIP: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/bridge-isolation-runtime.sh"

if ! unshare -Urn true 2>/dev/null; then
  skip "unshare -Urn not available; skipping hermetic bridge-isolation runtime check"
  exit 0
fi

unshare -Urn bash <<'EOF'
set -euo pipefail

unshare -n bash -c 'sleep 600' &
NETVM_PID=$!
unshare -n bash -c 'sleep 600' &
VM10_PID=$!
unshare -n bash -c 'sleep 600' &
VM11_PID=$!
trap 'kill "$NETVM_PID" "$VM10_PID" "$VM11_PID" 2>/dev/null || true' EXIT

ip link add br-work-lan type bridge
ip link set br-work-lan up

ip link add work-l1 type veth peer name eth0 netns "$NETVM_PID"
ip link add work-l10 type veth peer name eth0 netns "$VM10_PID"
ip link add work-l11 type veth peer name eth0 netns "$VM11_PID"

ip link set work-l1 master br-work-lan
ip link set work-l10 master br-work-lan
ip link set work-l11 master br-work-lan

ip link set work-l1 up
ip link set work-l10 up
ip link set work-l11 up

bridge link set dev work-l10 isolated on
bridge link set dev work-l11 isolated on

for pid in "$NETVM_PID" "$VM10_PID" "$VM11_PID"; do
  nsenter -t "$pid" -n ip link set lo up
  nsenter -t "$pid" -n ip link set eth0 up
 done

nsenter -t "$NETVM_PID" -n ip addr add 10.20.0.1/24 dev eth0
nsenter -t "$VM10_PID" -n ip addr add 10.20.0.10/24 dev eth0
nsenter -t "$VM11_PID" -n ip addr add 10.20.0.11/24 dev eth0

bridge -d link show dev work-l1 | grep -vq 'isolated on'
bridge -d link show dev work-l10 | grep -q 'isolated on'
bridge -d link show dev work-l11 | grep -q 'isolated on'

nsenter -t "$VM10_PID" -n ping -c1 -W1 10.20.0.1 >/dev/null
nsenter -t "$VM11_PID" -n ping -c1 -W1 10.20.0.1 >/dev/null

if nsenter -t "$VM10_PID" -n ping -c1 -W1 10.20.0.11 >/dev/null 2>&1; then
  echo 'workload→workload ping unexpectedly succeeded before spoof' >&2
  exit 1
fi

nsenter -t "$VM10_PID" -n ip link set dev eth0 address 02:20:00:00:00:11
if nsenter -t "$VM10_PID" -n ping -c1 -W1 10.20.0.11 >/dev/null 2>&1; then
  echo 'workload→workload ping unexpectedly succeeded after peer-style MAC spoof' >&2
  exit 1
fi
EOF

ok "net-VM port remains reachable while workload ports stay isolated across peer-style MAC spoofing"
log "==> bridge-isolation-runtime OK"
