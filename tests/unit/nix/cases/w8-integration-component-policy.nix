{ flakeRoot, ... }:

let
  planFor = args: import
    (flakeRoot + "/tests/unit/nix/eval-cases/w8-integration-wave-plan.nix")
    args;
  basePlan = planFor { };
  componentPolicy = import
    (flakeRoot + "/tests/unit/nix/eval-cases/w8-integration-component-policy.nix");
  evaluate = plan: branch: paths:
    componentPolicy {
      inherit branch plan;
      pathsJson = builtins.toJSON paths;
    };
  manifestBlockedDependencies = basePlan.externalDependencies // {
    shared-root-w8-manifest-seam =
      basePlan.externalDependencies.shared-root-w8-manifest-seam
      // { status = "blocked"; };
  };
  manifestBlockedPlan = planFor {
    externalDependenciesOverride = manifestBlockedDependencies;
  };
  manifestReadyPlan = basePlan;
  routingCommit = "0123456789abcdef0123456789abcdef01234567";
  routingLandedPlan = planFor {
    landedComponents = {
      realm-routing-work-executor-fabric = routingCommit;
    };
  };
  secrets = evaluate manifestBlockedPlan
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
      inherit (manifestBlockedPlan.launchSummary) blocked blockedCount ready readyCount;
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

  "w8-integration-component-policy/current-manifest-releases-independent-components" = {
    expr = basePlan.launchSummary;
    expected = {
      blocked = [ "systemd-user-shell-routing" ];
      blockedCount = 1;
      note = basePlan.launchSummary.note;
      pendingOnDependency = [
        "provider-parity-fallback-removal"
      ];
      pendingOnDependencyCount = 1;
      ready = [
        "secrets-lifecycle"
        "gateway-replacement"
        "restart-observability-audit"
      ];
      readyCount = 3;
      totalComponents = 6;
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
      launchReady = routingLandedPlan.launchSummary.ready;
      launchPending = routingLandedPlan.launchSummary.pendingOnDependency;
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
      launchReady = [
        "secrets-lifecycle"
        "gateway-replacement"
        "restart-observability-audit"
      ];
      launchPending = [ "provider-parity-fallback-removal" ];
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
