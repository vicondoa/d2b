{ pkgs }:

let
  cargoUdepsVersion = "0.1.61";
  cargoUdepsNightlyDate = "2025-12-01";
  cargoSemverChecksVersion = "0.47.0";
  rustStableVersion = "1.94.1";

  stableRust = pkgs.rust-bin.stable.${rustStableVersion}.minimal;
  nightlyRust = pkgs.rust-bin.nightly.${cargoUdepsNightlyDate}.minimal;
  stableRustPlatform = pkgs.makeRustPlatform {
    cargo = stableRust;
    rustc = stableRust;
  };

  gh = pkgs.gh;
  gitTown = pkgs.git-town;

  cargoUdepsRaw = stableRustPlatform.buildRustPackage {
    pname = "cargo-udeps";
    version = cargoUdepsVersion;

    src = pkgs.fetchFromGitHub {
      owner = "est31";
      repo = "cargo-udeps";
      rev = "v${cargoUdepsVersion}";
      hash = "sha256-yT/EJWGGhQapbU1o1Gus1Vk5cAhso5ALTBecB3BH46g=";
    };

    cargoHash = "sha256-DGfAsBucFRFJkjmJkpTpNfQO79jaNa5NezXKf7hYYeM=";
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.openssl ];
    doCheck = false;

    meta = {
      description = "Find unused Cargo dependencies";
      homepage = "https://github.com/est31/cargo-udeps";
      license = pkgs.lib.licenses.mit;
      mainProgram = "cargo-udeps";
      platforms = pkgs.lib.platforms.linux;
    };
  };

  cargoUdepsNightly = pkgs.runCommand "cargo-udeps-nightly-${cargoUdepsVersion}" {
    nativeBuildInputs = [ pkgs.makeWrapper ];
    passthru = {
      version = cargoUdepsVersion;
      nightlyDate = cargoUdepsNightlyDate;
      inherit nightlyRust;
    };
    meta = cargoUdepsRaw.meta // {
      mainProgram = "cargo-udeps-nightly";
    };
  } ''
    mkdir -p "$out/bin"
    makeWrapper ${cargoUdepsRaw}/bin/cargo-udeps "$out/bin/cargo-udeps" \
      --set CARGO ${nightlyRust}/bin/cargo \
      --set RUSTC ${nightlyRust}/bin/rustc \
      --prefix PATH : ${pkgs.lib.makeBinPath [ nightlyRust pkgs.sccache ]}
    makeWrapper ${cargoUdepsRaw}/bin/cargo-udeps "$out/bin/cargo-udeps-nightly" \
      --set CARGO ${nightlyRust}/bin/cargo \
      --set RUSTC ${nightlyRust}/bin/rustc \
      --prefix PATH : ${pkgs.lib.makeBinPath [ nightlyRust pkgs.sccache ]} \
      --add-flags udeps
  '';

  cargoSemverChecks = stableRustPlatform.buildRustPackage {
    pname = "cargo-semver-checks";
    version = cargoSemverChecksVersion;

    src = pkgs.fetchFromGitHub {
      owner = "obi1kenobi";
      repo = "cargo-semver-checks";
      tag = "v${cargoSemverChecksVersion}";
      hash = "sha256-1D6WFsiMOl/bJr0J+mmvLlgnRSKN6rPhDSnDsdLTC9E=";
    };

    cargoHash = "sha256-YbtYIHj899eJSrp5n5jODgTkL9L26EnruzECwBrBF00=";
    nativeBuildInputs = [ pkgs.cmake ];
    buildInputs = [ pkgs.zlib ];
    checkFlags = [
      "--skip=detects_target_dependencies"
      "--skip=query::tests_lints::feature_missing"
    ];
    preCheck = ''
      rm -r test_crates/feature_missing
      patchShebangs scripts/regenerate_test_rustdocs.sh
      scripts/regenerate_test_rustdocs.sh
      substituteInPlace test_outputs/integration_snapshots__bugreport.snap \
        --replace-fail \
          'cargo-semver-checks [VERSION] ([HASH])' \
          'cargo-semver-checks ${cargoSemverChecksVersion}'
    '';

    meta = {
      description = "Scan Rust crates for semantic-versioning violations";
      homepage = "https://github.com/obi1kenobi/cargo-semver-checks";
      license = with pkgs.lib.licenses; [ mit asl20 ];
      mainProgram = "cargo-semver-checks";
      platforms = pkgs.lib.platforms.linux;
    };
  };
in
{
  inherit
    cargoSemverChecks
    cargoUdepsNightly
    gh
    gitTown
    nightlyRust
    stableRust
    stableRustPlatform
    rustStableVersion
    ;
}
