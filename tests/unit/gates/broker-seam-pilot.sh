#!/usr/bin/env bash
#
# broker-seam-pilot.sh - U10 broker-seam cross-binary E2E gate.
#
# Proves one migrated process-family operation answers END TO END across the
# real broker + daemon binaries with fd carriage and audit continuity:
#
#   driver (python3, envelope wire)  ->  d2b-broker (host, --forward-socket)
#     ->  d2bd forward rendezvous (D2B_BROKER_FORWARD_SOCKET)
#     ->  d2b-provider-process OpenPidfd handler (in-process in d2bd)
#     ->  nested in-broker "open-pidfd" kernel invocation (EnvelopeInvoke
#         over the origination socket, kernel_client.envelope_invoke_kernel)
#     ->  pidfd back over the forward carrier (SCM_RIGHTS)
#
# The committed op names are asserted byte-for-byte. Spellings verified
# against docs/reference/policy/broker-operations.json and the
# KernelInvocation usages in packages/d2bd/src/composition.rs:
#   - "OpenPidfd"   family row (owner=family, declaringProvider
#                   d2b-provider-process; forwarded, never in-broker)
#   - "open-pidfd"  broker-generic kernel row (the nested in-broker leg the
#                   family handler invokes; audited broker-side, KTD6)
#
# The gate FAILS (non-zero + diagnostic) when the live broker audit record
# does not carry the committed kernel op name for the invocation, and when
# the returned pidfd is not a live descriptor (fstat via /proc/self/fd plus
# a signal delivered through the fd path).
#
# Env:
#   D2B_BROKER_BIN  the //packages/d2b-broker-composition:d2b-broker artifact
#   D2B_DAEMON_BIN  the //packages/d2bd:d2bd artifact
#   D2B_SEAM_PILOT_DRIVER_DEADLINE_S  driver retry deadline (default 180)
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(cd "$HERE/../../.." && pwd)}
# The pilot's sockets (forward socket, broker socket, audit paths) must
# stay under the 108-byte UNIX socket path limit; a deep repo worktree
# plus a repo-root scratch exceeds it (ENAMETOOLONG), so scratch lives on
# the short temp base unless the harness pins it elsewhere.
export D2B_TEST_SCRATCH_ROOT=${D2B_TEST_SCRATCH_ROOT:-/tmp}

# shellcheck source=tests/cli-rust-native-common.sh
. "$ROOT/tests/cli-rust-native-common.sh"

# Production posture: the resource plane verifies bundles under the
# production policy (root-owned, mode 0640), so the broker and daemon run
# as root here. A root runner needs no sudo; otherwise sudo -A is
# required (same lane as the host-integration gates).
if [ "$(id -u)" = 0 ]; then
  SUDO=()
else
  SUDO=(sudo -A)
  sudo -A true 2>/dev/null || {
    # Recorded skip, same lane as the host-integration gates: the
    # production resource plane verifies root-owned bundles, which an
    # unprivileged runner cannot stage.
    echo "SKIPPED: broker-seam-pilot requires root (or passwordless sudo -A) for the production bundle posture" >&2
    exit 0
  }
fi

log "==> tests/unit/gates/broker-seam-pilot.sh"

if [ -z "${D2B_SEAM_PILOT_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "broker-seam-pilot requires python3 (or nix to provide it)"
    exit 1
  fi
  export D2B_SEAM_PILOT_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

command -v jq >/dev/null 2>&1 || {
  fail "broker-seam-pilot requires jq"
  exit 1
}

[ -x "${D2B_BROKER_BIN:-}" ] || fail "D2B_BROKER_BIN must name the declared //packages/d2b-broker-composition:d2b-broker artifact"
broker_bin="$D2B_BROKER_BIN"
daemon_bin=$(d2b_daemon_native_bin)

# Committed op names (byte-for-byte, per broker-operations.json + the
# KernelInvocation spellings in packages/d2bd/src/composition.rs).
FAMILY_OP="OpenPidfd"
KERNEL_OP="open-pidfd"
DRIVER_DEADLINE_S=${D2B_SEAM_PILOT_DRIVER_DEADLINE_S:-180}

scratch=$(d2b_mktemp .broker-seam-pilot.XXXXXX)
add_cleanup "${SUDO[*]:+${SUDO[*]} }rm -rf -- \"$scratch\""

broker_socket="$scratch/broker/priv.sock"
broker_audit_dir="$scratch/broker/audit"
broker_state_dir="$scratch/broker/state"
forward_socket="$scratch/forward/d2bd-forward.sock"
daemon_public_socket="$scratch/daemon/public.sock"
daemon_state_lock="$scratch/daemon/daemon.lock"
daemon_locks_dir="$scratch/daemon/locks"
daemon_state_dir="$scratch/daemon/state"
daemon_config="$scratch/daemon/config.json"
mkdir -p "$scratch/broker" "$broker_audit_dir" "$broker_state_dir" \
  "$scratch/forward" "$scratch/daemon" "$daemon_locks_dir" "$daemon_state_dir"
chmod 0755 "$scratch/daemon"

# The trusted v3 bundle tree: a minimal fixture emitted here, mirroring
# the committed test writer `write_minimal_vm_start_bundle_artifacts` in
# packages/d2bd/src/composition.rs (the nix smoke tree helper is stale
# against the current d2b.zones module surface and is not required by
# this gate). The broker resolves its manifest from vms.json and the
# daemon opens its resource plane from bundle.json, publishing the
# process-family providers to the forward rendezvous.
bundle_root="$scratch/artifacts"
mkdir -p "$bundle_root/closures"
cp "$ROOT/tests/fixtures/deny-unknown/host-valid.json" "$bundle_root/host.json"
python3 - "$bundle_root" <<'PYEOF'
import hashlib, json, pathlib, sys

root = pathlib.Path(sys.argv[1])
privileges = {"schemaVersion": "v2", "operations": []}
(root / "privileges.json").write_text(json.dumps(privileges))

manifest = {
    "_manifest": {"manifestVersion": 6},
    "_observability": {
        "enabled": False, "signozUrl": "http://127.0.0.1:8080",
        "signozOtlpGrpcPort": 4317, "signozOtlpHttpPort": 4318,
        "obsVsockCid": 7, "obsVsockHostSocket": "/run/d2b/obs.sock",
        "vmName": "obs",
    },
    "vm-a": {
        "apiSocket": "/run/d2b/vm-a.api.sock", "audio": False,
        "audioService": None, "audioStateFile": "/var/lib/d2b/vms/vm-a/state/audio-state.json",
        "bridge": None, "env": "work", "gpuSocket": "/run/d2b-gpu/vm-a/gpu.sock",
        "graphics": False, "isNetVm": False, "name": "vm-a", "netVm": None,
        "observability": {"agentSocket": "/run/d2b/vms/vm-a/otel.sock", "enabled": False,
                          "vsockCid": 0, "vsockHostSocket": "/run/d2b/otel.sock"},
        "runtime": {"kind": "nixos",
                    "provider": {"id": "local-cloud-hypervisor", "type": "local",
                                 "driver": "cloud-hypervisor"},
                    "capabilities": {"lifecycle": True, "display": True, "usbHotplug": True,
                                     "exec": True, "configSync": True, "ssh": True,
                                     "storeSync": True, "keys": True, "inGuestObservability": True}},
        "sshUser": "alice", "stateDir": "/var/lib/d2b/vms/vm-a",
        "staticIp": "127.0.0.1", "tap": "d2b-vm-a", "tpm": False,
        "tpmSocket": "/run/swtpm/vm-a/swtpm.sock", "usbipYubikey": False,
        "usbipdHostIp": None,
    },
}
(root / "vms.json").write_text(json.dumps(manifest))

processes = {
    "schemaVersion": "v2",
    "vms": [
        {
            "vm": "vm-a",
            "nodes": [
                {
                    "id": "cloud-hypervisor",
                    "role": "cloud-hypervisor-runner",
                    "unit": None,
                    "profile": {
                        "profileId": "vm-vm-a-cloud-hypervisor",
                        "uid": 0, "gid": 0, "adr_carve_out": None,
                        "caps": [],
                        "namespaces": {"mount": False, "pid": False, "net": False,
                                       "ipc": False, "uts": False, "user": False},
                        "seccompPolicyRef": None,
                        "mountPolicy": {"readOnlyPaths": [], "writablePaths": [],
                                        "nixStoreReadOnly": True, "hideDeviceNodesByDefault": True},
                        "cgroupPlacement": {"subtree": "d2b.slice/vm-a/cloud-hypervisor",
                                            "controllers": [], "delegated": False},
                    },
                    "readiness": [],
                }
            ],
            "edges": [],
            "invariants": {"swtpmPreStartFlush": True, "perVmAuditPipeline": True,
                           "usbipGating": True, "tpmOwnershipMigrationWithoutRunningVmMutation": True},
        }
    ],
}
(root / "processes.json").write_text(json.dumps(processes))

# v3 native bundle with the hash-over-nullified-preimage contract
# (mirrors write_v3_native_bundle in packages/d2bd/src/composition.rs).
bundle = {
    "bundleVersion": 1, "schemaVersion": "v3",
    "privilegesPath": str(root / "privileges.json"),
    "zones": [], "artifactHashes": {},
    "generation": {"generator": "broker-seam-pilot", "sourceRevision": None, "generatedAt": None},
}
preimage = dict(bundle)
preimage.pop("bundleHash", None)
preimage["artifactHashes"] = None
# The Rust side derives the digest over the compact serde_json
# serialization of the same keys (alphabetical: serde_json Map is a
# BTreeMap without the preserve_order feature).
digest = hashlib.sha256(json.dumps(preimage, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
bundle["bundleHash"] = f"sha256:{digest}"
(root / "bundle.json").write_text(json.dumps(bundle))
PYEOF
zone="work"
log "zone=$zone bundle_root=$bundle_root (self-generated v3 fixture)"

# The production bundle posture: every artifact the plane verifies is
# root-owned with mode 0640 (files) / 0755 (dirs).
"${SUDO[@]}" chown -R root:root "$bundle_root" "$scratch/broker" "$scratch/audit" \
  "$scratch/state" "$scratch/forward" "$scratch/daemon" "$scratch/locks"
"${SUDO[@]}" chmod 0640 "$bundle_root"/*.json
"${SUDO[@]}" chmod 0755 "$scratch/daemon" "$scratch/locks" "$bundle_root" "$bundle_root/closures"

wait_for_socket() {
  local path="$1"
  local attempts=0
  while [ "$attempts" -lt 300 ]; do
    [ -S "$path" ] && return 0
    attempts=$((attempts + 1))
    sleep 0.1
  done
  fail "timed out waiting for socket: $path"
  return 1
}

cat >"$daemon_config" <<EOF
{
  "publicSocketPath": "$daemon_public_socket",
  "brokerSocketPath": "$broker_socket",
  "stateLockPath": "$daemon_state_lock",
  "locksDir": "$daemon_locks_dir",
  "daemonUser": "root",
  "daemonGroup": "root",
  "publicSocketGroup": "$(id -gn)",
  "launcherUsers": ["launcher-user"],
  "adminUsers": ["admin-user"],
  "serverVersion": "0.4.0",
  "acceptedClientVersionRange": ">=0.4.0, <0.5.0",
  "enableResourcePlane": true,
  "realmControllersConfigPath": "$scratch/daemon/no-realm-controllers.json",
  "realmIdentityConfigPath": "$scratch/daemon/no-realm-identity.json",
  "artifacts": {
    "publicManifestPath": "$bundle_root/vms.json",
    "bundlePath": "$bundle_root/bundle.json",
    "hostPath": "$bundle_root/host.json",
    "processesPath": "$bundle_root/processes.json",
    "closuresDir": "$bundle_root/closures"
  }
}
EOF

# Broker first: the daemon's plane publishes its trusted context to it.
(
  "${SUDO[@]}" "$broker_bin" host \
    --socket-path "$broker_socket" \
    --audit-dir "$broker_audit_dir" \
    --audit-retention-days 0 \
    --bundle-path "$bundle_root/vms.json" \
    --state-dir "$broker_state_dir" \
    --forward-socket "$forward_socket" \
    --d2bd-uid "$(id -u)" \
    --d2bd-gid "$(id -g)" \
    --test-mode
) >"$scratch/broker/serve.log" 2>&1 &
broker_pid=$!
add_cleanup "${SUDO[*]:+${SUDO[*]} }kill $broker_pid >/dev/null 2>&1 || true"
wait_for_socket "$broker_socket"
# The root-owned broker socket must admit the unprivileged driver: hand it
# to the invoking user's group (test-only gate posture).
"${SUDO[@]}" chgrp "$(id -gn)" "$broker_socket"
"${SUDO[@]}" chmod 0770 "$broker_socket"
kill -0 "$broker_pid" 2>/dev/null || {
  fail "d2b-broker exited during startup; see $scratch/broker/serve.log"
  exit 1
}

# Daemon with the forward rendezvous env: it binds the forward socket and,
# once its resource plane opens, publishes the providers + trusted context.
(
  export D2B_BROKER_FORWARD_SOCKET="$forward_socket"
  export D2BD_TEST_PEER_UID=60003
  export D2BD_TEST_PEER_GID=60003
  export D2BD_TEST_PEER_USERNAME=launcher-user
  export D2BD_TEST_PEER_GROUPS=wheel
  "${SUDO[@]}" "$daemon_bin" serve \
    --config "$daemon_config" \
    --test-listen-on "$daemon_public_socket" \
    --state-lock "$daemon_state_lock" \
    --locks-dir "$daemon_locks_dir" \
    --daemon-state-dir "$daemon_state_dir" \
    --allow-unprivileged-runtime-dir \
    --no-drop-privileges
) >"$scratch/daemon/serve.log" 2>&1 &
daemon_pid=$!
add_cleanup "${SUDO[*]:+${SUDO[*]} }kill $daemon_pid >/dev/null 2>&1 || true"
wait_for_socket "$daemon_public_socket"
wait_for_socket "$forward_socket"
kill -0 "$daemon_pid" 2>/dev/null || {
  fail "d2bd exited during startup; see $scratch/daemon/serve.log"
  exit 1
}

# Drive the migrated operation end to end: one EnvelopeInvoke root call for
# the committed family op "OpenPidfd" over the broker's origination socket.
# The broker forwards it to the daemon's rendezvous; the process-family
# handler nests the "open-pidfd" kernel invocation in-broker and returns the
# minted pidfd over the forward carrier. The driver retries the refusals
# that mean "not ready yet" (daemon plane still opening) and hard-fails on
# any other refusal, then proves the returned fd is a live pidfd.
driver_output=$(python3 - "$broker_socket" "$zone" "$(id -u)" "$DRIVER_DEADLINE_S" <<'PY'
import ctypes
import json
import os
import socket
import struct
import subprocess
import sys
import time

BROKER_SOCKET = sys.argv[1]
ZONE = sys.argv[2]
CALLER_UID = int(sys.argv[3])
DEADLINE_S = float(sys.argv[4])
VM_ID = "gate-vm"
ROLE_ID = "runner"

FAMILY_OP = "OpenPidfd"
# Refusals that mean the daemon plane is still opening: the broker holds no
# published values for the zone yet (stale-context), the rendezvous has no
# zone binding or no declaring provider yet (uncommitted-operation), or the
# forwarder cannot reach the daemon yet (unregistered-handler).
RETRYABLE_REFUSALS = {"stale-context", "uncommitted-operation", "unregistered-handler"}

child = subprocess.Popen(["sleep", "300"])
pid = child.pid
try:
    with open(f"/proc/{pid}/stat", encoding="utf-8") as fh:
        stat_text = fh.read()
    comm_end = stat_text.rfind(")")
    fields = stat_text[comm_end + 2:].split()
    starttime = int(fields[19])  # /proc/<pid>/stat field 22 (1-indexed)

    def envelope_frame():
        request = {
            "kind": "EnvelopeInvoke",
            "payload": {
                "operation": FAMILY_OP,
                "zone": ZONE,
                "payload": {
                    "vmId": VM_ID,
                    "roleId": ROLE_ID,
                    "pid": pid,
                    "expectedStartTimeTicks": starttime,
                },
                "chainRootInvocationId": None,
                "chainIdentities": None,
                "fdIndexes": [],
                "fdKinds": [],
            },
        }
        envelope = {
            "request": request,
            "callerRole": {"role": "AdminUid", "uid": CALLER_UID},
            "testPeerUid": CALLER_UID,
            "auditJoin": None,
        }
        body = json.dumps(envelope, separators=(",", ":")).encode()
        return struct.pack("<I", len(body)) + body

    def attempt():
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        sock.settimeout(30.0)
        try:
            sock.connect(BROKER_SOCKET)
            sock.sendall(envelope_frame())
            data, ancdata, _flags, _addr = sock.recvmsg(1024 * 1024, socket.CMSG_SPACE(256))
        finally:
            sock.close()
        if len(data) < 4:
            raise SystemExit("broker response shorter than length prefix")
        declared = struct.unpack("<I", data[:4])[0]
        body = data[4:]
        if declared != len(body):
            raise SystemExit("broker response length prefix mismatch")
        parsed = json.loads(body.decode())
        if parsed.get("kind") != "EnvelopeInvoke":
            raise SystemExit(f"unexpected broker response kind: {parsed.get('kind')!r}")
        return parsed["payload"], ancdata

    deadline = time.monotonic() + DEADLINE_S
    last_error = "no attempt completed"
    response = None
    ancdata = []
    while True:
        try:
            response, ancdata = attempt()
        except (ConnectionRefusedError, FileNotFoundError, ConnectionResetError,
                TimeoutError, OSError) as exc:
            last_error = f"transport: {exc}"
        else:
            refusal = response.get("refusal")
            if refusal is None:
                break
            last_error = f"refused: {refusal} (detail: {response.get('detail')})"
            if refusal not in RETRYABLE_REFUSALS:
                raise SystemExit(f"{FAMILY_OP} refused: {refusal} detail={response.get('detail')}")
        if time.monotonic() >= deadline:
            raise SystemExit(f"{FAMILY_OP} did not succeed within {DEADLINE_S:g}s; last: {last_error}")
        time.sleep(1.0)

    # Wire-continuity: the committed family op name echoed byte-for-byte.
    operation = response.get("operation")
    if operation != FAMILY_OP:
        raise SystemExit(f"response operation {operation!r} != committed family op name {FAMILY_OP!r}")
    invocation_id = response.get("invocation_id")
    if not invocation_id:
        raise SystemExit("response carries no invocation id")
    result = response.get("result") or {}
    if result.get("pid") != pid:
        raise SystemExit(f"result pid {result.get('pid')!r} != child pid {pid}")
    if result.get("verifiedStartTimeTicks") != starttime:
        raise SystemExit(
            f"result verifiedStartTimeTicks {result.get('verifiedStartTimeTicks')!r} != {starttime}"
        )
    if response.get("fd_indexes") != [0]:
        raise SystemExit(f"unexpected fd_indexes: {response.get('fd_indexes')!r}")
    if response.get("fd_kinds") != ["any"]:
        raise SystemExit(f"unexpected fd_kinds: {response.get('fd_kinds')!r}")

    received = []
    for level, ctype_, data_ in ancdata:
        if level == socket.SOL_SOCKET and ctype_ == socket.SCM_RIGHTS:
            received.extend(data_)
    if len(received) != 1:
        raise SystemExit(f"expected exactly one pidfd over SCM_RIGHTS, got {len(received)}")
    pidfd = received[0]

    # fd liveness: the descriptor must be a live pidfd, not just a JSON
    # field. fstat via /proc/self/fd, then a signal delivered through the
    # fd path (signal 0 probe, then SIGTERM) must reach the child.
    target = os.readlink(f"/proc/self/fd/{pidfd}")
    if "pidfd" not in target:
        raise SystemExit(f"received fd {pidfd} is not a pidfd: {target!r}")
    libc = ctypes.CDLL(None, use_errno=True)
    SYS_PIDFD_SEND_SIGNAL = 424  # x86_64 and aarch64 (asm-generic)

    def pidfd_send_signal(fd, sig):
        rc = libc.syscall(SYS_PIDFD_SEND_SIGNAL, fd, sig, None, 0)
        if rc != 0:
            raise OSError(ctypes.get_errno(), os.strerror(ctypes.get_errno()))

    pidfd_send_signal(pidfd, 0)  # liveness probe through the fd
    pidfd_send_signal(pidfd, 15)  # SIGTERM through the fd path
    _waited, status = os.waitpid(pid, 0)
    if not os.WIFSIGNALED(status) or os.WTERMSIG(status) != 15:
        raise SystemExit(f"child did not die from the pidfd SIGTERM: status {status}")
    print(f"INVOCATION_ID={invocation_id}")
finally:
    if child.poll() is None:
        child.kill()
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
PY
)
invocation_id=$(printf '%s\n' "$driver_output" | sed -n 's/^INVOCATION_ID=//p')
[ -n "$invocation_id" ] || {
  fail "driver did not report an invocation id"
  exit 1
}
ok "migrated operation $FAMILY_OP answered end to end (invocation $invocation_id, pidfd live)"

# Audit continuity: the SAME committed op name must appear byte-for-byte in
# the broker's live audit record. The nested in-broker kernel leg
# ("open-pidfd") is the broker-side record of this invocation (KTD6), keyed
# on the root invocation id. Absence or a renamed op fails the gate.
audit_seen=0
attempts=0
while [ "$attempts" -lt 200 ]; do
  audit_file=""
  for candidate in "$broker_audit_dir"/broker-*.jsonl; do
    [ -f "$candidate" ] && audit_file="$candidate" && break
  done
  if [ -n "$audit_file" ] \
    && grep -Fq "\"operation\":\"$KERNEL_OP\"" "$audit_file" \
    && grep -Fq "\"invocation_id\":\"$invocation_id\"" "$audit_file"; then
    audit_seen=1
    break
  fi
  attempts=$((attempts + 1))
  sleep 0.2
done
if [ "$audit_seen" != 1 ]; then
  fail "broker audit log missing committed op name '$KERNEL_OP' for invocation $invocation_id under $broker_audit_dir (expected the U10 chain record of the nested in-broker kernel leg)"
  exit 1
fi
ok "audit continuity: committed op name '$KERNEL_OP' + invocation id $invocation_id in broker audit log"

log "==> broker-seam-pilot.sh PASS"