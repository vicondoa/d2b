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
}
