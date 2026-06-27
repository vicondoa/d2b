#!/usr/bin/env bash
# Layer-2 optional test for live auditd -> journald -> Alloy -> Loki forwarding.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# shellcheck source=tests/lib.sh
. "$ROOT/tests/lib.sh"

MANIFEST=/run/current-system/sw/share/d2b/vms.json

skip() { log "  SKIP: $*"; exit 0; }
fail_now() { fail "$*"; exit 1; }

[ -r "$MANIFEST" ] || skip "manifest missing; d2b not installed on this host"
obs_vm=$(jq -r '._observability.vmName // empty' "$MANIFEST")
[ -n "$obs_vm" ] || skip "observability not enabled"
vm_running "$obs_vm" || skip "observability VM is not running"
[ -n "$(vm_ssh_user "$obs_vm")" ] || skip "observability VM has no SSH credentials in the manifest"
ssh_vm "$obs_vm" true >/dev/null 2>&1 || skip "observability VM is not SSH reachable"

candidate_vm=""
candidate_env=""
while IFS=$'\t' read -r vm env; do
  vm_running "$vm" || continue
  [ -n "$(vm_ssh_user "$vm")" ] || continue
  ssh_vm "$vm" true >/dev/null 2>&1 || continue
  if ! ssh_vm "$vm" 'systemctl is-active --quiet auditd && grep -q "active = yes" /etc/audit/plugins.d/syslog.conf && sudo -n true' >/dev/null 2>&1; then
    continue
  fi
  candidate_vm="$vm"
  candidate_env="$env"
  break
done < <(
  jq -r '
    to_entries[]
    | select(.key | startswith("_") | not)
    | select(.value.isNetVm != true)
    | select(.value.observability.enabled == true)
    | [.key, .value.env]
    | @tsv
  ' "$MANIFEST"
)

[ -n "$candidate_vm" ] || skip "no running audit-enabled workload VM with SSH + sudo was found"

nonce="$$-${RANDOM}"
audit_test_path="/run/d2b-audit-forwarding-test-${nonce}"
audit_test_key="d2b-audit-test-${nonce}"
remote_audit_script=$(cat <<EOF
set -eu
path='$audit_test_path'
key='$audit_test_key'
auditctl -l | grep -F -- '-w /etc/passwd -p wa -k identity' >/dev/null
trap 'auditctl -W "$audit_test_path" >/dev/null 2>&1 || true; rm -f "$audit_test_path"' EXIT
touch "$audit_test_path"
auditctl -w "$audit_test_path" -p wa -k "$audit_test_key" >/dev/null
touch "$audit_test_path"
sleep 1
EOF
)
ssh_vm "$candidate_vm" "sudo -n sh -c $(printf '%q' "$remote_audit_script")" >/dev/null 2>&1 \
  || skip "$candidate_vm could not install a temporary audit rule via sudo"

query=$(printf '{vm="%s",env="%s",source="audit",unit="audisp-syslog"} |= "%s"' "$candidate_vm" "$candidate_env" "$audit_test_key")
query_arg=$(printf '%q' "query=$query")

for _ in $(seq 1 15); do
  out=$(ssh_vm "$obs_vm" "command -v curl >/dev/null 2>&1 && curl -fsS -G --data-urlencode $query_arg http://127.0.0.1:3100/loki/api/v1/query" 2>/dev/null || true)
  if [ -n "$out" ] && printf '%s' "$out" | jq -e '.data.result | length > 0' >/dev/null 2>&1; then
    ok "audit event from $candidate_vm reached Loki with stable audit labels"
    log "==> audit-forwarding OK"
    exit 0
  fi
  sleep 2
done

fail_now "audit event from $candidate_vm did not appear in Loki via the audisp-syslog -> Alloy pipeline"
