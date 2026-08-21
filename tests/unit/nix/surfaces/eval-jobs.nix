{ lib, pkgs, system, ... }:

let
  evaluator = import ../eval-jobs.nix {
    inherit lib pkgs system;
  };
in
{
  cases = {
    "eval-jobs/empty-raw-cases-fail" = {
      expr = evaluator.evalSurface {
        name = "empty-raw";
        cases = { };
      };
      expectedError = { };
    };

    "eval-jobs/non-empty-raw-cases-pass" = {
      expr = evaluator.evalSurface {
        name = "ok-raw";
        cases = {
          ok = {
            expr = true;
            expected = true;
          };
        };
      };
      expected = {
        total = 1;
        message = "ok-raw: 1 cases passed";
      };
    };
  };
}
