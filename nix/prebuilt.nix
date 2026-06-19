{ pkgs, lib }:

# Reads nix/prebuilt.json and provides pre-built binary derivations.
# Uses autoPatchelfHook to fix library paths for the consumer's nixpkgs.
# Returns null when no release is available (callers fall back to source).

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
      nativeBuildInputs = [ pkgs.autoPatchelfHook ];
      buildInputs = [ pkgs.stdenv.cc.cc.lib ];
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
