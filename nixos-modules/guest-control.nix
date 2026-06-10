{ config, lib, nixlingInputs, pkgs, ... }:

let
  cfg = config.nixling.guestControl;
  guestPackages = nixlingInputs.self.packages.${pkgs.stdenv.hostPlatform.system};
in
{
  options.nixling.guestControl.enable = lib.mkOption {
    type = lib.types.bool;
    default = false;
    internal = true;
    description = "Whether nixling's guest-control credential surface is wired in this guest.";
  };

  config = {
    environment.systemPackages = [
      guestPackages.nixling-guestd-static
      guestPackages.nixling-userd-static
      guestPackages.nixling-exec-runner-static
    ];

    systemd.services.nixling-guestd = lib.mkIf cfg.enable {
      description = "nixling guest control daemon";
      wantedBy = [ ];
      unitConfig.RequiresMountsFor = [ "/run/nixling-guest-control-host" ];
      serviceConfig = {
        Type = "exec";
        ExecStart = "${guestPackages.nixling-guestd-static}/bin/nixling-guestd";
        LoadCredential = [
          "guest_control_token:/run/nixling-guest-control-host/token"
        ];
      };
    };
  };
}
