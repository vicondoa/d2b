{ pkgs }:

let
  inherit (pkgs) lib stdenv fetchurl autoPatchelfHook;

  version = "0.128.0";

  sources = {
    x86_64-linux = {
      arch = "amd64";
      hash = "sha256-+rq3wXdMwqIE7ndLzC3kaSv8iQJ6e3rIWA3XTjuoj0Y=";
      upstreamSha256 = "fabab7c1774cc2a204ee774bcc2de4692bfc89027a7b7ac8580dd74e3ba88f46";
    };
    aarch64-linux = {
      arch = "arm64";
      hash = "sha256-zeilSroOis7MSgZcmMHCwA3zF5PcFAXqkLDoKIOeonc=";
      upstreamSha256 = "cde8a54aba0e8acecc4a065c98c1c2c00df31793dc1405ea90b0e828839ea277";
    };
  };

  source = sources.${stdenv.hostPlatform.system}
    or (throw "SigNoz native package is not supported on ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "signoz";
  inherit version;

  src = fetchurl {
    url = "https://github.com/SigNoz/signoz/releases/download/v${version}/signoz_linux_${source.arch}.tar.gz";
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
    upstreamChecksumsUrl = "https://github.com/SigNoz/signoz/releases/download/v${version}/signoz_${version}_checksums.txt";
  };

  meta = {
    description = "SigNoz server and web UI native release artifact";
    homepage = "https://github.com/SigNoz/signoz";
    license = lib.licenses.asl20;
    mainProgram = "signoz";
    platforms = builtins.attrNames sources;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
