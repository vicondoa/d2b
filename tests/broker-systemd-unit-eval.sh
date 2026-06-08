#!/usr/bin/env bash
# v1.1 invariant gate: assert nixling-priv-broker.service +
# nixling-priv-broker.socket are unconditionally configured in
# `nixos-modules/host-broker.nix` (NOT gated behind
# `cfg.daemonExperimental.enable`), and that the canonical
# socket/service shape from host-broker.nix is preserved
# (ListenSequentialPacket = /run/nixling/priv.sock, SocketGroup =
# nixlingd, SocketMode = 0660, serviceConfig.Type = "notify").
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

broker_module="$ROOT/nixos-modules/host-broker.nix"

fail=0

if [ ! -f "$broker_module" ]; then
  printf 'broker-systemd-unit-eval: FAIL — %s missing\n' "$broker_module" >&2
  exit 1
fi

# (a) gating REMOVED — the module must not wrap its config in
# `lib.mkIf cfg.daemonExperimental.enable`.
if grep -q -E 'config\s*=\s*lib\.mkIf\s+cfg\.daemonExperimental\.enable' "$broker_module"; then
  printf 'broker-systemd-unit-eval: FAIL — config still gated behind cfg.daemonExperimental.enable in %s\n' "$broker_module" >&2
  fail=1
fi

# (b) socket declaration present + correct path/group/mode
if ! grep -q -E 'ListenSequentialPacket\s*=\s*"/run/nixling/priv\.sock"' "$broker_module"; then
  printf 'broker-systemd-unit-eval: FAIL — ListenSequentialPacket = "/run/nixling/priv.sock" missing\n' >&2
  fail=1
fi
if ! grep -q -E 'SocketGroup\s*=\s*"nixlingd"' "$broker_module"; then
  printf 'broker-systemd-unit-eval: FAIL — SocketGroup = "nixlingd" missing\n' >&2
  fail=1
fi
if ! grep -q -E 'SocketMode\s*=\s*"0660"' "$broker_module"; then
  printf 'broker-systemd-unit-eval: FAIL — SocketMode = "0660" missing\n' >&2
  fail=1
fi

# (c) serviceConfig.Type = "notify"
if ! grep -q -E 'Type\s*=\s*"notify"' "$broker_module"; then
  printf 'broker-systemd-unit-eval: FAIL — serviceConfig.Type = "notify" missing\n' >&2
  fail=1
fi

# (d) socket unit must wantedBy sockets.target so it activates
# at boot without operator intervention
if ! grep -q -E 'wantedBy\s*=\s*\[\s*"sockets\.target"\s*\]' "$broker_module"; then
  printf 'broker-systemd-unit-eval: FAIL — socket unit not wantedBy sockets.target\n' >&2
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'broker-systemd-unit-eval: PASS\n'
