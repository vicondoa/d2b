{ flakeRoot, ... }:

let
  basePlan = import
    (flakeRoot + "/tests/unit/nix/eval-cases/w8-integration-wave-plan.nix");
  componentPolicy = import
    (flakeRoot + "/tests/unit/nix/eval-cases/w8-integration-component-policy.nix");
  evaluate = plan: branch: paths:
    componentPolicy {
      inherit branch plan;
      pathsJson = builtins.toJSON paths;
    };
  manifestReadyPlan = basePlan // {
    externalDependencies = basePlan.externalDependencies // {
      shared-root-w8-manifest-seam =
        basePlan.externalDependencies.shared-root-w8-manifest-seam
        // { status = "ready"; };
    };
  };
  routingCommit = "0123456789abcdef0123456789abcdef01234567";
  routingLandedPlan = manifestReadyPlan // {
    landedComponents = {
      realm-routing-work-executor-fabric = routingCommit;
    };
  };
  secrets = evaluate basePlan
    "adr0045-w8-integration-secrets-lifecycle"
    [ "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs" ];
  gatewayBlocked = evaluate manifestReadyPlan
    "adr0045-w8-integration-gateway-replacement"
    [ "packages/d2b-gateway/src/replacement.rs" ];
  gatewayReady = evaluate routingLandedPlan
    "adr0045-w8-integration-gateway-replacement"
    [ "packages/d2b-gateway/src/replacement.rs" ];
  systemdUser = evaluate manifestReadyPlan
    "adr0045-w8-integration-systemd-user-shell-routing"
    [ "packages/d2b-systemd-user-agent/src/lib.rs" ];
in
{
  "w8-integration-component-policy/manifest-is-a-global-blocker" = {
    expr = {
      inherit (basePlan.launchSummary) blocked blockedCount ready readyCount;
      inherit (secrets) blockedExternalDependencies valid;
    };
    expected = {
      blocked = basePlan.componentOrder;
      blockedCount = 6;
      ready = [ ];
      readyCount = 0;
      blockedExternalDependencies = [ "shared-root-w8-manifest-seam" ];
      valid = false;
    };
  };

  "w8-integration-component-policy/internal-dependency-must-land" = {
    expr = {
      inherit (gatewayBlocked)
        blockedExternalDependencies
        landedDependencyCommits
        unmetDependencies
        valid
        ;
    };
    expected = {
      blockedExternalDependencies = [ ];
      landedDependencyCommits = [ ];
      unmetDependencies = [ "realm-routing-work-executor-fabric" ];
      valid = false;
    };
  };

  "w8-integration-component-policy/landed-dependency-releases-component" = {
    expr = {
      inherit (gatewayReady)
        blockedExternalDependencies
        invalidLandedDependencies
        landedDependencyCommits
        unmetDependencies
        valid
        ;
    };
    expected = {
      blockedExternalDependencies = [ ];
      invalidLandedDependencies = [ ];
      landedDependencyCommits = [
        {
          dependency = "realm-routing-work-executor-fabric";
          commit = routingCommit;
        }
      ];
      unmetDependencies = [ ];
      valid = true;
    };
  };

  "w8-integration-component-policy/workspace-seam-remains-independent" = {
    expr = {
      inherit (systemdUser) blockedExternalDependencies unmetDependencies valid;
    };
    expected = {
      blockedExternalDependencies = [ "workspace-crate-registration-seam" ];
      unmetDependencies = [ ];
      valid = false;
    };
  };
}
