{ lib
, pkgs
, system
}:

let
  evalCase = name: testCase:
    let
      value = builtins.deepSeq testCase.expr testCase.expr;
      result =
        if testCase.propagateError or false then
          {
            success = true;
            inherit value;
          }
        else
          builtins.tryEval value;
    in
    if testCase ? expectedError then
      if (builtins.isAttrs testCase.expectedError)
        && (testCase.expectedError != { }) then
        {
          inherit name;
          ok = false;
          detail = "expectedError must be `{ }`";
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

  resultsFor = cases:
    lib.mapAttrsToList evalCase cases;

  evalSurface = { name, cases }:
    let
      results = resultsFor cases;
      failures = lib.filter (result: !result.ok) results;
      report = lib.concatMapStringsSep "\n"
        (result: "FAIL ${result.name}: ${result.detail}") failures;
      total = builtins.length results;
    in builtins.deepSeq results (
    if failures != [ ] then
      throw ''
        ${name} surface FAILED (${toString (builtins.length failures)}/${toString total} cases failed) for ${system}:
        ${report}
      ''
    else {
      inherit total;
      message = "${name}: ${toString total} cases passed";
    });

  mkSurfaceCheck = { name, cases }:
    let
      result = evalSurface { inherit name cases; };
    in
      pkgs.runCommand "d2b-${name}" { } ''
        echo "${result.message}"
        mkdir -p "$out"
        echo ok > "$out/${name}"
      '';
in
{
  inherit evalCase evalSurface resultsFor mkSurfaceCheck;
}
