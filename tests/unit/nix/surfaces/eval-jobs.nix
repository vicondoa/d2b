{ lib, pkgs, system, ... }:

let
  evaluator = import ../eval-jobs.nix {
    inherit lib pkgs system;
  };
  selectCases = import ../helpers/select-cases.nix {
    context = { };
    surfaceName = "eval-jobs";
    inherit lib;
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

    "eval-jobs/missing-selected-case-fails" = {
      expr = selectCases.selectCaseFiles [
        {
          path = ../cases/selection-helper.nix;
          names = [ "selection/missing" ];
        }
      ];
      expectedError = { };
    };

    "eval-jobs/empty-explicit-selection-fails" = {
      expr = selectCases.selectCaseFiles [
        {
          path = ../cases/selection-helper.nix;
          names = [ ];
        }
      ];
      expectedError = { };
    };

    "eval-jobs/complete-selection-filters-cases" = {
      expr = builtins.attrNames (builtins.head (selectCases.selectCaseFiles [
        {
          path = ../cases/selection-helper.nix;
          names = [
            "selection/alpha"
            "selection/beta"
          ];
        }
      ]));
      expected = [
        "selection/alpha"
        "selection/beta"
      ];
    };

    "eval-jobs/structural-only-selection-remains-empty" = {
      expr = selectCases.selectCaseFiles [ ];
      expected = [ ];
    };

    "eval-jobs/selection-diagnostic-is-stable" = {
      expr = selectCases.formatSelectionError {
        path = ../cases/selection-helper.nix;
        missingNames = [
          "selection/missing"
          "selection/renamed"
        ];
      };
      expected = "eval-jobs surface case file selection-helper.nix missing requested names: selection/missing, selection/renamed";
    };
  };
}
