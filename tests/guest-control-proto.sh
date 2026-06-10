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

decoded="$scratch/guest_control.pb.txt"
"${protoc_cmd[@]}" \
  --decode=google.protobuf.FileDescriptorSet \
  google/protobuf/descriptor.proto \
  < "$descriptor" > "$decoded"

require_descriptor() {
  local pattern="$1" message="$2"
  if ! grep -q -- "$pattern" "$decoded"; then
    fail "guest-control-proto: descriptor missing $message"
  fi
}

reject_descriptor() {
  local pattern="$1" message="$2"
  if grep -q -- "$pattern" "$decoded"; then
    fail "guest-control-proto: descriptor unexpectedly contains $message"
  fi
}

require_descriptor 'name: "GuestControl"' "GuestControl service"
for method in \
  Hello Capabilities Health ExecCreate ExecInspect ExecWait ExecLogs \
  WriteStdin ReadOutput CloseStdin TtyWinResize ExecSignal ExecCancel
do
  require_descriptor "name: \"$method\"" "method $method"
done

for field in \
  guest_boot_id pending_read_output_waits_per_stream pending_exec_waits_per_vm \
  rpc_rate_per_connection_per_second rpc_rate_per_vm_burst end_offset \
  timed_out retry_after_ms
do
  require_descriptor "name: \"$field\"" "field $field"
done

optional_count=$(grep -c 'proto3_optional: true' "$decoded" || true)
if [ "$optional_count" -lt 6 ]; then
  fail "guest-control-proto: expected optional scalar/string fields in descriptor"
fi

require_descriptor 'name: "outcome"' "TerminalStatus outcome oneof"
require_descriptor 'name: "WRITE_DISPOSITION_REJECTED"' "rejected stdin disposition"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_STDIN_OFFSET_MISMATCH"' "stdin offset mismatch error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_OFFSET_IN_FUTURE"' "offset-in-future error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_PATH_UNSAFE"' "retained-log path error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_RETAINED_LOG_QUOTA_EXCEEDED"' "retained-log quota error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_CWD_INVALID"' "cwd-invalid error"
require_descriptor 'name: "GUEST_CONTROL_ERROR_KIND_CWD_DENIED"' "cwd-denied error"
reject_descriptor 'name: "GUEST_CAPABILITY_READ_GUEST_CONFIG"' "unbacked ReadGuestConfig capability"
reject_descriptor 'name: "SIGNAL_TARGET_ROOT_PROCESS"' "ungated root-process signal target"

ok "guest-control-proto: guest_control.proto descriptor compiles and matches required shape"
