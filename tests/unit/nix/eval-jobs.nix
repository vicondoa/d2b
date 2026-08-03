{ lib
, pkgs
, system
, flakeRoot
, d2bLib
, mkEval ? null
, nixpkgsFlake ? null
, d2bModule ? null
}:

let
  ctx = {
    inherit lib pkgs system flakeRoot d2bLib mkEval nixpkgsFlake d2bModule;
  };

  casesFor = caseFileNames:
    import ./default.nix (ctx // { inherit caseFileNames; });

  evalCase = name: testCase:
    let
      result = builtins.tryEval (
        let value = testCase.expr;
        in builtins.deepSeq value value
      );
    in
    if testCase ? expectedError then
      # `tryEval` can assert that an expression throws, but cannot match its
      # message. Message-sensitive negative cases assert over
      # `config.assertions` data instead.
      if (builtins.isAttrs testCase.expectedError)
        && (testCase.expectedError != { }) then
        {
          inherit name;
          ok = false;
          detail = "expectedError must be `{ }` - this runner asserts only THAT the expr throws; tryEval cannot match a throw message. Move message-substring checks to config.assertions data.";
        }
      else
        {
          inherit name;
          ok = !result.success;
          detail =
            if result.success
            then "expected an error, but eval succeeded"
            else "threw as expected";
        }
    else
      {
        inherit name;
        ok = result.success && result.value == testCase.expected;
        detail =
          if !result.success
          then "eval threw; expected a value"
          else "got=${builtins.toJSON result.value} expected=${builtins.toJSON testCase.expected}";
      };

  resultsFor = cases: lib.mapAttrsToList evalCase cases;

  # Keep every aggregate evaluator on the same case construction, result
  # ordering, and failure report. The flake's topical checks and the
  # nix-eval-jobs file surface both call this constructor.
  mkAggregateCheck = checkName: caseFileNames:
    let
      cases = casesFor caseFileNames;
      results = resultsFor cases;
      failures = lib.filter (result: !result.ok) results;
      report = lib.concatMapStringsSep "\n"
        (result: "FAIL ${result.name}: ${result.detail}") failures;
      total = builtins.length results;
    in
    if failures != [ ] then
      throw ''
        ${checkName} gate FAILED (${toString (builtins.length failures)}/${toString total} cases failed) for ${system}:
        ${report}
      ''
    else
      pkgs.runCommand "d2b-${checkName}" { } ''
        echo "${checkName}: ${toString total} cases passed"
        mkdir -p "$out"
        echo ok > "$out/${checkName}"
      '';

  # `attrNames` forces only the corpus shape needed to discover names. The
  # case expressions remain thunks, so the separate flake inventory can be
  # evaluated without traversing the assertion values.
  caseNamesFor = caseFileNames:
    builtins.sort builtins.lessThan (lib.attrNames (casesFor caseFileNames));

  caseFileNames =
    builtins.sort builtins.lessThan
      (lib.filter (name: lib.hasSuffix ".nix" name)
        (lib.attrNames (builtins.readDir ./cases)));
  fileJobName = caseFileName:
    "case-${lib.removeSuffix ".nix" caseFileName}";
  fileJobs =
    builtins.listToAttrs (map (caseFileName: {
      name = fileJobName caseFileName;
      value = mkAggregateCheck (fileJobName caseFileName) [ caseFileName ];
    }) caseFileNames);
  jobNames = builtins.sort builtins.lessThan (lib.attrNames fileJobs);
in
{
  inherit casesFor caseNamesFor evalCase resultsFor mkAggregateCheck
    caseFileNames fileJobName fileJobs jobNames;
  cases = casesFor null;
  caseNames = caseNamesFor null;
}
