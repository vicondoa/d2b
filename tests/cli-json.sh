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
    (p: ((p.pname or "") == "nixling") || lib.hasPrefix "nixling" (p.name or ""))
    (throw "nixling package not found in systemPackages")
    nixos.config.environment.systemPackages;
in
  cliPkg
EOF

CLI_OUT=$(nix-build --no-out-link "$SCRATCH/cli-json-test.nix")
CLI="$CLI_OUT/bin/nixling"
[ -x "$CLI" ] || fail "built CLI missing at $CLI"
ok "built synthetic nixling CLI"

mkdir -p "$SCRATCH/state/corp-vm" "$SCRATCH/fakebin"
ln -sfT /nix/store/nixling-current "$SCRATCH/state/corp-vm/current"
ln -sfT /nix/store/nixling-booted "$SCRATCH/state/corp-vm/booted"
cat > "$SCRATCH/fakebin/systemctl" <<'EOF'
#!/usr/bin/env bash
set -eu
if [ "${1:-}" = "--no-pager" ]; then shift; fi
[ "${1:-}" = "is-active" ] || exit 1
shift
quiet=false
if [ "${1:-}" = "--quiet" ]; then
  quiet=true
  shift
fi
unit="${1:-}"
case "$unit" in
  nixling@corp-vm.service|microvm@corp-vm.service)
    if [ "$quiet" = true ]; then exit 0; fi
    printf 'active\n'
    ;;
  *)
    if [ "$quiet" = true ]; then exit 3; fi
    printf 'inactive\n'
    ;;
esac
EOF
chmod +x "$SCRATCH/fakebin/systemctl"
cp "$CLI" "$SCRATCH/nixling-pending"
sed -i "s|systemctl|$SCRATCH/fakebin/systemctl|g" "$SCRATCH/nixling-pending"
sed -i "s|^[[:space:]]*STATE_ROOT=.*|      STATE_ROOT=$SCRATCH/state|" "$SCRATCH/nixling-pending"

HOME="$SCRATCH/home" XDG_RUNTIME_DIR="$SCRATCH/runtime" \
  "$CLI" list --json > "$SCRATCH/list.json"
if jq -e '
    type == "array"
    and all(.[];
      ((keys | sort) == ["env","graphics","isNetVm","name","staticIp","status","tpm","usbip"])
      and (.name | type == "string")
      and ((.env == null) or (.env | type == "string"))
      and (.graphics | type == "boolean")
      and (.tpm | type == "boolean")
      and (.usbip | type == "boolean")
      and ((.staticIp == null) or (.staticIp | type == "string"))
      and (.status | type == "string")
      and (.isNetVm | type == "boolean")
    )
    and any(.[]; .name == "corp-vm" and .env == "work" and .isNetVm == false and .status == "stopped")
    and any(.[]; .name == "sys-work-net" and .isNetVm == true)
  ' "$SCRATCH/list.json" >/dev/null 2>&1; then
  ok "list --json returns the documented VM inventory shape"
else
  fail "list --json output did not match the expected shape"
fi

HOME="$SCRATCH/home" XDG_RUNTIME_DIR="$SCRATCH/runtime" \
  "$CLI" status corp-vm --json > "$SCRATCH/status.json"
if jq -e '
    (keys | sort) == ["booted","current","name","pendingRestart","services"]
    and .name == "corp-vm"
    and ((.current == null) or (.current | type == "string"))
    and ((.booted == null) or (.booted | type == "string"))
    and (.pendingRestart | type == "boolean")
    and ((.services | keys | sort) == ["gpu","microvm","nixling","snd","swtpm","virtiofsd"])
    and (.services.nixling | type == "string")
    and (.services.microvm | type == "string")
    and (.services.virtiofsd | type == "string")
    and (.services.gpu == null)
    and (.services.snd == null)
    and (.services.swtpm == null)
    and (.pendingRestart == false)
  ' "$SCRATCH/status.json" >/dev/null 2>&1; then
  ok "status <vm> --json returns the documented stopped per-VM object"
else
  fail "status <vm> --json output did not match the expected stopped shape"
fi

PATH="$SCRATCH/fakebin:$PATH" HOME="$SCRATCH/home" XDG_RUNTIME_DIR="$SCRATCH/runtime" \
  "$SCRATCH/nixling-pending" list --json > "$SCRATCH/list-pending.json"
if jq -e 'any(.[]; .name == "corp-vm" and .status == "pending-restart")' "$SCRATCH/list-pending.json" >/dev/null 2>&1; then
  ok "list --json reports pending-restart when booted != current and the VM is active"
else
  fail "list --json did not surface the pending-restart status"
fi

PATH="$SCRATCH/fakebin:$PATH" HOME="$SCRATCH/home" XDG_RUNTIME_DIR="$SCRATCH/runtime" \
  "$SCRATCH/nixling-pending" status corp-vm --json > "$SCRATCH/status-pending.json"
if jq -e '
    .name == "corp-vm"
    and .pendingRestart == true
    and .current == "/nix/store/nixling-current"
    and .booted == "/nix/store/nixling-booted"
    and .services.nixling == "active"
    and .services.microvm == "active"
  ' "$SCRATCH/status-pending.json" >/dev/null 2>&1; then
  ok "status <vm> --json reports running pending-restart state consistently"
else
  fail "status <vm> --json did not preserve pending-restart/current/booted consistency"
fi

HOME="$SCRATCH/home" XDG_RUNTIME_DIR="$SCRATCH/runtime" \
  "$CLI" keys list --json > "$SCRATCH/keys.json"
if jq -e '
    type == "array"
    and all(.[];
      ((keys | sort) == ["fingerprint","mtime","publicKey","status","vm"])
      and (.vm | type == "string")
      and (.fingerprint | type == "string")
      and (.publicKey | type == "string")
      and (.mtime | type == "string")
      and (.status | type == "string")
    )
    and any(.[]; .vm == "corp-vm" and .status == "present" and (.fingerprint | startswith("SHA256:")) and (.publicKey | endswith("corp-vm_ed25519.pub")))
    and any(.[]; .vm == "sys-work-net" and .status == "missing")
  ' "$SCRATCH/keys.json" >/dev/null 2>&1; then
  ok "keys list --json returns structured key inventory"
else
  fail "keys list --json output did not match the expected shape"
fi

command -v script >/dev/null 2>&1 || fail "util-linux 'script' is required for the audit TTY check"
HOME="$SCRATCH/home" XDG_RUNTIME_DIR="$SCRATCH/runtime" \
  NIXLING_AUDIT_TESTMODE_KVM_MODE=660 \
  script -q -e -c "$CLI audit --json" /dev/null > "$SCRATCH/audit.raw" 2> "$SCRATCH/audit.stderr"
tr -d '\r' < "$SCRATCH/audit.raw" > "$SCRATCH/audit.json"
if jq -e '
    (keys | sort) == [
      "autoUpgrade_commits_lock",
      "bridge_isolation",
      "ch_crosvm_pair_ok",
      "ch_version",
      "crosvm_rev",
      "fail2ban_active",
      "kvm_dev_mode",
      "seccomp_rev",
      "sidecars_per_vm",
      "ssh",
      "store_delivery",
      "usbipd_per_env_isolation",
      "virtiofsd",
      "wayland_user_in_kvm"
    ]
    and (.kvm_dev_mode | type == "string")
    and (.wayland_user_in_kvm | type == "boolean")
    and (.store_delivery | type == "object")
    and (.virtiofsd | type == "object")
    and (.ssh | type == "object")
    and (.bridge_isolation | type == "object")
    and (.autoUpgrade_commits_lock | type == "boolean")
    and (.ch_version | type == "string")
    and (.crosvm_rev | type == "string")
    and (.seccomp_rev | type == "string")
    and (.ch_crosvm_pair_ok | type == "boolean")
    and (.fail2ban_active | type == "boolean")
    and (.sidecars_per_vm | type == "object")
    and (.usbipd_per_env_isolation | type == "object")
  ' "$SCRATCH/audit.json" >/dev/null 2>&1; then
  ok "audit --json stays JSON on a TTY and preserves the documented schema"
else
  fail "audit --json emitted non-JSON output or regressed its schema on a TTY"
fi

log "==> cli-json OK"
