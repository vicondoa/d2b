#!/usr/bin/env bash
# tests/cli-json.sh — build the bash CLI from a synthetic config and
# assert the machine-readable contract for list/status/keys/audit.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# shellcheck source=lib.sh
. "$HERE/lib.sh"

SCRATCH=$(nl_mktemp .cli-json.XXXXXX)

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/cli-json.sh"

mkdir -p "$SCRATCH/keys" "$SCRATCH/home" "$SCRATCH/runtime"
ssh-keygen -q -t ed25519 -N '' -f "$SCRATCH/keys/corp-vm_ed25519" >/dev/null

cat > "$SCRATCH/cli-json-test.nix" <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  lib = flake.inputs.nixpkgs.lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  keysDir = builtins.path {
    path = $SCRATCH/keys;
    name = "nixling-cli-json-keys";
  };
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
          keysDir = keysDir;
        };
        nixling.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        nixling.vms.corp-vm = {
          enable = true;
          env = "work";
          index = 10;
          ssh.user = "alice";
          config = {
            networking.hostName = lib.mkDefault "corp-vm";
            users.users.alice = { isNormalUser = true; uid = 1000; };
          };
        };
      })
    ];
  };
  cliPkg = lib.findFirst
    (p: builtins.pathExists "\${p}/bin/nixling")
    (throw "nixling package not found in systemPackages")
    nixos.config.environment.systemPackages;
  closureLinks = lib.concatStringsSep "\n" (lib.mapAttrsToList (_: closure: ''
    mkdir -p "\$out/\${dirOf closure.relativePath}"
    ln -s \${closure.path} "\$out/\${closure.relativePath}"
  '') nixos.config.nixling._bundle.closures);
in
  nixos.pkgs.runCommand "nixling-cli-json-fixture" { } ''
    mkdir -p "\$out/bin"
    ln -s \${cliPkg}/bin/nixling "\$out/bin/nixling"
    ln -s \${nixos.config.nixling._manifestJsonPath} "\$out/vms.json"
    ln -s \${nixos.config.nixling._bundle.bundle.path} "\$out/bundle.json"
    ln -s \${nixos.config.nixling._bundle.hostJson.path} "\$out/host.json"
    ln -s \${nixos.config.nixling._bundle.processesJson.path} "\$out/processes.json"
    \${closureLinks}
  ''
EOF

FIXTURE_OUT=$(nix-build --no-out-link "$SCRATCH/cli-json-test.nix")
CLI="$FIXTURE_OUT/bin/nixling"
[ -x "$CLI" ] || fail "built CLI missing at $CLI"
ok "built synthetic nixling CLI"

MANIFEST_PATH="$FIXTURE_OUT/vms.json"
BUNDLE_PATH="$FIXTURE_OUT/bundle.json"
STATE_ROOT="$SCRATCH/state"
DAEMON_STATE_DIR="$SCRATCH/daemon-state"
HOST_RUNTIME_PATH="$SCRATCH/missing-host-runtime.json"
PUBLIC_SOCKET="$SCRATCH/missing-public.sock"
BROKER_SOCKET="$SCRATCH/missing-priv.sock"
SYSTEM_STATE_STOPPED="$SCRATCH/system-state-stopped.json"
SYSTEM_STATE_ACTIVE="$SCRATCH/system-state-active.json"
mkdir -p "$STATE_ROOT/corp-vm" "$DAEMON_STATE_DIR"
cat > "$SYSTEM_STATE_STOPPED" <<'EOF'
{"units":{"nixlingd.service":"inactive"},"bridges":{}}
EOF
cat > "$SYSTEM_STATE_ACTIVE" <<'EOF'
{"units":{"nixlingd.service":"active"},"bridges":{}}
EOF

cli_env_stopped=(
  NIXLING_MANIFEST_PATH="$MANIFEST_PATH"
  NIXLING_BUNDLE_PATH="$BUNDLE_PATH"
  NIXLING_STATE_ROOT="$STATE_ROOT"
  NIXLING_DAEMON_STATE_DIR="$DAEMON_STATE_DIR"
  NIXLING_HOST_RUNTIME_PATH="$HOST_RUNTIME_PATH"
  NIXLING_PUBLIC_SOCKET="$PUBLIC_SOCKET"
  NIXLING_BROKER_SOCKET="$BROKER_SOCKET"
  NIXLING_TEST_SYSTEM_STATE_JSON="$SYSTEM_STATE_STOPPED"
  HOME="$SCRATCH/home"
  XDG_RUNTIME_DIR="$SCRATCH/runtime"
)
cli_env_active=(
  NIXLING_MANIFEST_PATH="$MANIFEST_PATH"
  NIXLING_BUNDLE_PATH="$BUNDLE_PATH"
  NIXLING_STATE_ROOT="$STATE_ROOT"
  NIXLING_DAEMON_STATE_DIR="$DAEMON_STATE_DIR"
  NIXLING_HOST_RUNTIME_PATH="$HOST_RUNTIME_PATH"
  NIXLING_PUBLIC_SOCKET="$PUBLIC_SOCKET"
  NIXLING_BROKER_SOCKET="$BROKER_SOCKET"
  NIXLING_TEST_SYSTEM_STATE_JSON="$SYSTEM_STATE_ACTIVE"
  HOME="$SCRATCH/home"
  XDG_RUNTIME_DIR="$SCRATCH/runtime"
)

ln -sfT /nix/store/nixling-current "$SCRATCH/state/corp-vm/current"
ln -sfT /nix/store/nixling-booted "$SCRATCH/state/corp-vm/booted"

env "${cli_env_stopped[@]}" "$CLI" list --json > "$SCRATCH/list.json"
if jq -e '
    type == "array"
    and all(.[];
      ((keys | sort) == ["env","graphics","isNetVm","name","runnerParityOk","staticIp","status","tpm","usbip"])
      and (.name | type == "string")
      and ((.env == null) or (.env | type == "string"))
      and (.graphics | type == "boolean")
      and (.tpm | type == "boolean")
      and (.usbip | type == "boolean")
      and ((.staticIp == null) or (.staticIp | type == "string"))
      and (.status | type == "string")
      and (.isNetVm | type == "boolean")
      and (.runnerParityOk | type == "boolean")
    )
    and any(.[]; .name == "corp-vm" and .env == "work" and .isNetVm == false and .status == "stopped")
    and any(.[]; .name == "sys-work-net" and .isNetVm == true)
  ' "$SCRATCH/list.json" >/dev/null 2>&1; then
  ok "list --json returns the documented VM inventory shape"
else
  fail "list --json output did not match the expected shape"
fi

env "${cli_env_stopped[@]}" "$CLI" status corp-vm --json > "$SCRATCH/status.json"
if jq -e '
    (keys | sort) == ["booted","current","declaredRoles","env","name","pendingRestart","readiness","runnerParity","runtime","services"]
    and .name == "corp-vm"
    and .env == "work"
    and .current == "/nix/store/nixling-current"
    and .booted == "/nix/store/nixling-booted"
    and (.pendingRestart | type == "boolean")
    and .runtime == "unknown"
    and (.declaredRoles == [])
    and (.readiness == [])
    and ((.services | keys | sort) == ["gpu","microvm","nixling","snd","swtpm","video","virtiofsd"])
    and (.services.nixling | type == "string")
    and (.services.microvm | type == "string")
    and (.services.virtiofsd | type == "string")
    and (.services.gpu == null)
    and (.services.video == null)
    and (.services.snd == null)
    and (.services.swtpm == null)
    and (.pendingRestart == false)
    and ((.runnerParity | keys | sort) == ["declaredRunner","runnerParityOk","runnerParityPath"])
    and (.runnerParity.runnerParityOk == true)
  ' "$SCRATCH/status.json" >/dev/null 2>&1; then
  ok "status <vm> --json returns the documented stopped per-VM object"
else
  fail "status <vm> --json output did not match the expected stopped shape"
fi

cat > "$DAEMON_STATE_DIR/pidfd-table.json" <<'EOF'
{"entries":[{"vm":"corp-vm","role":"ch-runner","pid":12345}]}
EOF

env "${cli_env_active[@]}" "$CLI" list --json > "$SCRATCH/list-pending.json"
if jq -e 'any(.[]; .name == "corp-vm" and .status == "pending-restart")' "$SCRATCH/list-pending.json" >/dev/null 2>&1; then
  ok "list --json reports pending-restart when booted != current and the VM is active"
else
  fail "list --json did not surface the pending-restart status"
fi

env "${cli_env_active[@]}" "$CLI" status corp-vm --json > "$SCRATCH/status-pending.json"
if jq -e '
    .name == "corp-vm"
    and .pendingRestart == true
    and .current == "/nix/store/nixling-current"
    and .booted == "/nix/store/nixling-booted"
    and .services.nixling == "active"
    and .services.microvm == "running"
    and .runtime == "unknown"
    and .runnerParity.runnerParityOk == true
  ' "$SCRATCH/status-pending.json" >/dev/null 2>&1; then
  ok "status <vm> --json reports running pending-restart state consistently"
else
  fail "status <vm> --json did not preserve pending-restart/current/booted consistency"
fi

set +e
env "${cli_env_stopped[@]}" "$CLI" keys list --json > "$SCRATCH/keys.json" 2> "$SCRATCH/keys.stderr"
keys_exit=$?
set -e
if [ "$keys_exit" -eq 1 ] \
  && [ ! -s "$SCRATCH/keys.stderr" ] \
  && jq -e '
    ((keys | sort) == ["code","docsAnchor","exitCode","kind","observedState","remediation","whatWasChecked"])
    and .kind == "nixling keys list requires nixlingd"
    and .code == "daemon-down"
    and .exitCode == 1
    and (.whatWasChecked | contains("Daemon connectivity"))
    and (.observedState | contains("nixlingd is unreachable"))
    and (.remediation | contains("Start nixlingd"))
    and .docsAnchor == "docs/reference/error-codes.md#daemon-down"
  ' "$SCRATCH/keys.json" >/dev/null 2>&1; then
  ok "keys list --json returns the structured daemon-down envelope without nixlingd"
else
  fail "keys list --json daemon-down envelope did not match the expected shape"
fi

command -v script >/dev/null 2>&1 || fail "util-linux 'script' is required for the audit TTY check"
set +e
env "${cli_env_stopped[@]}" \
  NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  script -q -e -c "$CLI audit --json" /dev/null > "$SCRATCH/audit.raw" 2> "$SCRATCH/audit.stderr"
audit_exit=$?
set -e
tr -d '\r' < "$SCRATCH/audit.raw" > "$SCRATCH/audit.json"
if [ "$audit_exit" -eq 1 ] \
  && [ ! -s "$SCRATCH/audit.stderr" ] \
  && jq -e '
    ((keys | sort) == ["code","docsAnchor","exitCode","kind","observedState","remediation","whatWasChecked"])
    and .kind == "nixling audit requires nixlingd"
    and .code == "daemon-down"
    and .exitCode == 1
    and (.whatWasChecked | contains("Daemon connectivity"))
    and (.observedState | contains("nixlingd is unreachable"))
    and (.remediation | contains("Start nixlingd"))
    and .docsAnchor == "docs/reference/error-codes.md#daemon-down"
  ' "$SCRATCH/audit.json" >/dev/null 2>&1; then
  ok "audit --json stays JSON on a TTY and returns the structured daemon-down envelope"
else
  fail "audit --json did not preserve the daemon-down JSON envelope on a TTY"
fi

log "==> cli-json OK"
