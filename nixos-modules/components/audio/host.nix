{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  rows = import ../../realm-audio-rows.nix { inherit config lib pkgs; };
  anyAudio = rows.processes != [ ];
  vhostDeviceSound = import ../../../pkgs/vhost-device-sound { inherit pkgs; };
in
{
  config = lib.mkMerge [
    {
      assertions = rows.assertions;
    }

    (lib.mkIf anyAudio {
      services.pipewire.extraConfig.client."90-d2b" = {
        "stream.rules" =
          (lib.optional (cfg.site.audio.inputTargetNode != null) {
            matches = [
              {
                "d2b.mic" = "on";
                "media.class" = "Stream/Input/Audio";
              }
            ];
            actions.update-props."target.object" =
              cfg.site.audio.inputTargetNode;
          })
          ++ [
            {
              matches = [
                {
                  "d2b.mic" = "off";
                  "media.class" = "Stream/Input/Audio";
                }
              ];
              actions.update-props = {
                "target.object" = "-1";
                "node.dont-reconnect" = true;
                "node.dont-fallback" = true;
                "node.linger" = true;
              };
            }
            {
              matches = [
                {
                  "d2b.speaker" = "off";
                  "media.class" = "Stream/Output/Audio";
                }
              ];
              actions.update-props = {
                "target.object" = "-1";
                "node.dont-reconnect" = true;
                "node.dont-fallback" = true;
                "node.linger" = true;
              };
            }
          ];
      };

      environment.systemPackages = [ vhostDeviceSound ];
    })
  ];
}
