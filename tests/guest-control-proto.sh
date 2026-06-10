#!/usr/bin/env bash
# Validate the checked-in guest-control protobuf source compiles to a descriptor.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

proto_dir="$ROOT/packages/nixling-ipc/proto"
proto="$proto_dir/guest_control.proto"
scratch=$(nl_mktemp .guest-control-proto.XXXXXX)
descriptor="$scratch/guest_control.pb"

if [ ! -f "$proto" ]; then
  fail "guest-control-proto: missing $proto"
fi

if command -v protoc >/dev/null 2>&1; then
  protoc_cmd=(protoc)
else
  protoc_cmd=(nix shell --quiet --inputs-from "$ROOT" nixpkgs#protobuf --command protoc)
fi

"${protoc_cmd[@]}" \
  --proto_path="$proto_dir" \
  --include_source_info \
  --descriptor_set_out="$descriptor" \
  "$proto"

if [ ! -s "$descriptor" ]; then
  fail "guest-control-proto: empty descriptor output"
fi

ok "guest-control-proto: guest_control.proto descriptor compiles"
