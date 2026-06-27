# Home Manager support for d2b VMs. Imported by host.nix when a
# VM sets `d2b.vms.<name>.homeManager.enable = true`. The
# per-VM `homeManager.users` attrset declared host-side is
# propagated into this guest module's `d2b.homeManager.users`,
# and from there into the upstream `home-manager.users` option.
#
# Default HM wiring matches the host's setup (useGlobalPkgs +
# useUserPackages + .hm-backup extension + inputs in
# extraSpecialArgs) so VM HM configs can reuse modules from
# ./home/<user>/ without surprises.
{ lib, inputs, config, ... }:

let
  cfg = config.d2b.homeManager;
in

{
  imports = [ inputs.home-manager.nixosModules.home-manager ];

  options.d2b.homeManager.users = lib.mkOption {
    type = lib.types.attrsOf lib.types.unspecified;
    default = { };
    description = ''
      Per-user Home Manager config attrsets. Populated by host.nix
      from the host-side `d2b.vms.<name>.homeManager.users`
      option. Each value is a NixOS HM module:

        { alice = {
            imports = [ ./home/alice/core.nix ];
            home.username = "alice";
            home.homeDirectory = "/home/alice";
            home.stateVersion = "25.11";
          };
        }
    '';
  };

  config = {
    home-manager = {
      useGlobalPkgs = true;
      useUserPackages = true;
      backupFileExtension = "hm-backup";
      extraSpecialArgs = { inherit inputs; };
      users = cfg.users;
    };
  };
}
