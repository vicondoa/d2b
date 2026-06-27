#!/usr/bin/env bash
# Run the W0b Ubuntu host-check stub locally and show a human-readable diff.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
EXPECTED="$HERE/expected-host-check.json"
STUB="$HERE/host-check-stub.sh"

if ! command -v jq >/dev/null 2>&1; then
  printf 'error: jq is required for the diff helper; the stub itself does not require jq.\n' >&2
  exit 1
fi

normalize_for_local_diff() {
  jq 'del(._comment)
    | .kernel_version.detail = "<expected-to-differ: running kernel>"
    | .nftables.detail = "<expected-to-differ: command version/path>"
    | .nftables_table_d2b.detail = "<expected-to-differ: local nft state>"
    | .firewalld_coexistence.detail = "<expected-to-differ: local firewall tools>"
    | .network_manager_unmanaged.detail = "<expected-to-differ: local NetworkManager state>"
    | .bridge_tap_port_flag_readback.detail = "<expected-to-differ: local ip/jq capability>"
    | .ipv6_sysctl_drift.detail = "<expected-to-differ: local IPv6 sysctls>"
    | .etc_hosts_marked_block.detail = "<expected-to-differ: local /etc/hosts>"
    | .usbip_busid_lock_module_firewall_prereqs.detail = "<expected-to-differ: local module tree>"
    | .nix.detail = "<expected-to-differ: command version/path>"
    | .minijail.detail = "<expected-to-differ: command version/path>"'
}

printf '==> W0b Ubuntu host-check stub on current host\n'
printf 'The following diff compares statuses against the canonical green-host snapshot.\n'
printf 'Known host-specific details are normalized before diffing.\n'
printf 'Fields with status todo-wave-w3 are intentionally deferred to W3 host prepare.\n\n'

if diff -u \
  --label expected-host-check.json \
  --label current-host-check.json \
  --ignore-matching-lines='"detail": "<expected-to-differ:' \
  <(normalize_for_local_diff < "$EXPECTED") \
  <(bash "$STUB" | normalize_for_local_diff); then
  printf '\n==> No unexpected differences.\n'
else
  printf '\n==> Differences above are informational for W0b; this helper does not mutate the host.\n'
fi
