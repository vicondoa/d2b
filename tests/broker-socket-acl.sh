#!/usr/bin/env bash
# W2 s3 Layer-1 gate: only nixlingd may talk to the private broker socket.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
MANIFEST=${MANIFEST:-$ROOT/packages/nixling-priv-broker/Cargo.toml}

if [ -n "${NL_RUST_TOOLCHAIN_PATH:-}" ]; then
  PATH="$NL_RUST_TOOLCHAIN_PATH:$PATH"
  export PATH
fi

if [ -z "${NIXLING_BROKER_SOCKET_ACL_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    echo "broker-socket-acl: neither cargo nor nix is on PATH" >&2
    exit 1
  fi
  export NIXLING_BROKER_SOCKET_ACL_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    --command bash "$0" "$@"
fi

# shellcheck source=lib.sh
. "$HERE/lib.sh"

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$(nl_cargo_target_dir broker)}

scratch=$(nl_mktemp .broker-socket-acl.XXXXXX)
mkdir -p "$scratch/run/nixling" "$scratch/var/lib/nixling/audit"
cleanup_broker_socket_acl() {
  if [ -n "${broker_pid:-}" ] && kill -0 "$broker_pid" 2>/dev/null; then
    kill "$broker_pid"
    wait "$broker_pid" || true
  fi
}
add_cleanup cleanup_broker_socket_acl

broker_bin=$(nl_cargo_bin_path broker nixling-priv-broker)
if [ ! -x "$broker_bin" ]; then
  cargo build --manifest-path "$MANIFEST" --features layer1-bootstrap >/dev/null
fi
socket_path=$scratch/run/nixling/priv.sock
audit_dir=$scratch/var/lib/nixling/audit
# W4 retire-shim: the broker now writes a daily-rotated file under
# `$audit_dir/broker-<utc-date>.jsonl`. The legacy single-file
# `/var/lib/nixling/broker-audit.log` is retired.
audit_path=$audit_dir/broker-$(date -u +%F).jsonl
nixlingd_uid=4242
launcher_uid=2001
admin_uid=2002

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
  echo "broker-socket-acl: broker did not create $socket_path" >&2
  cat "$scratch/server.log" >&2 || true
  exit 1
fi

if [ "$(stat -c '%a' "$socket_path")" != "660" ]; then
  echo "broker-socket-acl: expected socket mode 660" >&2
  stat -c 'mode=%a path=%n' "$socket_path" >&2
  exit 1
fi

if "$broker_bin" probe-hello --socket-path "$socket_path" --test-uid 0 >"$scratch/root.out" 2>&1; then
  echo "broker-socket-acl: root peer unexpectedly succeeded" >&2
  cat "$scratch/root.out" >&2 || true
  exit 1
fi

if "$broker_bin" probe-hello --socket-path "$socket_path" --test-uid "$launcher_uid" >"$scratch/launcher.out" 2>&1; then
  echo "broker-socket-acl: launcher peer unexpectedly succeeded" >&2
  cat "$scratch/launcher.out" >&2 || true
  exit 1
fi

if "$broker_bin" probe-hello --socket-path "$socket_path" --test-uid "$admin_uid" >"$scratch/admin.out" 2>&1; then
  echo "broker-socket-acl: admin peer unexpectedly succeeded" >&2
  cat "$scratch/admin.out" >&2 || true
  exit 1
fi

hello_json=$("$broker_bin" probe-hello --socket-path "$socket_path" --test-uid "$nixlingd_uid")
case "$hello_json" in
  *'"response":"HelloOk"'*) ;;
  *)
    echo "broker-socket-acl: nixlingd peer did not receive HelloOk" >&2
    printf '%s\n' "$hello_json" >&2
    exit 1
    ;;
esac

for denied_uid in 0 "$launcher_uid" "$admin_uid"; do
  if ! grep -q "\"caller_uid\":$denied_uid" "$audit_path"; then
    echo "broker-socket-acl: missing denied audit row for uid $denied_uid" >&2
    cat "$audit_path" >&2 || true
    exit 1
  fi
done

printf 'broker-socket-acl: PASS\n'
