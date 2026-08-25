{ config, lib, pkgs, d2bHostTools, d2bHostToolOverrides ? null, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };
  sourcePackage = d2bHostTools.unsafeLocalHelper;
  helperPackage = d2bLib.selectHostToolPackage {
    overrides = d2bHostToolOverrides;
    key = "unsafeLocalHelper";
    fallback =
      if prebuilt != null && prebuilt ? "d2b-unsafe-local-helper"
      then prebuilt."d2b-unsafe-local-helper"
      else sourcePackage;
  };
  unsafeLocalRealms = lib.filter
    (realm:
      lib.any
        (workload: workload.enable && workload.kind == "unsafe-local")
        realm.workloads)
    cfg._index.realms.enabledList;
  eligibleUsers = lib.sort lib.lessThan
    (lib.unique (lib.concatMap (realm: realm.allowedUsers) unsafeLocalRealms));
in
{
  config = lib.mkIf cfg.daemonExperimental.enable {
    users.groups.d2b-unsafe-local = { };
    users.users = lib.genAttrs eligibleUsers (_: {
      extraGroups = [ "d2b-unsafe-local" ];
    });

    d2b._hostToolPackages.d2bUnsafeLocalHelper = helperPackage;
    environment.systemPackages = [ helperPackage ];

    systemd.user.services.d2b-unsafe-local-helper = {
      description = "d2b same-uid unsafe-local runtime helper";
      wantedBy = [ "default.target" ];
      unitConfig.ConditionGroup = "d2b-unsafe-local";
      serviceConfig = {
        Type = "simple";
        ExecStart = "${helperPackage}/bin/d2b-unsafe-local-helper --wayland-proxy ${cfg._hostToolPackages.d2bWaylandProxy}/bin/d2b-wayland-proxy";
        Restart = "on-failure";
        RestartSec = "5s";
        Slice = "app.slice";
      };
    };
  };
}
