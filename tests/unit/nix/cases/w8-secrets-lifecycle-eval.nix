{ flakeRoot, ... }:

let
  plan = import
    (flakeRoot + "/tests/unit/nix/eval-cases/w8-integration-wave-plan.nix")
    { };
  componentPolicy = import
    (flakeRoot + "/tests/unit/nix/eval-cases/w8-integration-component-policy.nix");
  branch = "adr0045-w8-integration-secrets-lifecycle";
  evaluate = paths:
    componentPolicy {
      inherit branch plan;
      pathsJson = builtins.toJSON paths;
    };
  component = plan.components."secrets-lifecycle";
  ownedFiles = [
    "docs/how-to/rotate-secrets.md"
    "docs/reference/secrets-lifecycle.md"
    "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs"
    "packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs"
    "packages/d2b-sk-frontend/src/secrets_channel.rs"
    "tests/unit/nix/cases/w8-secrets-lifecycle-eval.nix"
  ];
  fullCommit = evaluate ownedFiles;
  partialCommit = evaluate [
    "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs"
    "packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs"
  ];
  unownedPath = evaluate (ownedFiles ++ [
    "packages/d2b-priv-broker/src/ops/mod.rs"
  ]);
  forbiddenAndUnowned = evaluate [
    "packages/d2b-priv-broker/src/runtime.rs"
  ];
  wrongBranch = componentPolicy {
    branch = "adr0045-w8-integration-gateway-replacement";
    inherit plan;
    pathsJson = builtins.toJSON ownedFiles;
  };
in
{
  "w8-secrets-lifecycle-eval/component-metadata-matches-the-plan" = {
    expr = {
      inherit (component) dependsOn externalDependsOn ownedFiles reservedPaths deletes;
    };
    expected = {
      dependsOn = [ ];
      externalDependsOn = [ ];
      inherit ownedFiles;
      reservedPaths = [
        "docs/reference/secrets-lifecycle.md"
        "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs"
      ];
      deletes = [ ];
    };
  };

  "w8-secrets-lifecycle-eval/component-is-ready-under-the-current-manifest" = {
    expr = builtins.elem "secrets-lifecycle" plan.launchSummary.ready;
    expected = true;
  };

  "w8-secrets-lifecycle-eval/branch-resolves-to-this-component" = {
    expr = fullCommit.component;
    expected = "secrets-lifecycle";
  };

  "w8-secrets-lifecycle-eval/full-owned-file-commit-is-valid" = {
    expr = {
      inherit (fullCommit) valid violations forbiddenViolations unmetDependencies;
      inherit (fullCommit) blockedExternalDependencies;
    };
    expected = {
      valid = true;
      violations = [ ];
      forbiddenViolations = [ ];
      unmetDependencies = [ ];
      blockedExternalDependencies = [ ];
    };
  };

  "w8-secrets-lifecycle-eval/partial-owned-subset-commit-is-still-valid" = {
    expr = {
      inherit (partialCommit) valid violations;
    };
    expected = {
      valid = true;
      violations = [ ];
    };
  };

  "w8-secrets-lifecycle-eval/unowned-path-is-a-violation" = {
    expr = {
      inherit (unownedPath) valid violations;
    };
    expected = {
      valid = false;
      violations = [ "packages/d2b-priv-broker/src/ops/mod.rs" ];
    };
  };

  "w8-secrets-lifecycle-eval/runtime-rs-is-both-unowned-and-forbidden" = {
    expr = {
      inherit (forbiddenAndUnowned) valid violations forbiddenViolations;
    };
    expected = {
      valid = false;
      violations = [ "packages/d2b-priv-broker/src/runtime.rs" ];
      forbiddenViolations = [ "packages/d2b-priv-broker/src/runtime.rs" ];
    };
  };

  "w8-secrets-lifecycle-eval/owned-files-never-resolve-under-the-wrong-branch" = {
    expr = {
      inherit (wrongBranch) valid component;
      violations = wrongBranch.violations;
    };
    expected = {
      valid = false;
      component = "gateway-replacement";
      violations = ownedFiles;
    };
  };
}
