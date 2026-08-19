# Contract-first cases for the shared nix-unit corpus.
#
# The missing-file and duplicate-name probes call the production corpus
# aggregator directly. Pin coverage is checked against the generator's
# committed output for this file.
#
# Scope note on the shard probe below. It reads the committed flake source
# and asserts that THIS file appears in the shard map exactly once. That is
# self-registration only: it says nothing about the other case files and does
# not prove shard coverage of the corpus. The general bijection - every case
# file in exactly one shard, no unknown entries, no duplicates - is computed
# in flake.nix as `nixUnitShardMissingFiles`, `nixUnitShardUnknownFiles`,
# `nixUnitShardDuplicateFiles` and `nixUnitShardCoverageOk`, and the
# `nix-unit` integrity check fails closed on it with a JSON report. Do not
# reimplement that here; a second evaluator would be a second answer.
{ lib
, pkgs
, system
, flakeRoot
, d2bLib
, mkEval ? null
, nixpkgsFlake ? null
, d2bModule ? null
, ...
}:

let
  aggregate = caseFileNames:
    import (flakeRoot + "/tests/unit/nix/default.nix") {
      inherit lib pkgs system flakeRoot d2bLib mkEval nixpkgsFlake d2bModule;
      inherit caseFileNames;
    };

  # Keep the malformed-input probes on the real aggregator. In particular,
  # do not copy its missing-file or duplicate-name merge logic here.
  smallCorpusFile = "volume-mounts.nix";
  missingCorpusFile = "missing-case-file.nix";

  flakeSource = builtins.readFile (flakeRoot + "/flake.nix");
  rustHostToolsSource =
    builtins.readFile (flakeRoot + "/nixos-modules/rust-host-tools.nix");
  hostBrokerSource =
    builtins.readFile (flakeRoot + "/nixos-modules/host-broker.nix");
  bundleSource = builtins.readFile (flakeRoot + "/nixos-modules/bundle.nix");
  processesSource = builtins.readFile (flakeRoot + "/nixos-modules/processes-json.nix");
  privilegesSource = builtins.readFile (flakeRoot + "/nixos-modules/privileges-json.nix");
  liveCutoverSource =
    builtins.readFile (flakeRoot + "/tests/integration/live/cutover-real-host.sh");
  makefileSource = builtins.readFile (flakeRoot + "/Makefile");
  sccacheSandboxDir = "/var/cache/d2b-sccache";
  providerSchemaPaths = [
    "docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json"
    "docs/reference/schemas/v3/providers/transport-vsock.transport-binding.json"
  ];
  shardEntryLines = builtins.filter
    (line: lib.hasInfix ''"test-infrastructure.nix"'' line)
    (lib.splitString "\n" flakeSource);

  readPins = path:
    lib.filter
      (name: name != "" && !(lib.hasPrefix "#" name))
      (lib.splitString "\n" (builtins.readFile path));
  pinnedNames =
    readPins (flakeRoot + "/tests/unit/nix/pinned/common.txt")
    ++ readPins (flakeRoot + "/tests/unit/nix/pinned/${system}.txt");

  ownCaseNames = [
    "test-infrastructure/shared-corpus-missing-file-rejected"
    "test-infrastructure/shared-corpus-duplicate-name-rejected"
    "test-infrastructure/own-shard-registration-unique"
    "test-infrastructure/pin-integrity-complete"
    "test-infrastructure/cutover-runner-host-tool-contract"
    "test-infrastructure/cutover-live-driver-contract"
  ];
  unpinnedOwnCases =
    lib.filter (name: !(builtins.elem name pinnedNames)) ownCaseNames;
in
{
  "test-infrastructure/shared-corpus-missing-file-rejected" = {
    expr = aggregate [ missingCorpusFile ];
    expectedError = { };
  };

  "test-infrastructure/shared-corpus-duplicate-name-rejected" = {
    expr = aggregate [ smallCorpusFile smallCorpusFile ];
    expectedError = { };
  };

  # Self-registration only: this file is named in the flake's shard map
  # exactly once. Corpus-wide shard coverage is flake.nix's job (see the
  # header note); this case must not grow into a second implementation of it.
  "test-infrastructure/own-shard-registration-unique" = {
    expr = {
      missing = shardEntryLines == [ ];
      duplicate = lib.length shardEntryLines > 1;
    };
    expected = {
      missing = false;
      duplicate = false;
    };
  };

  "test-infrastructure/pin-integrity-complete" = {
    expr = {
      pinFilesExist =
        builtins.pathExists (flakeRoot + "/tests/unit/nix/pinned/common.txt")
        && builtins.pathExists (flakeRoot + "/tests/unit/nix/pinned/${system}.txt");
      unpinned = unpinnedOwnCases;
    };
    expected = {
      pinFilesExist = true;
      unpinned = [ ];
    };
  };

  "test-infrastructure/provider-runtime-schema-is-staged-with-rust-sources" = {
    expr = {
      flakeSource = lib.all (path: lib.hasInfix path flakeSource) providerSchemaPaths;
      rustHostToolsSource =
        lib.all (path: lib.hasInfix path rustHostToolsSource) providerSchemaPaths;
    };
    expected = {
      flakeSource = true;
      rustHostToolsSource = true;
    };
  };

  "test-infrastructure/host-tools-sccache-contract" = {
    expr = {
      constantSandboxDir =
        lib.hasInfix ''sccacheDir = "${sccacheSandboxDir}"'' rustHostToolsSource
        && lib.hasInfix "SCCACHE_DIR = sccacheDir" rustHostToolsSource;
      noImpureSccacheEnv =
        !(lib.hasInfix ''builtins.getEnv "SCCACHE_DIR"'' rustHostToolsSource);
      wrapperRequiresWritableDir =
        lib.hasInfix "[ -d \"''\${SCCACHE_DIR}\" ]" rustHostToolsSource
        && lib.hasInfix "[ -w \"''\${SCCACHE_DIR}\" ]" rustHostToolsSource
        && lib.hasInfix "exec sccache" rustHostToolsSource
        && lib.hasInfix ''exec "$@"'' rustHostToolsSource;
      waylandProxySourceBuilt =
        lib.hasInfix "waylandProxy = mkMainPackage" rustHostToolsSource;
      makefileNoWorldWritableCache = !(lib.hasInfix "chmod 1777" makefileSource);
      makefileCacheOptIn =
        lib.hasInfix "D2B_HOST_SCCACHE" makefileSource
        && lib.hasInfix "chmod 0700" makefileSource
        && lib.hasInfix "${sccacheSandboxDir}=" makefileSource;
      makefileDefaultBuildIsPure =
        !(lib.hasInfix "nix build --impure" makefileSource);
    };
    expected = {
      constantSandboxDir = true;
      noImpureSccacheEnv = true;
      wrapperRequiresWritableDir = true;
      waylandProxySourceBuilt = true;
      makefileNoWorldWritableCache = true;
      makefileCacheOptIn = true;
      makefileDefaultBuildIsPure = true;
    };
  };

  "test-infrastructure/cutover-runner-host-tool-contract" = {
    expr = {
      sourcePackage = lib.hasInfix ''"d2b-cutover"'' rustHostToolsSource;
      outputPackage = lib.hasInfix "cutoverRunner = mkMainPackage" rustHostToolsSource;
      brokerPath = lib.hasInfix "D2B_CUTOVER_RUNNER_PATH" hostBrokerSource;
      bundlePath = lib.hasInfix "cutoverRunnerPath" bundleSource;
      processContract = lib.hasInfix "cutoverRunner" processesSource;
      privilegeContract =
        lib.hasInfix ''"operation": "LaunchCutoverRunner"'' privilegesSource
        && lib.hasInfix ''"operation": "CutoverAudit"'' privilegesSource
        && lib.hasInfix ''"operation": "CutoverEffect"'' privilegesSource;
      noPersistentUnit = !(lib.hasInfix "systemd.services.d2b-cutover" hostBrokerSource);
    };
    expected = {
      sourcePackage = true;
      outputPackage = true;
      brokerPath = true;
      bundlePath = true;
      processContract = true;
      privilegeContract = true;
      noPersistentUnit = true;
    };
  };

  "test-infrastructure/cutover-live-driver-contract" = {
    expr = {
      selfGuard =
        lib.hasInfix ''d2b_heavy_gate_reexec "$ROOT" "$0" "$@"'' liveCutoverSource;
      explicitLiveGate =
        lib.hasInfix ''D2B_LIVE:-0'' liveCutoverSource
        && lib.hasInfix ''D2B_LIVE=1'' liveCutoverSource;
      requiredEvidencePaths = lib.all
        (marker: lib.hasInfix marker liveCutoverSource)
        [
          "D2B_LIVE_STATE_DIR"
          "D2B_LIVE_CANDIDATE_DIR"
          "D2B_LIVE_SNAPSHOT"
          "D2B_LIVE_SEAL"
          "D2B_LIVE_RECOVERY_ATTESTATION"
          "D2B_LIVE_CUTOVER_RECOVERY"
          "D2B_LIVE_CONSENT"
          "D2B_LIVE_HANDOFF"
          "D2B_LIVE_VERIFICATION"
        ];
      productionValidators =
        lib.hasInfix "delivery wave recovery-import" liveCutoverSource
        && lib.hasInfix "delivery wave seal" liveCutoverSource
        && lib.hasInfix "delivery wave merge-eligibility" liveCutoverSource
        && lib.hasInfix "host cutover preview" liveCutoverSource
        && lib.hasInfix "host cutover apply" liveCutoverSource
        && lib.hasInfix "host cutover verify" liveCutoverSource
        && lib.hasInfix "host cutover doctor" liveCutoverSource;
      stopsBeforeFinalization =
        !(lib.hasInfix "host cutover finalize" liveCutoverSource)
        && !(lib.hasInfix "d2b host finalize" liveCutoverSource);
      noSudoOrShellPredicates =
        !(lib.hasInfix "sudo" liveCutoverSource)
        && !(lib.hasInfix "jq -e" liveCutoverSource)
        && !(lib.hasInfix "grep -q" liveCutoverSource);
      redactedFailureSurface =
        lib.hasInfix "validation failed before mutation" liveCutoverSource
        && lib.hasInfix "raw paths and identities are intentionally suppressed"
          liveCutoverSource;
    };
    expected = {
      selfGuard = true;
      explicitLiveGate = true;
      requiredEvidencePaths = true;
      productionValidators = true;
      stopsBeforeFinalization = true;
      noSudoOrShellPredicates = true;
      redactedFailureSurface = true;
    };
  };
}
