{ pkgs, lib }:

# Reads nix/prebuilt.json and provides pre-built binary derivations.
# Uses autoPatchelfHook to fix library paths for the consumer's nixpkgs.
# Returns null when no release is available (callers fall back to source).
# A manifest entry may set `sourceBinary` when a released asset still carries
# a legacy executable name that the consumer package must rename.

let
  manifest = builtins.fromJSON (builtins.readFile ./prebuilt.json);
  hasRelease =
    manifest.version != null
    && builtins.length (builtins.attrNames manifest.binaries) > 0;

  mkPrebuilt = name: spec:
    let
      sourceBinary = spec.sourceBinary or null;
      installBinaries =
        if sourceBinary != null then ''
          candidate=./${sourceBinary}
          if [ ! -f "$candidate" ] || [ ! -x "$candidate" ]; then
            echo "prebuilt ${name}: expected executable ${sourceBinary}" >&2
            exit 1
          fi
          install -Dm755 "$candidate" "$out/bin/${name}"
        '' else ''
          for candidate in ./*; do
            if [ -f "$candidate" ] && [ -x "$candidate" ]; then
              install -Dm755 "$candidate" "$out/bin/$(basename "$candidate")"
            fi
          done
        '';
    in
    pkgs.stdenv.mkDerivation {
      pname = name;
      version = manifest.version;
      src = pkgs.fetchurl {
        inherit (spec) url hash;
      };
      nativeBuildInputs = [ pkgs.autoPatchelfHook ];
      buildInputs = [ pkgs.stdenv.cc.cc.lib ];
      passthru = { inherit sourceBinary; };
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
        ${installBinaries}
        runHook postInstall
      '';
      meta.platforms = [ manifest.system ];
    };
in
if hasRelease then
  lib.mapAttrs mkPrebuilt manifest.binaries
else
  null
