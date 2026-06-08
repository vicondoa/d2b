#!/usr/bin/env bash
# Broker audit log stays append-only and admin-gated.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
MANIFEST=${MANIFEST:-$ROOT/packages/nixling-priv-broker/Cargo.toml}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_BROKER_EXPORT_AUDIT_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "broker-export-audit: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_BROKER_EXPORT_AUDIT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir broker)}

scratch=$(nl_mktemp .broker-export-audit.XXXXXX)
mkdir -p "$scratch/run/nixling" "$scratch/var/lib/nixling/audit"
cleanup_broker_export_audit() {
  if [ -n "${broker_pid:-}" ] && kill -0 "$broker_pid" 2>/dev/null; then
    kill "$broker_pid"
    wait "$broker_pid" || true
  fi
}
add_cleanup cleanup_broker_export_audit

broker_bin=$(nl_cargo_bin_path broker nixling-priv-broker)
if [ ! -x "$broker_bin" ]; then
  cargo build --manifest-path "$MANIFEST" --features layer1-bootstrap >/dev/null
fi
socket_path=$scratch/run/nixling/priv.sock
audit_dir=$scratch/var/lib/nixling/audit
# Retire-shim: the broker now writes a daily-rotated file under
# `$audit_dir/broker-<utc-date>.jsonl`. The legacy single-file
# `/var/lib/nixling/broker-audit.log` is retired.
audit_path=$audit_dir/broker-$(date -u +%F).jsonl
nixlingd_uid=4242

"$broker_bin" serve \
  --socket-path "$socket_path" \
  --audit-dir "$audit_dir" \
  --nixlingd-uid "$nixlingd_uid" \
  --nixlingd-gid "$(id -g)" \
  --test-mode \
  >"$scratch/server.log" 2>&1 &
broker_pid=$!

for _ in $(seq 1 50); do
  [ -S "$socket_path" ] && break
  sleep 0.1
done

if [ ! -S "$socket_path" ]; then
  echo "broker-export-audit: broker did not create $socket_path" >&2
  cat "$scratch/server.log" >&2 || true
  exit 1
fi

"$broker_bin" probe-hello --socket-path "$socket_path" --test-uid "$nixlingd_uid" >/dev/null
"$broker_bin" probe-stub --socket-path "$socket_path" --test-uid "$nixlingd_uid" --operation ApplyNftables >/dev/null

if [ ! -f "$audit_path" ]; then
  echo "broker-export-audit: expected audit log at $audit_path" >&2
  exit 1
fi

owner_gid_mode=$(stat -c '%u:%g:%a' "$audit_path")
expected_owner_gid_mode="$(id -u):$(id -g):640"
if [ "$owner_gid_mode" != "$expected_owner_gid_mode" ]; then
  echo "broker-export-audit: expected simulated ownership/mode $expected_owner_gid_mode but saw $owner_gid_mode" >&2
  exit 1
fi

write_fds=()
for link in /proc/$broker_pid/fd/*; do
  [ -e "$link" ] || continue
  target=$(readlink -f "$link" 2>/dev/null || true)
  if [ "$target" = "$audit_path" ]; then
    fd=${link##*/}
    flags=$(awk '/^flags:/{print $2}' "/proc/$broker_pid/fdinfo/$fd")
    flags_value=$((8#$flags))
    access_mode=$((flags_value & 3))
    if [ "$access_mode" -ne 0 ]; then
      write_fds+=("$fd:$flags")
    fi
  fi
done

if [ "${#write_fds[@]}" -ne 1 ]; then
  echo "broker-export-audit: expected exactly one write fd for the audit log" >&2
  printf 'write_fds=%s\n' "${write_fds[*]:-<none>}" >&2
  ls -l "/proc/$broker_pid/fd" >&2 || true
  exit 1
fi

append_flags=${write_fds[0]#*:}
if [ $(((8#$append_flags) & 02000)) -eq 0 ]; then
  echo "broker-export-audit: audit fd is not O_APPEND" >&2
  printf 'fd=%s flags=%s\n' "${write_fds[0]%:*}" "$append_flags" >&2
  exit 1
fi

unauthorized_json=$("$broker_bin" probe-export-audit --socket-path "$socket_path" --test-uid "$nixlingd_uid" --caller-role not-authorized)
case "$unauthorized_json" in
  *'"kind":"authz-audit-requires-admin"'*) ;;
  *)
    echo "broker-export-audit: expected Authz::AuditRequiresAdmin for non-admin caller role" >&2
    printf '%s\n' "$unauthorized_json" >&2
    exit 1
    ;;
esac

export_json=$("$broker_bin" probe-export-audit --socket-path "$socket_path" --test-uid "$nixlingd_uid" --caller-role admin:9000)
case "$export_json" in
  *ApplyNftables*) ;;
  *)
    echo "broker-export-audit: admin export did not contain the denied ApplyNftables record" >&2
    printf '%s\n' "$export_json" >&2
    exit 1
    ;;
esac

case "$export_json" in
  *"$scratch"*)
    echo "broker-export-audit: exported audit data leaked a filesystem path" >&2
    printf '%s\n' "$export_json" >&2
    exit 1
    ;;
esac

printf 'broker-export-audit: PASS\n'
