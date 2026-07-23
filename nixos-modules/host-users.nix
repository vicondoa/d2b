{ config, lib, ... }:

let
  cfg = config.d2b;
  # Use the shared helper from nixos-modules/lib.nix instead of
  # duplicating the formula here. The duplicate was a drift-risk: if
  # minijail-profiles.nix's copy changed, broker setuid target would
  # diverge from system passwd uid and the ownership-matrix bug would
  # silently return.
  d2bLib = import ./lib.nix { inherit lib; };
  inherit (d2bLib) stablePrincipalId;
  normalNixosVms = d2bLib.normalNixosVms cfg.vms;
  qemuMediaVms = d2bLib.qemuMediaVms cfg.vms;
  hostLocalRealms =
    lib.filter (realm: realm.placement == "host-local") cfg._index.realms.enabledList;
  hostLocalRealmAllowedUsers =
    lib.unique (lib.concatMap (realm: realm.allowedUsers) hostLocalRealms);
  hostAccessUsers =
    lib.unique (cfg.site.launcherUsers ++ hostLocalRealmAllowedUsers);
  realmSocketGroupsForUser = user:
    map
      (realm: realm.controller.daemon.publicSocketGroup)
      (lib.filter (realm: builtins.elem user realm.allowedUsers) hostLocalRealms);
  hostAccessGroupsForUser = user:
    lib.unique (
      lib.optional (builtins.elem user cfg.site.launcherUsers) "d2b"
      ++ realmSocketGroupsForUser user
    );
  waylandProxyVms =
    (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms)
    // qemuMediaVms;
in
{
  # ---------------------------------------------------------------------------
  # Per-VM dedicated system users for GPU + audio sidecars.
  # Each per-VM sidecar runs as its own dedicated user
  # (`d2b-<vm>-{gpu,snd,swtpm}`), NOT the host's Wayland user.
  # The `d2b.site.launcherUsers` list controls who gets the
  # canonical `d2b` lifecycle group.
  # ---------------------------------------------------------------------------
  users.groups = {
    # d2b: members of this group can call the daemon public socket.
    # Add users to it via `d2b.site.launcherUsers`.
    d2b = { };
    # DEPRECATED v1.2: kept as migration tombstone for the
    # d2b-launcher{,s} → d2b rename. No module references the
    # legacy groups; no user is a member. The empty declaration
    # preserves the legacy gid in /etc/group so the
    # d2bGroupMigration helper can match by numeric gid on direct
    # upgrades. Slated for removal in v1.3 after one release of
    # confirmed clean migration.
    d2b-launcher = { };
  } // (lib.listToAttrs (map
    (realm:
      lib.nameValuePair realm.controller.daemon.group {
        gid = stablePrincipalId realm.controller.daemon.group;
      })
    hostLocalRealms))
  // (lib.listToAttrs (map
    (realm:
      lib.nameValuePair realm.controller.daemon.publicSocketGroup {
        gid = stablePrincipalId realm.controller.daemon.publicSocketGroup;
      })
    hostLocalRealms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-gpu" { gid = stablePrincipalId "d2b-${name}-gpu"; })
    (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-video" { gid = stablePrincipalId "d2b-${name}-video"; })
    (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-wlproxy" { gid = stablePrincipalId "d2b-${name}-wlproxy"; })
    waylandProxyVms)
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-snd" { gid = stablePrincipalId "d2b-${name}-snd"; })
    (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-swtpm" { gid = stablePrincipalId "d2b-${name}-swtpm"; })
    (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms))
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-runner" { gid = stablePrincipalId "d2b-${name}-runner"; })
    normalNixosVms)
  // (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-qemu-media" { gid = stablePrincipalId "d2b-${name}-qemu-media"; })
    qemuMediaVms);

  users.users = lib.mkMerge [
    # d2b lifecycle group membership for any user the site
    # declares. We ONLY add the supplementary group — the user
    # must already exist (declared elsewhere in the consumer's
    # NixOS config). The assertions module enforces that.
    #
    # Realm socket groups are generated in the same fragment as the
    # lifecycle group so overlapping users do not receive duplicate
    # `d2b` entries from multiple internal definitions.
    (lib.genAttrs hostAccessUsers (user: {
      extraGroups = hostAccessGroupsForUser user;
    }))
    (lib.listToAttrs (map
      (realm:
        lib.nameValuePair realm.controller.daemon.user {
          isSystemUser = true;
          uid = stablePrincipalId realm.controller.daemon.user;
          group = realm.controller.daemon.group;
          extraGroups = [ realm.controller.daemon.publicSocketGroup "d2bd" ];
          description = "d2b realm daemon user for ${realm.path}";
        })
      hostLocalRealms))
    (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-gpu" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-gpu";
        group = "d2b-${name}-gpu";
        extraGroups = [ "kvm" "d2b-${name}-runner" ];
        description = "d2b GPU+hypervisor sidecar for VM ${name}";
      }) (lib.filterAttrs (_: vm: vm.graphics.enable) normalNixosVms))
    (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-video" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-video";
        group = "d2b-${name}-video";
        description = "d2b video decode sidecar for VM ${name}";
      }) (lib.filterAttrs (_: vm: vm.graphics.enable && vm.graphics.videoSidecar) normalNixosVms))
    (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-wlproxy" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-wlproxy";
        group = "d2b-${name}-wlproxy";
        description = "d2b Wayland proxy sidecar for VM ${name}";
      }) waylandProxyVms)
    (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-snd" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-snd";
        group = "d2b-${name}-snd";
        extraGroups = [ "audio" ];
        description = "d2b audio sidecar for VM ${name}";
      }) (lib.filterAttrs (_: vm: vm.audio.enable) normalNixosVms))
    (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-swtpm" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-swtpm";
        group = "d2b-${name}-swtpm";
        description = "d2b swtpm emulator for VM ${name}";
      }) (lib.filterAttrs (_: vm: vm.tpm.enable) normalNixosVms))
    (lib.mapAttrs' (name: _:
      lib.nameValuePair "d2b-${name}-qemu-media" {
        isSystemUser = true;
        uid = stablePrincipalId "d2b-${name}-qemu-media";
        group = "d2b-${name}-qemu-media";
        description = "d2b QEMU media runner for VM ${name}";
      }) qemuMediaVms)
  ];
}
