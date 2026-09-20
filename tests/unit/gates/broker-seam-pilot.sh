#!/usr/bin/env bash
#
# broker-seam-pilot.sh - U10/U1 broker-seam cross-binary E2E gate.
#
# Proves the migrated process-family operations that are HERMETIC answer
# END TO END across the real broker + daemon binaries:
#
#   driver (python3, envelope wire)  ->  d2b-broker (host, --forward-socket)
#     ->  d2bd forward rendezvous (D2B_BROKER_FORWARD_SOCKET)
#     ->  d2b-provider-process operation handlers (in-process in d2bd)
#
# Proven per operation:
#   - "OpenPidfd":            pidfd back over the forward carrier
#                             (SCM_RIGHTS) plus a nested in-broker
#                             "open-pidfd" kernel leg (EnvelopeInvoke over
#                             the origination socket) audited broker-side -
#                             fd liveness (fstat via /proc/self/fd + a
#                             signal through the fd path) and audit
#                             continuity are asserted.
#   - "inspect-process-family": no fd leg; the committed family spelling,
#                             the member roster, and the full 11-row
#                             operations inventory are asserted
#                             byte-for-byte, plus zone wiring.
#   - "PollChildReaped":      no fd leg; the committed spelling and the
#                             empty notifications record are asserted.
#   - "SeedDnsmasqLease":     the network family's hermetic migrated
#                             operation: the derived per-VM dnsmasq lease
#                             admission check and its acknowledgement
#                             through the nested in-broker
#                             "seed-dnsmasq-lease" kernel leg (audited
#                             broker-side, KTD6) - the committed spelling,
#                             the ack, and audit continuity are asserted,
#                             with no fd leg.
#
# The committed op names are asserted byte-for-byte. Spellings verified
# against docs/reference/policy/broker-operations.json and the
# KernelInvocation usages in packages/d2bd/src/composition.rs:
#   - "OpenPidfd"             family row (owner=family, declaringProvider
#                             d2b-provider-process; forwarded, never
#                             in-broker)
#   - "open-pidfd"            broker-generic kernel row (the nested
#                             in-broker leg the family handler invokes;
#                             audited broker-side, KTD6)
#   - "inspect-process-family" / "PollChildReaped" are forwarded family
#                             rows with no nested kernel leg; the daemon
#                             audits them daemon-side only (KTD6), so the
#                             broker audit log is deliberately NOT grepped
#                             for them.
#   - "SeedDnsmasqLease"      family row (owner=family, declaringProvider
#                             d2b-provider-network-local; forwarded, never
#                             in-broker)
#   - "seed-dnsmasq-lease"    broker-generic kernel row (the nested
#                             in-broker leg the family handler invokes;
#                             audited broker-side, KTD6)
#
# NOT proven here, by design, and why: the remaining eight process-family
# rows mutate host state or touch live processes
# (OpenPeerPidfdFromAcceptedSocket, ObserveRunner, PrepareRuntimeDir,
# PrepareStateDir, CgroupKill, SignalRunner, DeregisterRunnerPidfd,
# SpawnRunner), and the remaining twelve network-family rows mutate the
# host fabric (ApplyNftablesProjection, ApplyNmUnmanaged, ApplyRoute,
# ApplySysctl, CreateBridge, CreatePersistentTap, CreateTapFd,
# DeleteBridge, DeletePersistentTap, SetBridgePortFlags, UpdateHostsFile,
# ApplyNftables); both sets belong to the host-integration lane that
# stage-runs real workers. Restart adoption (KTD8) needs a daemon restart
# between two live planes, which the fresh binaries of this gate cannot
# stage - see the daemon-smoke host-integration check's restart stage. The
# orchestrator runs this gate after the unit lands; the
# operation-to-scenario mapping lives in the U1/U14 plan sections.
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
INSPECT_OP="inspect-process-family"
POLL_OP="PollChildReaped"
SEED_OP="SeedDnsmasqLease"
SEED_KERNEL_OP="seed-dnsmasq-lease"
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
# this gate). The broker and the daemon both resolve the same
# zone-native bundle.json (the resolver refuses any non-v3 artifact at
# --bundle-path), and the daemon opens its resource plane from it,
# publishing the process- and network-family providers to the forward
# rendezvous.
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

def framed_digest(domain, payload):
    frame = {"domain": domain, "framing": "d2b-digest/v1", "payload": payload}
    canonical = json.dumps(frame, sort_keys=True, separators=(",", ":"))
    return "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()

def sha256_bytes(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()

# The daemon's resource plane opens only over a committed Zone set: the
# bundle must carry one zone's resource bundle, its storage row, and the
# sealed topology index, each pinned in artifactHashes (the v3 loader
# verifies ownership, mode, and hash for every private artifact). The zone
# bundle carries zero resources - the plane opens and publishes the fixed
# provider set, which is all the hermetic operations need.
zone_dir = root / "zones" / "work"
zone_dir.mkdir(parents=True)
zone_uid = "123e4567-e89b-42d3-a456-426614174000"
store_uid = "223e4567-e89b-42d3-a456-426614174001"
content_hash = framed_digest("d2b:v3:resource-bundle", "[]")
resource_bundle = {
    "schemaVersion": 3, "bundleVersion": 1, "zone": "work",
    "zoneUid": zone_uid,
    "contentHash": content_hash,
    "artifactCatalogDigest": "sha256:" + "0" * 64,
    "schemaFingerprints": {}, "providerSchemaDigests": {},
    "resources": [],
    "generatedAt": "1970-01-01T00:00:00.000Z",
}
(zone_dir / "resource-bundle.json").write_text(
    json.dumps(resource_bundle, sort_keys=True, separators=(",", ":"))
)
storage_row = {
    "identity": {"zoneUid": zone_uid, "storeUid": store_uid, "storeEpoch": 1},
    "zoneStoreId": "zone-store-work",
    "storageOwnerPrincipal": "d2b-zonert",
    "parentDirectoryId": "zone-store-parent-work",
    "ownership": {
        "owner": "d2b-zonert", "group": "d2b-zonert", "mode": "0640", "linkCount": 1,
    },
    "auxiliaryDirectories": {
        "audit": {
            "directoryId": "zone-store-audit-work", "owner": "d2bd", "group": "d2bd",
            "mode": "0700", "repairOwner": "privileged-broker",
        },
        "telemetry": {
            "directoryId": "zone-store-telemetry-work", "owner": "d2bd", "group": "d2bd",
            "mode": "0700", "repairOwner": "privileged-broker",
        },
    },
    "filesystem": "regular-file-anchored-fd-relative-no-follow",
    "locking": "ofd-close-on-exec",
    "marker": {"identityMarkerId": "zone-store-marker-work"},
    "replacementDetection": "fail-closed-on-missing-replaced-or-identity-mismatch",
    "fsync": "database-and-parent-directory",
    "publication": {
        "descriptor": "owned-descriptor-close-on-exec-verified-before-concurrency",
        "replacement": "atomic-rename-retain-prior-quarantine-ambiguity",
    },
}
(zone_dir / "storage.json").write_text(
    json.dumps(storage_row, sort_keys=True, separators=(",", ":"))
)
parent_map = {"work": None}
index_document = {
    "zones": {"work": {"zoneUid": zone_uid}},
    "topology": {
        "sealed": True,
        "parentMap": parent_map,
        "parentMapDigest": framed_digest(
            "d2b:v3:parent-topology",
            json.dumps(parent_map, sort_keys=True, separators=(",", ":")),
        ),
        "generationByZone": {"work": content_hash},
    },
}
(root / "index.json").write_text(
    json.dumps(index_document, sort_keys=True, separators=(",", ":"))
)

# v3 native bundle with the hash-over-nullified-preimage contract
# (mirrors write_v3_native_bundle in packages/d2bd/src/composition.rs).
bundle = {
    "bundleVersion": 1, "schemaVersion": "v3",
    "privilegesPath": str(root / "privileges.json"),
    "zones": [{"zone": "work", "path": "zones/work/resource-bundle.json"}],
    "artifactHashes": {
        "zones/work/resource-bundle.json": sha256_bytes(
            (zone_dir / "resource-bundle.json").read_bytes()
        ),
        "zones/work/storage.json": sha256_bytes((zone_dir / "storage.json").read_bytes()),
        "index.json": sha256_bytes((root / "index.json").read_bytes()),
    },
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
# root:d2bd-owned with mode 0640 (files) / 0755 (dirs) - the exact
# owner/group/mode the BundleVerifyPolicy::production() tamper check
# enforces (owner uid 0, group d2bd when the group exists, mode 0640).
# The pilot may run inside a user namespace whose gid map cannot express
# the host d2bd group (unshare -rm on a host that has one): chown to that
# group fails with EINVAL there. The resolver skips the GID check when
# the group is absent from /etc/group, so on such hosts the gate stages
# root:root and installs a namespace-local /etc/group view without the
# d2bd line (bind mount over the private mount namespace unshare -rm
# creates; the host view is untouched, and on a shared mount namespace
# the mount fails closed before anything is staged). On hosts where the
# group is chown-able the gate stages the real root:d2bd posture.
bundle_group=root
if getent group d2bd >/dev/null 2>&1; then
  probe="$scratch/.ownership-probe"
  : >"$probe"
  if "${SUDO[@]}" chown root:d2bd "$probe" 2>/dev/null; then
    bundle_group=d2bd
  else
    etc_group="$scratch/etc-group"
    sed '/^d2bd:/d' /etc/group >"$etc_group"
    if ! "${SUDO[@]}" mount --bind "$etc_group" /etc/group 2>/dev/null; then
      rm -f "$probe"
      fail "cannot stage the production bundle posture: the d2bd group is not chown-able in this namespace and /etc/group is not bind-mountable"
      exit 1
    fi
    add_cleanup "${SUDO[*]:+${SUDO[*]} }umount /etc/group >/dev/null 2>&1 || true"
  fi
  rm -f "$probe"
fi
"${SUDO[@]}" chown -R root:"$bundle_group" "$bundle_root" "$scratch/broker" "$broker_audit_dir" \
  "$broker_state_dir" "$scratch/forward" "$scratch/daemon" "$daemon_locks_dir"
"${SUDO[@]}" chmod 0640 "$bundle_root"/*.json "$bundle_root"/zones/work/*.json
"${SUDO[@]}" chmod 0755 "$scratch/daemon" "$daemon_locks_dir" "$bundle_root" "$bundle_root/zones" "$bundle_root/zones/work" "$bundle_root/closures"

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
    --bundle-path "$bundle_root/bundle.json" \
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

# Drive the hermetic migrated operations end to end: one EnvelopeInvoke root
# call per committed family op over the broker's origination socket. The
# broker forwards each to the daemon's rendezvous; the process- and
# network-family handlers answer in-process in d2bd. "OpenPidfd" nests the
# "open-pidfd" kernel invocation in-broker and returns the minted pidfd over
# the forward carrier; the two pure process operations return plain result
# objects with no fd leg; "SeedDnsmasqLease" nests the "seed-dnsmasq-lease"
# kernel invocation in-broker and returns its acknowledgement (the derived
# per-VM lease admission check, no host mutation, no fd leg). The driver
# retries the refusals that mean "not ready yet" (daemon plane still
# opening) and hard-fails on any other refusal, then proves the OpenPidfd fd
# is a live pidfd and the pure results carry the committed spellings and
# shapes.
driver_output=$(python3 - "$broker_socket" "$zone" "$(id -u)" "$DRIVER_DEADLINE_S" "$INSPECT_OP" "$POLL_OP" "$SEED_OP" <<'PY'
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
INSPECT_OP = sys.argv[5]
POLL_OP = sys.argv[6]
SEED_OP = sys.argv[7]
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

    def envelope_frame(operation, payload):
        request = {
            "kind": "EnvelopeInvoke",
            "payload": {
                "operation": operation,
                "zone": ZONE,
                "payload": payload,
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

    def attempt(operation, payload):
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        sock.settimeout(30.0)
        try:
            sock.connect(BROKER_SOCKET)
            sock.sendall(envelope_frame(operation, payload))
            data, ancdata, _flags, _addr = sock.recvmsg(1024 * 1024, socket.CMSG_SPACE(256))
        finally:
            sock.close()
        if len(data) < 4:
            raise SystemExit(f"{operation}: broker response shorter than length prefix")
        declared = struct.unpack("<I", data[:4])[0]
        body = data[4:]
        if declared != len(body):
            raise SystemExit(f"{operation}: broker response length prefix mismatch")
        parsed = json.loads(body.decode())
        if parsed.get("kind") != "EnvelopeInvoke":
            raise SystemExit(f"{operation}: unexpected broker response kind: {parsed.get('kind')!r}")
        return parsed["payload"], ancdata

    def run_op(operation, payload, deadline_s):
        deadline = time.monotonic() + deadline_s
        last_error = "no attempt completed"
        while True:
            try:
                response, ancdata = attempt(operation, payload)
            except (ConnectionRefusedError, FileNotFoundError, ConnectionResetError,
                    TimeoutError, OSError) as exc:
                last_error = f"transport: {exc}"
            else:
                refusal = response.get("refusal")
                if refusal is None:
                    return response, ancdata
                last_error = f"refused: {refusal} (detail: {response.get('detail')})"
                if refusal not in RETRYABLE_REFUSALS:
                    raise SystemExit(f"{operation} refused: {refusal} detail={response.get('detail')}")
            if time.monotonic() >= deadline:
                raise SystemExit(f"{operation} did not succeed within {deadline_s:g}s; last: {last_error}")
            time.sleep(1.0)

    response, ancdata = run_op(FAMILY_OP, {
        "vmId": VM_ID,
        "roleId": ROLE_ID,
        "pid": pid,
        "expectedStartTimeTicks": starttime,
    }, DEADLINE_S)

    # Wire-continuity: the committed family op name echoed byte-for-byte.
    operation = response.get("operation")
    if operation != FAMILY_OP:
        raise SystemExit(f"response operation {operation!r} != committed family op name {FAMILY_OP!r}")
    invocation_id = response.get("invocation_id") or response.get("invocationId")
    if not invocation_id:
        raise SystemExit(f"response carries no invocation id; raw={json.dumps(response)[:600]}")
    result = response.get("result") or {}
    if result.get("pid") != pid:
        raise SystemExit(f"result pid {result.get('pid')!r} != child pid {pid}")
    if result.get("verifiedStartTimeTicks") != starttime:
        raise SystemExit(
            f"result verifiedStartTimeTicks {result.get('verifiedStartTimeTicks')!r} != {starttime}"
        )
    if response.get("fdIndexes") != [0]:
        raise SystemExit(f"unexpected fd_indexes: {response.get('fd_indexes')!r}")
    if response.get("fdKinds") != ["any"]:
        raise SystemExit(f"unexpected fd_kinds: {response.get('fd_kinds')!r}")

    received = []
    for level, ctype_, data_ in ancdata:
        if level == socket.SOL_SOCKET and ctype_ == socket.SCM_RIGHTS:
            received.extend(data_)
    # The carrier appends its own relay descriptors to the carried ones, so the
    # reply holds more than the declared leg. Select the pidfd by kind: exactly
    # one received descriptor must resolve to a pidfd, and it must be the one
    # the declared leg names.
    targets = {}
    for fd in received:
        try:
            targets[fd] = os.readlink(f"/proc/self/fd/{fd}")
        except OSError as exc:
            targets[fd] = f"<unreadable: {exc}>"
    pidfds = [fd for fd, target in targets.items() if "pidfd" in target]
    if len(pidfds) != 1:
        raise SystemExit(
            f"expected exactly one pidfd over SCM_RIGHTS, got {len(pidfds)} of "
            f"{len(received)}: {targets}"
        )
    pidfd = pidfds[0]
    if received.index(pidfd) != response["fdIndexes"][0]:
        raise SystemExit(
            f"the pidfd is not at the declared leg index {response['fdIndexes']!r}: "
            f"received {targets}"
        )

    # fd liveness: the descriptor must be a live pidfd, not just a JSON
    # field. fstat via /proc/self/fd, then a signal delivered through the
    # fd path (signal 0 probe, then SIGTERM) must reach the child.
    target = targets[pidfd]
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

    def no_fd_leg(operation, response, ancdata):
        if response.get("fdIndexes") != [] or response.get("fdKinds") != []:
            raise SystemExit(
                f"{operation}: unexpected fd leg: fdIndexes={response.get('fdIndexes')!r} "
                f"fdKinds={response.get('fdKinds')!r}"
            )
        for level, ctype_, _data_ in ancdata:
            if level == socket.SOL_SOCKET and ctype_ == socket.SCM_RIGHTS:
                raise SystemExit(f"{operation}: returned SCM_RIGHTS descriptors")

    # The family inventory op is hermetic: no fd leg, the committed spelling
    # echoed byte-for-byte, the member roster, the full 11-row operations
    # inventory, and zone wiring all asserted. Spellings match
    # packages/d2b-provider-process/src/operations.rs (INSPECT_PROCESS_FAMILY
    # handler) byte for byte.
    response, ancdata = run_op(INSPECT_OP, {"resourceType": "Process"}, min(DEADLINE_S, 60.0))
    if response.get("operation") != INSPECT_OP:
        raise SystemExit(
            f"response operation {response.get('operation')!r} != committed family op name {INSPECT_OP!r}"
        )
    inspection = response.get("result") or {}
    if inspection.get("family") != "process":
        raise SystemExit(f"inspect family {inspection.get('family')!r} != 'process'")
    if inspection.get("resourceType") != "Process":
        raise SystemExit(f"inspect resourceType {inspection.get('resourceType')!r} != 'Process'")
    if inspection.get("zone") != ZONE:
        raise SystemExit(f"inspect zone {inspection.get('zone')!r} != driver zone {ZONE!r}")
    member_types = inspection.get("memberTypes") or []
    if set(member_types) != {"Process", "EphemeralProcess"}:
        raise SystemExit(f"unexpected memberTypes: {member_types!r}")
    expected_operations = [
        "inspect-process-family",
        "OpenPidfd",
        "OpenPeerPidfdFromAcceptedSocket",
        "ObserveRunner",
        "PollChildReaped",
        "PrepareRuntimeDir",
        "PrepareStateDir",
        "CgroupKill",
        "SignalRunner",
        "DeregisterRunnerPidfd",
        "SpawnRunner",
    ]
    if inspection.get("operations") != expected_operations:
        raise SystemExit(
            f"operations inventory mismatch: {inspection.get('operations')!r} != {expected_operations!r}"
        )
    no_fd_leg(INSPECT_OP, response, ancdata)
    print(f"INSPECT_OK={INSPECT_OP}")

    # PollChildReaped is hermetic in this lane: the committed spelling and
    # an empty notification record, with no fd leg (a live child would need
    # the host-integration lane to observe a reaping window).
    response, ancdata = run_op(POLL_OP, {}, min(DEADLINE_S, 60.0))
    if response.get("operation") != POLL_OP:
        raise SystemExit(
            f"response operation {response.get('operation')!r} != committed family op name {POLL_OP!r}"
        )
    poll_result = response.get("result") or {}
    if poll_result.get("notifications") != []:
        raise SystemExit(f"PollChildReaped notifications not empty: {poll_result.get('notifications')!r}")
    no_fd_leg(POLL_OP, response, ancdata)
    print(f"POLL_OK={POLL_OP}")

    # SeedDnsmasqLease is the network family's hermetic migrated operation
    # (U14): the per-VM dnsmasq lease row is derived, never caller-supplied,
    # so the handler runs the kernel's pure admission check - the child VM
    # name re-derived from the admitted Network identity, the scope, and the
    # nonzero generations - and returns the acknowledgement, with no fd leg
    # and no host mutation. The committed family spelling is echoed
    # byte-for-byte, and the nested in-broker "seed-dnsmasq-lease" kernel
    # leg is audited broker-side keyed on this invocation's root id (KTD6).
    # The derivation mirrors d2b_contracts::v3::derive_network_child_name
    # (FNV-1a 64 over "d2b-network-child/v1\\0<uid>\\0vm", Crockford base32,
    # eight characters, lowercased).
    network_uid = "323e4567-e89b-42d3-a456-426614174002"
    zone_uid = "223e4567-e89b-42d3-a456-426614174001"

    def fnv1a(data):
        h = 0xCBF29CE484222325
        for byte in data:
            h ^= byte
            h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
        return h

    def base32_crockford(value, characters):
        alphabet = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
        out = []
        for _ in range(characters):
            out.append(alphabet[value & 0x1F])
            value >>= 5
        return "".join(reversed(out))

    def derive_network_child_name(network_uid, kind):
        data = b"d2b-network-child/v1" + b"\x00" + network_uid.encode() + b"\x00" + kind.encode()
        return f"net-{kind}-{base32_crockford(fnv1a(data), 8).lower()}"

    expected_vm = derive_network_child_name(network_uid, "vm")
    response, ancdata = run_op(SEED_OP, {
        "vmId": expected_vm,
        "scopeId": f"network:{zone_uid}:{network_uid}",
        "zoneUid": zone_uid,
        "networkUid": network_uid,
        "networkGeneration": 7,
        "attachmentGeneration": 11,
        "bundleGeneration": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    }, min(DEADLINE_S, 60.0))
    if response.get("operation") != SEED_OP:
        raise SystemExit(
            f"response operation {response.get('operation')!r} != committed family op name {SEED_OP!r}"
        )
    seed_invocation_id = response.get("invocation_id") or response.get("invocationId")
    if not seed_invocation_id:
        raise SystemExit("seed response carries no invocation id")
    seed_result = response.get("result") or {}
    if seed_result.get("seeded") is not True:
        raise SystemExit(f"SeedDnsmasqLease result not acknowledged: {seed_result!r}")
    no_fd_leg(SEED_OP, response, ancdata)
    print(f"SEED_OK={SEED_OP}")
    print(f"SEED_INVOCATION_ID={seed_invocation_id}")
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
printf '%s\n' "$driver_output" | grep -Fxq "INSPECT_OK=$INSPECT_OP" || {
  fail "driver did not prove the hermetic operation $INSPECT_OP"
  exit 1
}
ok "hermetic operation $INSPECT_OP answered end to end (family, roster, 11-row inventory)"
printf '%s\n' "$driver_output" | grep -Fxq "POLL_OK=$POLL_OP" || {
  fail "driver did not prove the hermetic operation $POLL_OP"
  exit 1
}
ok "hermetic operation $POLL_OP answered end to end (empty notifications, no fd leg)"
printf '%s\n' "$driver_output" | grep -Fxq "SEED_OK=$SEED_OP" || {
  fail "driver did not prove the hermetic operation $SEED_OP"
  exit 1
}
ok "hermetic operation $SEED_OP answered end to end (derived lease admission + ack, no fd leg)"
seed_invocation_id=$(printf '%s\n' "$driver_output" | sed -n 's/^SEED_INVOCATION_ID=//p')
[ -n "$seed_invocation_id" ] || {
  fail "driver did not report a seed invocation id"
  exit 1
}

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

# The seed op's nested in-broker kernel leg ("seed-dnsmasq-lease") is the
# broker-side record of the SeedDnsmasqLease invocation (KTD6), keyed on
# its root invocation id - the same continuity the open-pidfd leg proves
# for OpenPidfd.
seed_audit_seen=0
attempts=0
while [ "$attempts" -lt 200 ]; do
  audit_file=""
  for candidate in "$broker_audit_dir"/broker-*.jsonl; do
    [ -f "$candidate" ] && audit_file="$candidate" && break
  done
  if [ -n "$audit_file" ] \
    && grep -Fq "\"operation\":\"$SEED_KERNEL_OP\"" "$audit_file" \
    && grep -Fq "\"invocation_id\":\"$seed_invocation_id\"" "$audit_file"; then
    seed_audit_seen=1
    break
  fi
  attempts=$((attempts + 1))
  sleep 0.2
done
if [ "$seed_audit_seen" != 1 ]; then
  fail "broker audit log missing committed op name '$SEED_KERNEL_OP' for invocation $seed_invocation_id under $broker_audit_dir (expected the U14 chain record of the nested in-broker kernel leg)"
  exit 1
fi
ok "audit continuity: committed op name '$SEED_KERNEL_OP' + invocation id $seed_invocation_id in broker audit log"

log "==> broker-seam-pilot.sh PASS"