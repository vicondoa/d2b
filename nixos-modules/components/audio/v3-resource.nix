# Provider-neutral v3 audio resource authoring facts.
{ config, lib, ... }:

{
  options.d2b.audio.v3 = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
    };
    providerRef = lib.mkOption {
      type = lib.types.str;
      default = "Provider/audio-pipewire";
      readOnly = true;
    };
    captureAlias = lib.mkOption {
      type = lib.types.nullOr (lib.types.strMatching "^[a-z][a-z0-9-]{0,63}$");
      default = null;
    };
  };

  options.d2b._audioV3 = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    default = { };
    internal = true;
    visible = false;
  };

  config.d2b._audioV3 = {
    enabled = config.d2b.audio.v3.enable;
    providerRef = config.d2b.audio.v3.providerRef;
    serviceType = "audio.d2bus.org.AudioService";
    bindingType = "audio.d2bus.org.AudioBinding";
    microphone = "exclusive";
    speaker = "multiplexed";
    captureAlias = config.d2b.audio.v3.captureAlias;
    declaresStateVolume = false;
    mkServiceResource = {
      name,
      role ? "owner",
      endpointRefs ? [ ]
    }: {
      inherit name;
      type = "audio.d2bus.org.AudioService";
      spec = {
        providerRef = "Provider/audio-pipewire";
        serviceRole = role;
        implementationEndpointRefs = endpointRefs;
        operations = [ "playback" "capture" ];
      };
    };
    mkBindingResource = {
      name,
      serviceRef,
      targetRef,
      grants ? { mic = "off"; speaker = "off"; }
    }: {
      inherit name;
      type = "audio.d2bus.org.AudioBinding";
      spec = {
        providerRef = "Provider/audio-pipewire";
        inherit serviceRef targetRef grants;
      };
    };
  };
}
