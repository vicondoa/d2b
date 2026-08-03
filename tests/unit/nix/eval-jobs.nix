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

  # `attrNames` forces only the corpus shape needed to discover names. The
  # case expressions remain thunks, so the separate flake inventory can be
  # evaluated without traversing the assertion values.
  caseNamesFor = caseFileNames:
    builtins.sort builtins.lessThan (lib.attrNames (casesFor caseFileNames));
in
{
  inherit casesFor caseNamesFor evalCase resultsFor;
  cases = casesFor null;
  caseNames = caseNamesFor null;
}
