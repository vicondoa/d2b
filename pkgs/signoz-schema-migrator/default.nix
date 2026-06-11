{ pkgs }:

let
  inherit (pkgs) lib stdenv fetchurl autoPatchelfHook;

  version = "0.144.5";

  sources = {
    x86_64-linux = {
      arch = "amd64";
      hash = "sha256-uPwX92fxsSyjTEDUuPkZ44C7ED25BKWH6he1TkITxZY=";
      upstreamSha256 = "b8fc17f767f1b12ca34c40d4b8f919e380bb103db904a587ea17b54e4213c596";
    };
    aarch64-linux = {
      arch = "arm64";
      hash = "sha256-AajTy5P5vVYo0W48+T29DP7VYW9ApVQSeti7mV/nvHo=";
      upstreamSha256 = "01a8d3cb93f9bd5628d16e3cf93dbd0cfed5616f40a554127ad8bb995fe7bc7a";
    };
  };

  source = sources.${stdenv.hostPlatform.system}
    or (throw "SigNoz schema migrator native package is not supported on ${stdenv.hostPlatform.system}");
in
stdenv.mkDerivation {
  pname = "signoz-schema-migrator";
  inherit version;

  src = fetchurl {
    url = "https://github.com/SigNoz/signoz-otel-collector/releases/download/v${version}/signoz-schema-migrator_linux_${source.arch}.tar.gz";
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
    upstreamChecksumsUrl = "https://github.com/SigNoz/signoz-otel-collector/releases/download/v${version}/signoz-schema-migrator_${version}_checksums.txt";
  };

  meta = {
    description = "SigNoz ClickHouse schema migrator native release artifact";
    homepage = "https://github.com/SigNoz/signoz-otel-collector";
    license = lib.licenses.asl20;
    mainProgram = "signoz-schema-migrator";
    platforms = builtins.attrNames sources;
    sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
  };
}
