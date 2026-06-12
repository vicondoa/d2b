{ pkgs }:

let
  inherit (pkgs) lib stdenv fetchurl autoPatchelfHook;

  version = "0.144.5";

  sources = {
    x86_64-linux = {
      arch = "amd64";
      hash = "sha256-CkQO2mEnEX/csUNoU9POv146LR7rpVLUMufG2TnwGsE=";
      upstreamSha256 = "0a440eda6127117fdcb1436853d3cebf5e3a2d1eeba552d432e7c6d939f01ac1";
    };
    aarch64-linux = {
      arch = "arm64";
      hash = "sha256-gZi2ZVz9SrgXFpJ83d4s8gwWcPiDXiddjEQe2gRmngI=";
      upstreamSha256 = "8198b6655cfd4ab81716927cddde2cf20c1670f8835e275d8c441eda04669e02";
    };
  };

  source = sources.${stdenv.hostPlatform.system}
    or (throw "SigNoz OTel Collector native package is not supported on ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "signoz-otel-collector";
  inherit version;

  src = fetchurl {
    url = "https://github.com/SigNoz/signoz-otel-collector/releases/download/v${version}/signoz-otel-collector_linux_${source.arch}.tar.gz";
    inherit (source) hash;
  };

  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [ stdenv.cc.cc.lib ];

  installPhase = ''
    runHook preInstall

    mkdir -p "$out"
    cp -R ./* "$out"/

    runHook postInstall
  '';

  passthru = {
    inherit (source) upstreamSha256;
    upstreamChecksumsUrl = "https://github.com/SigNoz/signoz-otel-collector/releases/download/v${version}/signoz-otel-collector_${version}_checksums.txt";
  };

  meta = {
    description = "SigNoz OpenTelemetry Collector native release artifact";
    homepage = "https://github.com/SigNoz/signoz-otel-collector";
    license = lib.licenses.asl20;
    mainProgram = "signoz-otel-collector";
    platforms = builtins.attrNames sources;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
