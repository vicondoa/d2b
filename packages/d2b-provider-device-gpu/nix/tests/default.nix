{ lib, modules, ... }:

let
  moduleSources = map builtins.readFile modules;
  source = builtins.concatStringsSep "\n" moduleSources;
in
{
  cases = {
    "provider-device-gpu/modules-are-modules" = {
      expr = builtins.all
        (module: builtins.isFunction (import module))
        modules;
      expected = true;
    };

    "provider-device-gpu/video-worker-contract" = {
      expr =
        lib.hasInfix "vhost-user-media" source
        && lib.hasInfix "video" source;
      expected = true;
    };
  };
}
