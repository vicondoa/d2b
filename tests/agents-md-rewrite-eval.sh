#!/usr/bin/env bash
# Asserts AGENTS.md reflects the P6 daemon-only end-state (ADR 0015):
# no line may describe the bash CLI or a per-VM systemd template as a
# live framework surface. Historical / retired / "deleted in P6"
# context is allowed when the line is explicitly marked as such.
#
# Layer-1, eval-only (no flake build, no daemon, no host state).

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

agents="$ROOT/AGENTS.md"

fail() {
  printf 'tests/agents-md-rewrite-eval: %s\n' "$*" >&2
  exit 1
}

[ -f "$agents" ] || fail "missing $agents"

# Positive invariants: the rewrite must surface the daemon-only
# end-state explicitly and cross-reference ADR 0015.
grep -qE '^## Daemon-only end-state \(P6 onward\)' "$agents" \
  || fail "AGENTS.md is missing the '## Daemon-only end-state (P6 onward)' section"
grep -qE '0015-daemon-only-clean-break\.md' "$agents" \
  || fail "AGENTS.md does not cross-reference docs/adr/0015-daemon-only-clean-break.md"
grep -qE 'nixlingd' "$agents" \
  || fail "AGENTS.md does not mention nixlingd"
grep -qE 'nixling-priv-broker\.socket' "$agents" \
  || fail "AGENTS.md does not mention nixling-priv-broker.socket (socket-activation contract)"
grep -qE 'SpawnRunner' "$agents" \
  || fail "AGENTS.md does not describe broker SpawnRunner for TPM/USBIP/GPU rewire"

# Negative invariants: legacy patterns may not appear UNLESS the same
# line carries an explicit historical / retired marker. This is the
# core P6 docs invariant — agents must not regress AGENTS.md back to
# describing the bash CLI + per-VM systemd templates as canonical.
#
# Forbidden patterns (canonical legacy-as-live shapes):
#   - nixling@<vm>.service / nixling@${name}
#   - microvm@<vm>.service, microvm-virtiofsd@, microvm-set-booted@,
#     microvm-{tap,macvtap,pci}-interfaces@
#   - nixling-<vm>-{gpu,snd,video,swtpm,store-sync}.service
#   - nixling-sys-<env>-usbipd-*
#   - nixling-otel-relay@, nixling-known-hosts-refresh@,
#     nixling-vfsd-watchdog@
#   - host-singleton framework services
#     (nixling-ch-exporter, nixling-otel-host-bridge,
#      nixling-net-route-preflight, nixling-audit-check, microvms.target)
#   - cli.nix as a live module, bash CLI as live surface,
#     NIXLING_LEGACY_BASH_OPT_IN / NIXLING_LEGACY_CLI as live knobs
#
# Allowed-context markers (case-insensitive): any line that ALSO
# mentions one of these may keep the legacy pattern because it is
# describing the deletion / migration / historical end-state itself.
allowed_marker_re="retired|removed|deleted|legacy|historical|no longer|no per-|no per-VM|end-state|P6|pre-v1|v0\.4|ADR 0015|denylist|ph6-|rewire|rewritten|migration|supersedes|reintroduce|Don't|There is no|not mention|moved into|either moved|fail-closed|-style"

forbidden_re='nixling@<vm>|nixling@\$\{name\}|nixling@sys-|microvm@<vm>|microvm-virtiofsd@|microvm-set-booted@|microvm-tap-interfaces@|microvm-macvtap-interfaces@|microvm-pci-devices@|nixling-<vm>-(gpu|snd|video|swtpm|store-sync)\.service|nixling-sys-<env>-usbipd|nixling-otel-relay@|nixling-known-hosts-refresh@|nixling-vfsd-watchdog@|nixling-ch-exporter\.service|nixling-otel-host-bridge\.service|nixling-net-route-preflight\.service|nixling-audit-check\.(service|timer)|microvms\.target|NIXLING_LEGACY_BASH_OPT_IN|NIXLING_LEGACY_CLI|\bbash CLI\b'

violations=0
while IFS= read -r entry; do
  [ -n "$entry" ] || continue
  lineno=${entry%%:*}
  line=${entry#*:}
  if printf '%s\n' "$line" | grep -qEi "$allowed_marker_re"; then
    continue
  fi
  printf 'tests/agents-md-rewrite-eval: AGENTS.md:%s describes a retired surface as live (no historical marker): %s\n' \
    "$lineno" "$line" >&2
  violations=$((violations + 1))
done < <(grep -nE "$forbidden_re" "$agents" || true)

if [ "$violations" -gt 0 ]; then
  fail "$violations line(s) describe retired surfaces as live; see ADR 0015"
fi

printf 'tests/agents-md-rewrite-eval: OK\n'
