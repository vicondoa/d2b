{ pkgs, lib }:

# Reads nix/prebuilt.json and provides pre-built binary derivations.
# When a release has been published and hashes committed, this returns
# real fetchurl derivations. Otherwise returns null so callers can
# fall back to building from source.
#
# Usage in NixOS modules (e.g., host-daemon.nix):
#
#   let
#     prebuilt = import ../nix/prebuilt.nix { inherit pkgs lib; };
#     nixlingdBinary =
#       if prebuilt != null && prebuilt ? nixlingd
#       then "${prebuilt.nixlingd}/bin/nixlingd"
#       else "${nixlingdPackage}/bin/nixlingd";
#   in
#   ...

let
  manifest = builtins.fromJSON (builtins.readFile ./prebuilt.json);
  hasRelease =
    manifest.version != null
    && builtins.length (builtins.attrNames manifest.binaries) > 0;

  mkPrebuilt = name: spec:
    pkgs.stdenv.mkDerivation {
      pname = name;
      version = manifest.version;
      src = pkgs.fetchurl {
        inherit (spec) url hash;
      };
      dontConfigure = true;
      dontBuild = true;
      unpackPhase = ''
        runHook preUnpack
        tar -xzf "$src"
        runHook postUnpack
      '';
      installPhase = ''
        runHook preInstall
        mkdir -p "$out/bin"
        for candidate in ./*; do
          if [ -f "$candidate" ] && [ -x "$candidate" ]; then
            install -Dm755 "$candidate" "$out/bin/$(basename "$candidate")"
          fi
        done
        runHook postInstall
      '';
      meta.platforms = [ manifest.system ];
    };
in
if hasRelease then
  lib.mapAttrs mkPrebuilt manifest.binaries
else
  null
