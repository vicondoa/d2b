{ config, lib, ... }:

let
  cfg = config.nixling;
  # v1.1.2-final-R1 (panel-software HIGH): use the shared helper
  # from nixos-modules/lib.nix instead of duplicating the formula
  # here. The duplicate was a drift-risk: if minijail-profiles.nix's
  # copy changed, broker setuid target would diverge from system
  # passwd uid and the fu35 ownership-matrix bug would silently
  # return.
  nixlingLib = import ./lib.nix { inherit lib; };
  inherit (nixlingLib) stablePrincipalId;
in
{
  # ---------------------------------------------------------------------------
  # P4 C3/H5: Per-VM dedicated system users for GPU + audio sidecars.
  # Each per-VM sidecar runs as its own dedicated user
  # (`nixling-<vm>-{gpu,snd,swtpm}`), NOT the host's Wayland user.
  # The `nixling.site.launcherUsers` list controls who gets the
  # `nixling-launcher` group (and thus the polkit grant on the
  # framework's units).
  # ---------------------------------------------------------------------------
  users.groups = {
    # nixling-launcher: members of this group get the polkit grant
    # to start/stop/restart the framework's own systemd units. Add
    # users to it via `nixling.site.launcherUsers`.
    nixling-launcher = { };
  } // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-gpu" { gid = stablePrincipalId "nixling-${name}-gpu"; })
    (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-snd" { gid = stablePrincipalId "nixling-${name}-snd"; })
    (lib.filterAttrs (_: vm: vm.enable && vm.audio.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-swtpm" { gid = stablePrincipalId "nixling-${name}-swtpm"; })
    (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-runner" { gid = stablePrincipalId "nixling-${name}-runner"; })
    (lib.filterAttrs (_: vm: vm.enable) cfg.vms));

  users.users =
    # nixling-launcher group membership for any user the site
    # declares. We ONLY add the supplementary group — the user
    # must already exist (declared elsewhere in the consumer's
    # NixOS config). The assertions module enforces that.
    (lib.genAttrs cfg.site.launcherUsers (_: {
      extraGroups = [ "nixling-launcher" ];
    }))
    // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-gpu" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-gpu";
      group = "nixling-${name}-gpu";
      extraGroups = [ "kvm" "nixling-${name}-runner" ];
      description = "nixling GPU+hypervisor sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.enable && vm.graphics.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-snd" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-snd";
      group = "nixling-${name}-snd";
      extraGroups = [ "audio" ];
      description = "nixling audio sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.enable && vm.audio.enable) cfg.vms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-swtpm" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-swtpm";
      group = "nixling-${name}-swtpm";
      description = "nixling swtpm emulator for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.enable && vm.tpm.enable) cfg.vms));
}
