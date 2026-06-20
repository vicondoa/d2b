{ config, lib, ... }:

let
  cfg = config.nixling;
  # Use the shared helper from nixos-modules/lib.nix instead of
  # duplicating the formula here. The duplicate was a drift-risk: if
  # minijail-profiles.nix's copy changed, broker setuid target would
  # diverge from system passwd uid and the ownership-matrix bug would
  # silently return.
  nixlingLib = import ./lib.nix { inherit lib; };
  inherit (nixlingLib) stablePrincipalId;
  normalNixosVms = nixlingLib.normalNixosVms cfg.vms;
  qemuMediaVms = nixlingLib.qemuMediaVms cfg.vms;
  waylandProxyVms =
    (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms)
    // qemuMediaVms;
in
{
  # ---------------------------------------------------------------------------
  # Per-VM dedicated system users for GPU + audio sidecars.
  # Each per-VM sidecar runs as its own dedicated user
  # (`nixling-<vm>-{gpu,snd,swtpm}`), NOT the host's Wayland user.
  # The `nixling.site.launcherUsers` list controls who gets the
  # canonical `nixling` lifecycle group.
  # ---------------------------------------------------------------------------
  users.groups = {
    # nixling: members of this group can call the daemon public socket.
    # Add users to it via `nixling.site.launcherUsers`.
    nixling = { };
    # DEPRECATED v1.2: kept as migration tombstone for the
    # nixling-launcher{,s} → nixling rename. No module references the
    # legacy groups; no user is a member. The empty declaration
    # preserves the legacy gid in /etc/group so the
    # nixlingGroupMigration helper can match by numeric gid on direct
    # upgrades. Slated for removal in v1.3 after one release of
    # confirmed clean migration.
    nixling-launcher = { };
  } // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-gpu" { gid = stablePrincipalId "nixling-${name}-gpu"; })
    (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-video" { gid = stablePrincipalId "nixling-${name}-video"; })
    (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-wlproxy" { gid = stablePrincipalId "nixling-${name}-wlproxy"; })
    waylandProxyVms)
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-snd" { gid = stablePrincipalId "nixling-${name}-snd"; })
    (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-swtpm" { gid = stablePrincipalId "nixling-${name}-swtpm"; })
    (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-runner" { gid = stablePrincipalId "nixling-${name}-runner"; })
    normalNixosVms)
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "nixling-${name}-qemu-media" { gid = stablePrincipalId "nixling-${name}-qemu-media"; })
    qemuMediaVms);

  users.users =
    # nixling lifecycle group membership for any user the site
    # declares. We ONLY add the supplementary group — the user
    # must already exist (declared elsewhere in the consumer's
    # NixOS config). The assertions module enforces that.
    (lib.genAttrs cfg.site.launcherUsers (_: {
      extraGroups = [ "nixling" ];
    }))
    // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-gpu" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-gpu";
      group = "nixling-${name}-gpu";
      extraGroups = [ "kvm" "nixling-${name}-runner" ];
      description = "nixling GPU+hypervisor sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-video" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-video";
      group = "nixling-${name}-video";
      description = "nixling video decode sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-wlproxy" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-wlproxy";
      group = "nixling-${name}-wlproxy";
      description = "nixling Wayland filter proxy sidecar for VM ${name}";
    }) waylandProxyVms)
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-snd" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-snd";
      group = "nixling-${name}-snd";
      extraGroups = [ "audio" ];
      description = "nixling audio sidecar for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-swtpm" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-swtpm";
      group = "nixling-${name}-swtpm";
      description = "nixling swtpm emulator for VM ${name}";
    }) (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
    lib.nameValuePair "nixling-${name}-qemu-media" {
      isSystemUser = true;
      uid = stablePrincipalId "nixling-${name}-qemu-media";
      group = "nixling-${name}-qemu-media";
      description = "nixling QEMU media runner for VM ${name}";
    }) qemuMediaVms);
}
