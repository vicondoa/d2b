{ config, lib, pkgs, ... }:

let
  cfg = config.d2b.site.hostSccache;
  cacheDir = "/var/cache/d2b-sccache";
  buildUsersGroup = "nixbld";
  hostIsLinux = pkgs.stdenv.hostPlatform.isLinux;
  configuredBuildUsersGroup =
    config.nix.settings."build-users-group" or buildUsersGroup;
in
{
  config = lib.mkMerge [
    {
      assertions = lib.optionals cfg.enable [
        {
          assertion = hostIsLinux;
          message = ''
            d2b.site.hostSccache.enable requires a Linux NixOS host;
            the fixed /var/cache/d2b-sccache tmpfiles and Nix daemon
            sandbox contract is not portable to this platform.
          '';
        }
        {
          assertion = builtins.hasAttr buildUsersGroup config.users.groups;
          message = ''
            d2b.site.hostSccache.enable requires the Nix multi-user build
            group '${buildUsersGroup}'. Enable the Nix daemon/build-user
            module before enabling this option.
          '';
        }
        {
          assertion = configuredBuildUsersGroup == buildUsersGroup;
          message = ''
            d2b.site.hostSccache.enable requires
            nix.settings.build-users-group = '${buildUsersGroup}' so the
            root:${buildUsersGroup} setgid cache is writable by daemon build
            users.
          '';
        }
      ];
    }
    (lib.mkIf cfg.enable {
      nix.settings = {
        "build-users-group" = lib.mkDefault buildUsersGroup;
        "extra-sandbox-paths" = lib.mkAfter [ cacheDir ];
      };

      systemd.tmpfiles.rules = [
        "d ${cacheDir} 2770 root ${buildUsersGroup} -"
      ];
    })
  ];
}
