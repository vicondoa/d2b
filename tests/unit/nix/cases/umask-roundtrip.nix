{ lib, flakeRoot, system, ... }:

lib.optionalAttrs (system == "x86_64-linux") (
  let
    source = builtins.readFile
      (flakeRoot + "/nixos-modules/minijail-profiles.nix");
    roleHasUmask = role:
      lib.hasInfix
        ''[ "swtpm" "gpu" "gpu-render-node" "video" "audio" "wayland-proxy" ]''
        source
      && lib.hasInfix "then 7" source
      && lib.hasInfix role source;
  in
  {
    "umask-roundtrip/swtpm" = {
      expr = roleHasUmask ''"swtpm"'';
      expected = true;
    };
    "umask-roundtrip/gpu" = {
      expr = roleHasUmask ''"gpu"'';
      expected = true;
    };
    "umask-roundtrip/video" = {
      expr = roleHasUmask ''"video"'';
      expected = true;
    };
    "umask-roundtrip/audio" = {
      expr = roleHasUmask ''"audio"'';
      expected = true;
    };
  }
)
