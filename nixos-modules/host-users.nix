{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  inherit (d2bLib) stablePrincipalId;
  declaredVms = cfg.vms or { };
  normalNixosVms = d2bLib.normalNixosVms declaredVms;
  qemuMediaVms = d2bLib.qemuMediaVms declaredVms;
  waylandProxyVms = (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms) // qemuMediaVms;
in
{
  imports = [
    ./realm-users.nix
    ./realm-access.nix
  ];

  users.groups = {
    # This remains the sole local-root lifecycle admission group.
    d2b = { };
  }
  # Workload principals remain after the realm principals so a later workload
  # process cutover can remove this suffix without disturbing realm admission.
  // (lib.mapAttrs' (
    name: _: lib.nameValuePair "d2b-${name}-gpu" { gid = stablePrincipalId "d2b-${name}-gpu"; }
  ) (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms))
  // (lib.mapAttrs' (
    name: _: lib.nameValuePair "d2b-${name}-video" { gid = stablePrincipalId "d2b-${name}-video"; }
  ) (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms))
  // (lib.mapAttrs' (
    name: _: lib.nameValuePair "d2b-${name}-wlproxy" { gid = stablePrincipalId "d2b-${name}-wlproxy"; }
  ) waylandProxyVms)
  // (lib.mapAttrs' (
    name: _: lib.nameValuePair "d2b-${name}-snd" { gid = stablePrincipalId "d2b-${name}-snd"; }
  ) (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms))
  // (lib.mapAttrs' (
    name: _: lib.nameValuePair "d2b-${name}-swtpm" { gid = stablePrincipalId "d2b-${name}-swtpm"; }
  ) (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms))
  // (lib.mapAttrs' (
    name: _: lib.nameValuePair "d2b-${name}-runner" { gid = stablePrincipalId "d2b-${name}-runner"; }
  ) normalNixosVms)
  // (lib.mapAttrs' (
    name: _:
    lib.nameValuePair "d2b-${name}-qemu-media" { gid = stablePrincipalId "d2b-${name}-qemu-media"; }
  ) qemuMediaVms);

  users.users = lib.mkMerge [
    (lib.genAttrs (cfg.site.launcherUsers or [ ]) (_: {
      extraGroups = [ "d2b" ];
    }))
    (lib.mapAttrs' (
      name: _:
      lib.nameValuePair "d2b-${name}-gpu" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-gpu";
        group = "d2b-${name}-gpu";
        extraGroups = [
          "kvm"
          "d2b-${name}-runner"
        ];
        description = "d2b GPU+hypervisor sidecar for VM ${name}";
      }
    ) (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms))
    (lib.mapAttrs' (
      name: _:
      lib.nameValuePair "d2b-${name}-video" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-video";
        group = "d2b-${name}-video";
        description = "d2b video decode sidecar for VM ${name}";
      }
    ) (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms))
    (lib.mapAttrs' (
      name: _:
      lib.nameValuePair "d2b-${name}-wlproxy" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-wlproxy";
        group = "d2b-${name}-wlproxy";
        description = "d2b Wayland proxy sidecar for VM ${name}";
      }
    ) waylandProxyVms)
    (lib.mapAttrs' (
      name: _:
      lib.nameValuePair "d2b-${name}-snd" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-snd";
        group = "d2b-${name}-snd";
        extraGroups = [ "audio" ];
        description = "d2b audio sidecar for VM ${name}";
      }
    ) (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms))
    (lib.mapAttrs' (
      name: _:
      lib.nameValuePair "d2b-${name}-swtpm" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-swtpm";
        group = "d2b-${name}-swtpm";
        description = "d2b swtpm emulator for VM ${name}";
      }
    ) (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms))
    (lib.mapAttrs' (
      name: _:
      lib.nameValuePair "d2b-${name}-qemu-media" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-qemu-media";
        group = "d2b-${name}-qemu-media";
        description = "d2b QEMU media runner for VM ${name}";
      }
    ) qemuMediaVms)
  ];
}
