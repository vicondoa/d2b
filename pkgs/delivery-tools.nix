{ pkgs }:

let
  cargoUdepsVersion = "0.1.61";
  cargoUdepsNightlyDate = "2025-12-01";
  cargoSemverChecksVersion = "0.47.0";
  ghVersion = "2.92.0";
  gitTownVersion = "23.0.1";
  rustStableVersion = "1.94.1";

  stableRust = pkgs.rust-bin.stable.${rustStableVersion}.minimal;
  nightlyRust = pkgs.rust-bin.nightly.${cargoUdepsNightlyDate}.minimal;
  stableRustPlatform = pkgs.makeRustPlatform {
    cargo = stableRust;
    rustc = stableRust;
  };

  gh = pkgs.buildGoModule (finalAttrs: {
    pname = "gh";
    version = ghVersion;

    src = pkgs.fetchFromGitHub {
      owner = "cli";
      repo = "cli";
      tag = "v${finalAttrs.version}";
      hash = "sha256-/7EiX4ZZPhSNgY/D5OVOako/c0ujHq05GMj3UB11bqQ=";
    };

    vendorHash = "sha256-pBLRCIRjN3VoXbTFSq+R9/N3uAUCEjvPtk8LKKKS51s=";
    nativeBuildInputs = [
      pkgs.installShellFiles
      pkgs.makeWrapper
    ];

    buildPhase = ''
      runHook preBuild
      make \
        GO_LDFLAGS="-s -w -X github.com/cli/cli/v${pkgs.lib.versions.major finalAttrs.version}/internal/build.Date=d2b" \
        GH_VERSION=${finalAttrs.version} \
        bin/gh \
        ${pkgs.lib.optionalString (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform) "manpages"}
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      installBin bin/gh
      wrapProgram "$out/bin/gh" --set-default GH_TELEMETRY false
    ''
    + pkgs.lib.optionalString
      (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform)
      ''
        installManPage share/man/*/*.[1-9]
        installShellCompletion --cmd gh \
          --bash <("$out/bin/gh" completion -s bash) \
          --fish <("$out/bin/gh" completion -s fish) \
          --zsh <("$out/bin/gh" completion -s zsh)
      ''
    + ''
      runHook postInstall
    '';

    doCheck = false;
    nativeInstallCheckInputs = [ pkgs.versionCheckHook ];
    doInstallCheck = true;

    meta = {
      description = "GitHub CLI tool";
      homepage = "https://cli.github.com/";
      changelog = "https://github.com/cli/cli/releases/tag/v${finalAttrs.version}";
      license = pkgs.lib.licenses.mit;
      mainProgram = "gh";
      platforms = pkgs.lib.platforms.linux;
    };
  });

  gitTown = pkgs.buildGoModule (finalAttrs: {
    pname = "git-town";
    version = gitTownVersion;

    src = pkgs.fetchFromGitHub {
      owner = "git-town";
      repo = "git-town";
      tag = "v${finalAttrs.version}";
      hash = "sha256-kAAzfb0rg10k9PnUKYEqdSWYWi0JR6jiKDHUv/RSUSs=";
    };

    # This release commits its vendor tree, which is covered by the source hash.
    vendorHash = null;
    nativeBuildInputs = [
      pkgs.installShellFiles
      pkgs.makeWrapper
    ];
    buildInputs = [ pkgs.git ];

    ldflags = [
      "-s"
      "-w"
      "-X github.com/git-town/git-town/v${pkgs.lib.versions.major finalAttrs.version}/src/cmd.version=v${finalAttrs.version}"
      "-X github.com/git-town/git-town/v${pkgs.lib.versions.major finalAttrs.version}/src/cmd.buildDate=d2b"
    ];

    nativeCheckInputs = [
      pkgs.git
      pkgs.writableTmpDirAsHomeHook
    ];
    preCheck = ''
      rm main_test.go
    '';
    checkFlags = [
      "-skip=^TestMockingRunner/MockCommand$|^TestMockingRunner/MockCommitMessage$|^TestMockingRunner/QueryWith$|^TestTestCommands/CreateChildFeatureBranch$|^TestTestCommands/CreateChildBranch$|^TestTestCommands/CreateLocalBranchUsingGitTown$|^TestFrontendRunner_RetryOnIndexLock$"
    ];

    postInstall =
      pkgs.lib.optionalString
        (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform)
        ''
          installShellCompletion --cmd git-town \
            --bash <("$out/bin/git-town" completions bash) \
            --fish <("$out/bin/git-town" completions fish) \
            --zsh <("$out/bin/git-town" completions zsh)
        ''
      + ''
        wrapProgram "$out/bin/git-town" \
          --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.git ]}
      '';

    meta = {
      description = "Generic, high-level Git workflow support";
      homepage = "https://www.git-town.com/";
      license = pkgs.lib.licenses.mit;
      mainProgram = "git-town";
      platforms = pkgs.lib.platforms.linux;
    };
  });

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
