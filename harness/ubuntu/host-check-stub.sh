#!/usr/bin/env bash
# W0b Ubuntu host-check stub. This is intentionally read-only and always exits 0.

set -euo pipefail

TODO_DETAIL="deferred to W3 host prepare"

json_escape() {
  local s=${1-}
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  s=${s//$'\t'/\\t}
  printf '%s' "$s"
}

print_comment() {
  local value=$1 comma=$2
  printf '  "_comment": "%s"%s\n' "$(json_escape "$value")" "$comma"
}

print_field() {
  local name=$1 status=$2 detail=$3 comma=$4
  printf '  "%s": {\n' "$(json_escape "$name")"
  printf '    "status": "%s",\n' "$(json_escape "$status")"
  printf '    "detail": "%s"\n' "$(json_escape "$detail")"
  printf '  }%s\n' "$comma"
}

kernel_release=$(uname -r 2>/dev/null || printf 'unknown')
kernel_status=unsupported
kernel_base=${kernel_release%%-*}
kernel_major=${kernel_base%%.*}
kernel_minor_part=${kernel_base#*.}
kernel_minor=${kernel_minor_part%%.*}
if [[ $kernel_major =~ ^[0-9]+$ && $kernel_minor =~ ^[0-9]+$ ]]; then
  if [ "$kernel_major" -gt 6 ] || { [ "$kernel_major" -eq 6 ] && [ "$kernel_minor" -ge 6 ]; }; then
    kernel_status=ok
  fi
fi

if [ -e /sys/fs/cgroup/cgroup.controllers ]; then
  cgroup_status=ok
  cgroup_detail=/sys/fs/cgroup/cgroup.controllers
else
  cgroup_status=missing
  cgroup_detail=/sys/fs/cgroup/cgroup.controllers
fi

nft_status=missing
nft_detail="nft command not found"
if command -v nft >/dev/null 2>&1; then
  if nft_detail=$(nft --version 2>&1); then
    nft_status=ok
    nft_detail=${nft_detail%%$'\n'*}
  else
    nft_detail=${nft_detail%%$'\n'*}
  fi
fi

nft_table_status=missing
nft_table_detail="nft command not found"
if command -v nft >/dev/null 2>&1; then
  if nft list table inet d2b >/dev/null 2>&1; then
    nft_table_status=existing-d2b-table
    nft_table_detail="found existing inet d2b table, run d2b host destroy first"
  else
    nft_table_status=ok
    nft_table_detail="inet d2b table absent"
  fi
fi

firewalld_active=0
firewalld_installed=0
firewalld_unit=""
if command -v systemctl >/dev/null 2>&1; then
  if systemctl is-active firewalld >/dev/null 2>&1; then
    firewalld_active=1
    firewalld_installed=1
  elif firewalld_unit=$(systemctl list-unit-files firewalld.service --no-legend 2>/dev/null) \
    && printf '%s\n' "$firewalld_unit" | grep -q '^firewalld\.service[[:space:]]'; then
    firewalld_installed=1
  fi
fi
if command -v firewall-cmd >/dev/null 2>&1; then
  firewalld_installed=1
fi

ufw_active=0
ufw_installed=0
ufw_output=""
if command -v ufw >/dev/null 2>&1; then
  ufw_installed=1
  ufw_output=$(ufw status 2>/dev/null || true)
  if printf '%s\n' "$ufw_output" | grep -qi '^Status:[[:space:]]*active'; then
    ufw_active=1
  fi
fi

firewall_status=unsupported
firewall_detail="firewalld and ufw absent"
if [ "$firewalld_active" -eq 1 ] || [ "$ufw_active" -eq 1 ]; then
  firewall_status=refuse
  firewall_detail="firewalld_active=$firewalld_active ufw_active=$ufw_active"
elif [ "$firewalld_installed" -eq 1 ] || [ "$ufw_installed" -eq 1 ]; then
  firewall_status=coexist
  firewall_detail="firewalld_installed=$firewalld_installed ufw_installed=$ufw_installed inactive"
fi

nm_status=unsupported
nm_detail="NetworkManager not present"
nm_running=0
if command -v systemctl >/dev/null 2>&1 && systemctl is-active NetworkManager >/dev/null 2>&1; then
  nm_running=1
fi
if [ -d /etc/NetworkManager ] || command -v NetworkManager >/dev/null 2>&1 || command -v nmcli >/dev/null 2>&1; then
  nm_detail="NetworkManager present but not active"
  if [ -d /etc/NetworkManager/conf.d ]; then
    nm_found_config=""
    shopt -s nullglob
    for nm_conf in /etc/NetworkManager/conf.d/*.conf; do
      if grep -q 'unmanaged-devices' "$nm_conf" 2>/dev/null && grep -q 'd2b' "$nm_conf" 2>/dev/null; then
        nm_found_config=$nm_conf
        break
      fi
    done
    shopt -u nullglob
    if [ -n "$nm_found_config" ]; then
      nm_status=ok
      nm_detail="found d2b unmanaged-devices config: $nm_found_config"
    elif [ "$nm_running" -eq 1 ]; then
      nm_status=todo
      nm_detail="NetworkManager is active but no d2b unmanaged-devices config was found"
    fi
  elif [ "$nm_running" -eq 1 ]; then
    nm_status=todo
    nm_detail="NetworkManager is active but /etc/NetworkManager/conf.d is absent"
  fi
fi

bridge_readback_status=missing
bridge_readback_detail="ip or jq command not found"
if command -v ip >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  if ip -j -d link show type bridge 2>/dev/null | jq . >/dev/null 2>&1; then
    bridge_readback_status=ok
    bridge_readback_detail="ip -j -d link show type bridge parses with jq"
  else
    bridge_readback_detail="ip bridge JSON did not parse with jq"
  fi
fi

ipv6_disable_all_path=/proc/sys/net/ipv6/conf/all/disable_ipv6
ipv6_accept_ra_default_path=/proc/sys/net/ipv6/conf/default/accept_ra
ipv6_status=missing
ipv6_detail="IPv6 sysctl files not readable"
if [ -r "$ipv6_disable_all_path" ] && [ -r "$ipv6_accept_ra_default_path" ]; then
  ipv6_disable_all=$(<"$ipv6_disable_all_path")
  ipv6_accept_ra_default=$(<"$ipv6_accept_ra_default_path")
  ipv6_status=ok
  ipv6_detail="all.disable_ipv6=$ipv6_disable_all default.accept_ra=$ipv6_accept_ra_default"
fi

ifnamsiz_regex='^nx[a-z0-9-]{0,12}$'
ifnamsiz_detail="W3 interface-name regex: $ifnamsiz_regex"

hosts_status=unsupported
hosts_detail="/etc/hosts not readable"
if [ -r /etc/hosts ]; then
  hosts_count=$(grep -c '# BEGIN d2b' /etc/hosts 2>/dev/null || true)
  if [ "$hosts_count" -gt 0 ]; then
    hosts_status=existing
    hosts_detail="found $hosts_count d2b marked block(s) in /etc/hosts"
  else
    hosts_status=ok
    hosts_detail="no d2b marked block found in /etc/hosts"
  fi
fi

usbip_status=unsupported
usbip_detail="modprobe command not found"
if command -v modprobe >/dev/null 2>&1; then
  if usbip_detail=$(modprobe -n usbip_host 2>&1); then
    usbip_status=ok
    usbip_detail=${usbip_detail%%$'\n'*}
    if [ -z "$usbip_detail" ]; then
      usbip_detail="usbip_host module dry-run succeeded"
    fi
  else
    usbip_status=missing
    usbip_detail=${usbip_detail%%$'\n'*}
  fi
fi

nix_status=missing
nix_detail="nix command not found"
if command -v nix >/dev/null 2>&1; then
  if nix_detail=$(nix --version 2>&1); then
    nix_status=ok
    nix_detail=${nix_detail%%$'\n'*}
  else
    nix_detail=${nix_detail%%$'\n'*}
  fi
fi

if [ -e /dev/kvm ]; then
  if [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    kvm_status=ok
    kvm_detail="/dev/kvm present and accessible"
  else
    kvm_status=permission-denied
    kvm_detail="/dev/kvm exists but is not readable and writable by this user"
  fi
else
  kvm_status=missing
  kvm_detail="/dev/kvm not present"
fi

minijail_status=missing
minijail_detail="minijail0 command not found"
if command -v minijail0 >/dev/null 2>&1; then
  if minijail_detail=$(minijail0 --help 2>&1); then
    minijail_status=ok
    minijail_detail=${minijail_detail%%$'\n'*}
  else
    minijail_detail=${minijail_detail%%$'\n'*}
  fi
fi

printf '{\n'
print_comment "Canonical W0b stub output. Regenerate expected-host-check.json by running: bash harness/ubuntu/host-check-stub.sh | jq . > harness/ubuntu/expected-host-check.json, then normalize host-specific green-host details before committing." ","
print_field kernel_version "$kernel_status" "$kernel_release" ","
print_field cgroup_v2_unified "$cgroup_status" "$cgroup_detail" ","
print_field nftables "$nft_status" "$nft_detail" ","
print_field nftables_table_d2b "$nft_table_status" "$nft_table_detail" ","
print_field firewalld_coexistence "$firewall_status" "$firewall_detail" ","
print_field network_manager_unmanaged "$nm_status" "$nm_detail" ","
print_field route_preflight todo-wave-w3 "deferred to W3 host prepare; d2b envs not yet declared" ","
print_field bridge_tap_port_flag_readback "$bridge_readback_status" "$bridge_readback_detail" ","
print_field ipv6_sysctl_drift "$ipv6_status" "$ipv6_detail" ","
print_field ifnamsiz_validation_negatives ok "$ifnamsiz_detail" ","
print_field etc_hosts_marked_block "$hosts_status" "$hosts_detail" ","
print_field usbip_busid_lock_module_firewall_prereqs "$usbip_status" "$usbip_detail" ","
print_field nix "$nix_status" "$nix_detail" ","
print_field kvm "$kvm_status" "$kvm_detail" ","
print_field minijail "$minijail_status" "$minijail_detail" ","
print_field vhost_net todo-wave-w3 "$TODO_DETAIL" ","
print_field vhost_vsock todo-wave-w3 "$TODO_DETAIL" ","
print_field cgroup_delegation todo-wave-w3 "$TODO_DETAIL" ","
print_field tuntap todo-wave-w3 "$TODO_DETAIL" ","
print_field cgroup.kill todo-wave-w3 "$TODO_DETAIL" ","
print_field pidfd_open todo-wave-w3 "$TODO_DETAIL" ","
print_field selinux todo-wave-w3 "$TODO_DETAIL" ","
print_field apparmor todo-wave-w3 "$TODO_DETAIL" ","
print_field init_system todo-wave-w3 "$TODO_DETAIL" ","
print_field glibc_version todo-wave-w3 "$TODO_DETAIL" ","
print_field architecture todo-wave-w3 "$TODO_DETAIL" ""
printf '}\n'

exit 0
