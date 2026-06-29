# shellcheck shell=bash

set -euo pipefail

HERE=$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")
ROOT=${ROOT:-$(dirname "$HERE")}
export ROOT
export FLAKE=${FLAKE:-$ROOT}
export D2B_LOG=${D2B_LOG:-$ROOT/.cli-rust-native.log}
export D2B_STATIC_CACHE=${D2B_STATIC_CACHE:-$ROOT/.cli-rust-native-cache}
mkdir -p "$D2B_STATIC_CACHE"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

d2b_activate_rust_toolchain_path || true

d2b_cli_flake_source_root() {
  # The cli-rust-native eval sites resolve the framework flake as
  # `git+file://$(d2b_cli_flake_source_root)`. Return $ROOT directly:
  # git+file fetches only git-tracked files straight from the repo
  # (target/ is gitignored), matching `nix flake check` semantics, so
  # there is no working-tree copy. (Earlier this pre-copied a clean
  # tracked-only tree for the `path:` fetcher; git+file makes that
  # indirection unnecessary, and a copied tree is not a git repo so
  # git+file cannot fetch from it.)
  printf '%s\n' "$ROOT"
}

d2b_cli_toolchain_shell() {
  if [ -n "${D2B_RUST_TOOLCHAIN_PATH:-}" ]; then
    env PATH="$PATH" bash -lc "$*"
  else
    nix shell --quiet --inputs-from "$ROOT" \
      nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
      --command bash -lc "$*"
  fi
}

d2b_cli_native_bin() {
  local bin workspace_target_dir
  bin=$(d2b_cargo_bin_path workspace d2b) || return 1
  if [ ! -x "$bin" ]; then
    workspace_target_dir=$(d2b_cargo_target_dir workspace) || return 1
    d2b_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$workspace_target_dir' cargo build -q --manifest-path '$ROOT/packages/Cargo.toml' -p d2b"
  fi
  printf '%s\n' "$bin"
}

d2b_daemon_native_bin() {
  local bin workspace_target_dir
  bin=$(d2b_cargo_bin_path workspace d2bd) || return 1
  if [ ! -x "$bin" ]; then
    workspace_target_dir=$(d2b_cargo_target_dir workspace) || return 1
    d2b_cli_toolchain_shell "cd '$ROOT/packages' && CARGO_TARGET_DIR='$workspace_target_dir' cargo build -q --manifest-path '$ROOT/packages/Cargo.toml' -p d2bd"
  fi
  printf '%s\n' "$bin"
}

_d2b_cli_reap_repo_sockets() {
  local target_dir="$ROOT/packages/d2bd/target"
  [ -d "$target_dir" ] || return 0
  find "$target_dir" -maxdepth 1 -type s -name '*.sock' -exec rm -f -- {} +
}

_d2b_cli_smoke_eval_raw() {
  local expr="$1" out="$2"
  local modules flake_root
  modules=$(_d2b_smoke_config_modules)
  flake_root=$(d2b_cli_flake_source_root)
  : > "$out.stderr"
  if ! nix eval --impure --raw --expr "
    let
      flake = builtins.getFlake \"git+file://$flake_root\";
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          $modules
        ];
      };
    in $expr
  " > "$out.tmp" 2> "$out.stderr"; then
    head -40 "$out.stderr" >&2 || true
    rm -f "$out.tmp"
    return 1
  fi
  mv -f "$out.tmp" "$out"
}

_d2b_cli_smoke_eval_value() {
  local expr="$1" out="$D2B_STATIC_CACHE/.cli-smoke-value"
  _d2b_cli_smoke_eval_raw "$expr" "$out"
  cat "$out"
}

d2b_cli_smoke_bundle_tree() {
  local tree="$D2B_STATIC_CACHE/cli-bundle-tree"
  local lock_file="$D2B_STATIC_CACHE/cli-bundle-tree.lock"
  if [ -f "$tree/.ready" ] \
    && [ -f "$tree/vms.json" ] \
    && [ -f "$tree/bundle.json" ] \
    && [ -f "$tree/host.json" ] \
    && [ -f "$tree/processes.json" ]; then
    printf '%s\n' "$tree"
    return 0
  fi

  mkdir -p "$D2B_STATIC_CACHE"
  exec {tree_lock_fd}>>"$lock_file"
  flock -x "$tree_lock_fd"
  if [ -f "$tree/.ready" ] \
    && [ -f "$tree/vms.json" ] \
    && [ -f "$tree/bundle.json" ] \
    && [ -f "$tree/host.json" ] \
    && [ -f "$tree/processes.json" ]; then
    flock -u "$tree_lock_fd"
    exec {tree_lock_fd}>&-
    printf '%s\n' "$tree"
    return 0
  fi

  rm -rf -- "$tree"
  mkdir -p "$tree/closures"
  cp "$(d2b_smoke_vms_json)" "$tree/vms.json"
  local modules flake_root bundle_path
  modules=$(_d2b_smoke_config_modules)
  flake_root=$(d2b_cli_flake_source_root)
  bundle_path=$(nix build --impure --no-link --print-out-paths --expr "
    let
      flake = builtins.getFlake \"git+file://$flake_root\";
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          $modules
        ];
      };
    in nixos.config.d2b._bundle.bundle.path
  ")
  cp "$bundle_path" "$tree/bundle.json"
  _d2b_cli_smoke_eval_raw 'nixos.config.d2b._bundle.hostJson.jsonText' "$tree/host.json"
  _d2b_cli_smoke_eval_raw 'nixos.config.d2b._bundle.processesJson.jsonText' "$tree/processes.json"

  # Each closures/<vm>.json artifact is a runCommand-emitted derivation
  # exposed via `d2b._bundle.closures.<vm>.path`. Evaluating `.path`
  # alone gives the future output store path; we must REALISE it to copy
  # the file. `nix build --impure --expr ... --no-link --print-out-paths`
  # both instantiates and builds the derivation, returning the realised
  # output path. Bare `nix eval .path` + `cp` fails post-GC because the
  # output path is gone and there is no .drv left to rebuild from.
  while IFS= read -r vm; do
    [ -n "$vm" ] || continue
    local path
    path=$(nix build --impure --no-link --print-out-paths --expr "
      let
        flake = builtins.getFlake \"git+file://$flake_root\";
        nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
        nixos = nixosSystem {
          system = builtins.currentSystem;
          modules = [
            $modules
          ];
        };
      in (builtins.getAttr \"$vm\" nixos.config.d2b._bundle.closures).path
    ")
    cp "$path" "$tree/closures/$vm.json"
  done < <(jq -r 'keys[] | select(startswith("_") | not)' "$tree/vms.json")

  : > "$tree/.ready"
  flock -u "$tree_lock_fd"
  exec {tree_lock_fd}>&-
  printf '%s\n' "$tree"
}

d2b_cli_smoke_bundle_tree_runner_drift() {
  local base tree
  base=$(d2b_cli_smoke_bundle_tree)
  tree="$D2B_STATIC_CACHE/cli-bundle-tree-runner-drift"
  if [ ! -f "$tree/.ready" ]; then
    rm -rf "$tree"
    cp -R "$base" "$tree"
    jq '.runnerParityOk = false | .runnerParityPath = (.runnerParityPath + "-drift")' \
      "$tree/closures/corp-vm.json" > "$tree/closures/corp-vm.json.tmp"
    mv -f "$tree/closures/corp-vm.json.tmp" "$tree/closures/corp-vm.json"
    : > "$tree/.ready"
  fi
  printf '%s\n' "$tree"
}

d2b_legacy_cli_bin() {
  local cache="$D2B_STATIC_CACHE/legacy-cli.path"
  local keys_dir="$D2B_STATIC_CACHE/legacy-cli-keys"
  local expr="$D2B_STATIC_CACHE/legacy-cli.nix"
  local cli_out

  if [ -f "$cache" ] && [ -x "$(cat "$cache")" ]; then
    cat "$cache"
    return 0
  fi

  mkdir -p "$keys_dir"
  if [ ! -f "$keys_dir/corp-vm_ed25519" ]; then
    ssh-keygen -q -t ed25519 -N '' -f "$keys_dir/corp-vm_ed25519" >/dev/null
  fi

  local flake_root
  flake_root=$(d2b_cli_flake_source_root)

  cat > "$expr" <<EOF2
let
  flake = builtins.getFlake "git+file://$flake_root";
  lib = flake.inputs.nixpkgs.lib;
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  keysDir = builtins.path {
    path = $keys_dir;
    name = "d2b-cli-json-keys";
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
        d2b.site = {
          waylandUser = "alice";
          launcherUsers = [ "alice" ];
          yubikey.enable = false;
          keysDir = keysDir;
        };
        d2b.envs.work = {
          lanSubnet = "10.20.0.0/24";
          uplinkSubnet = "192.0.2.0/30";
        };
        d2b.vms.corp-vm = {
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
    (p: (p.pname or "") == "d2b")
    (throw "d2b CLI package not found in systemPackages")
    nixos.config.environment.systemPackages;
in
  cliPkg
EOF2
  cli_out=$(nix-build --no-out-link "$expr")
  printf '%s\n' "$cli_out/bin/d2b" > "$cache"
  cat "$cache"
}

d2b_write_system_state_fixture() {
  local out="$1"
  cat > "$out" <<'EOF2'
{
  "units": {
    "d2b@corp-vm.service": "inactive",
    "microvm@corp-vm.service": "inactive",
    "d2b@sys-work-net.service": "active",
    "microvm@sys-work-net.service": "active"
  },
  "bridges": {
    "br-work-lan": {
      "state": "UP",
      "admin": "up",
      "expectedCarrier": "NO-CARRIER",
      "result": "ok"
    },
    "br-work-up": {
      "state": "UP",
      "admin": "up",
      "expectedCarrier": "UP",
      "result": "ok"
    }
  }
}
EOF2
}

d2b_write_system_state_fixture_pending() {
  local out="$1"
  cat > "$out" <<'EOF2'
{
  "units": {
    "d2b@corp-vm.service": "active",
    "microvm@corp-vm.service": "active"
  },
  "bridges": {
    "br-work-lan": {
      "state": "UP",
      "admin": "up",
      "expectedCarrier": "NO-CARRIER",
      "result": "ok"
    },
    "br-work-up": {
      "state": "UP",
      "admin": "up",
      "expectedCarrier": "UP",
      "result": "ok"
    }
  }
}
EOF2
}

_d2b_host_check_sysctls_json() {
  local bundle_root="$1"
  # host_check now enforces
  # kernelModules[].sysctls when the module is loaded/built-in. The
  # passing fixtures must therefore include every declared
  # kernelModules[].sysctls (`key=value`) under the dotted key so the
  # fixture-backed probe returns the documented value.
  jq -c '
    (.environments
     | map(.ipv6Sysctls[])
     | reduce .[] as $entry ({};
         . + {
           ($entry.ifName + ".disable_ipv6"): ($entry.disableIpv6 | tostring),
           ($entry.ifName + ".accept_ra"): ($entry.acceptRa | tostring),
           ($entry.ifName + ".autoconf"): ($entry.autoconf | tostring),
           ($entry.ifName + ".addr_gen_mode"): ($entry.addrGenMode | tostring),
           ($entry.ifName + ".arp_ignore"): ($entry.arpIgnore | tostring)
         }))
    +
    (.kernelModules
     | map(.sysctls[]?)
     | reduce .[] as $entry ({};
         ($entry | split("=")) as $kv
         | . + { ($kv[0]): ($kv[1:] | join("=")) }))
  ' "$bundle_root/host.json"
}

d2b_write_host_check_fixture_pass() {
  local out="$1" bundle_root="$2" sysctls
  sysctls=$(_d2b_host_check_sysctls_json "$bundle_root")
  cat > "$out" <<EOF2
{
  "kernelRelease": "6.8.12-d2b",
  "cgroupV2Present": true,
  "cpuVendor": "intel",
  "loadedModules": [
    "kvm", "kvm_intel", "tun", "vhost_net", "fuse", "nf_tables", "bridge",
    "br_netfilter", "i915", "amdgpu", "nvidia", "nvidia_modeset", "nvidia_uvm",
    "nvidia_drm", "usbip_host"
  ],
  "nftHasD2bTable": true,
  "firewalldActive": false,
  "ufwActive": false,
  "sysctls": $sysctls
}
EOF2
}

d2b_write_host_check_fixture_warn() {
  local out="$1" bundle_root="$2" sysctls
  sysctls=$(_d2b_host_check_sysctls_json "$bundle_root")
  cat > "$out" <<EOF2
{
  "kernelRelease": "6.8.12-d2b",
  "cgroupV2Present": true,
  "cpuVendor": "intel",
  "loadedModules": [
    "kvm", "kvm_intel", "tun", "vhost_net", "fuse", "nf_tables", "bridge",
    "br_netfilter", "i915", "amdgpu", "nvidia", "nvidia_modeset", "nvidia_uvm",
    "nvidia_drm", "usbip_host"
  ],
  "nftHasD2bTable": false,
  "firewalldActive": false,
  "ufwActive": false,
  "sysctls": $sysctls
}
EOF2
}

d2b_write_host_check_fixture_fail() {
  local out="$1" bundle_root="$2" sysctls
  sysctls=$(_d2b_host_check_sysctls_json "$bundle_root")
  cat > "$out" <<EOF2
{
  "kernelRelease": "6.5.0-d2b",
  "cgroupV2Present": true,
  "cpuVendor": "intel",
  "loadedModules": [
    "kvm", "kvm_intel", "tun", "vhost_net", "fuse", "nf_tables", "bridge",
    "br_netfilter", "i915", "amdgpu", "nvidia", "nvidia_modeset", "nvidia_uvm",
    "nvidia_drm", "usbip_host"
  ],
  "nftHasD2bTable": true,
  "firewalldActive": false,
  "ufwActive": false,
  "sysctls": $sysctls
}
EOF2
}

d2b_write_auth_status_fixture() {
  local out="$1" role="$2"
  case "$role" in
    launcher)
      cat > "$out" <<'EOF2'
{
  "publicReachable": true,
  "publicVersion": "0.4.0-test",
  "brokerReachable": false,
  "brokerVersion": null
}
EOF2
      ;;
    none)
      cat > "$out" <<'EOF2'
{
  "publicReachable": false,
  "publicVersion": null,
  "brokerReachable": false,
  "brokerVersion": null
}
EOF2
      ;;
    admin)
      cat > "$out" <<'EOF2'
{
  "publicReachable": true,
  "publicVersion": "0.4.0-test",
  "brokerReachable": false,
  "brokerVersion": null
}
EOF2
      ;;
    *)
      printf 'unknown auth fixture role: %s\n' "$role" >&2
      return 1
      ;;
  esac
}

d2b_assert_json_schema() {
  local schema="$1" json_file="$2"
  nix shell --quiet --inputs-from "$ROOT" nixpkgs#check-jsonschema --command bash -lc \
    "check-jsonschema --schemafile '$schema' '$json_file'" >/dev/null
}
