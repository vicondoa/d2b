{ lib, ... }:

let
  evaluated = lib.evalModules {
    modules = [ (import ../default.nix) ];
  };
  enabled = lib.evalModules {
    modules = [
      (import ../default.nix)
      { config.d2b.audio.v3.enable = true; }
    ];
  };
in
{
  cases = {
    "provider-audio-pipewire/modules-evaluate" = {
      expr = builtins.deepSeq evaluated.config.d2b._audioV3 true;
      expected = true;
      propagateError = true;
    };

    "provider-audio-pipewire/defaults-are-provider-owned" = {
      expr = {
        providerRef = evaluated.config.d2b._audioV3.providerRef;
        stateVolume = evaluated.config.d2b._audioV3.declaresStateVolume;
        enabled = enabled.config.d2b._audioV3.enabled;
      };
      expected = {
        providerRef = "Provider/audio-pipewire";
        stateVolume = false;
        enabled = true;
      };
    };
  };
}
