{ pkgs }:

let
  inherit (pkgs) lib stdenv fetchurl;

  version = "9.2.0";

  sources = {
    x86_64-linux = {
      url = "https://github.com/bazelbuild/bazel/releases/download/9.2.0/bazel-9.2.0-linux-x86_64";
      hash = "sha256-dmipXbElDxLEBAclHk4gO07Ivzm8SV0vSFstjJkEhpQ=";
      upstreamSha256 = "7668a95db1250f12c40407251e4e203b4ec8bf39bc495d2f485b2d8c99048694";
      upstreamChecksumUrl = "https://github.com/bazelbuild/bazel/releases/download/9.2.0/bazel-9.2.0-linux-x86_64.sha256";
    };
    aarch64-linux = {
      url = "https://github.com/bazelbuild/bazel/releases/download/9.2.0/bazel-9.2.0-linux-arm64";
      hash = "sha256-BJ3SH0Ctl52xHD7mjJakLOdfEYXmmsYasg3hUBQnpBA=";
      upstreamSha256 = "049dd21f40ad979db11c3ee68c96a42ce75f1185e69ac61ab20de1501427a410";
      upstreamChecksumUrl = "https://github.com/bazelbuild/bazel/releases/download/9.2.0/bazel-9.2.0-linux-arm64.sha256";
    };
  };

  source = sources.${stdenv.hostPlatform.system}
    or (throw "Bazel ${version} native package is not supported on ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "bazel";
  inherit version;

  src = fetchurl {
    inherit (source) url hash;
  };
  dontUnpack = true;
  # The release artifact is a self-extracting ELF plus an appended ZIP. Nix's
  # normal strip or ELF patching would truncate the embedded Bazel payload.
  dontStrip = true;
  dontPatchELF = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 "$src" "$out/bin/bazel"
    runHook postInstall
  '';

  passthru = {
    inherit (source) upstreamChecksumUrl upstreamSha256;
    officialReleaseUrl =
      "https://github.com/bazelbuild/bazel/releases/tag/${version}";
  };

  meta = {
    description = "Official upstream Bazel ${version} release binary";
    homepage = "https://bazel.build/";
    license = lib.licenses.asl20;
    mainProgram = "bazel";
    platforms = builtins.attrNames sources;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
