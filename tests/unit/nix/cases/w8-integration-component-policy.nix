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
  routingPendingPlan = planFor {
    landedComponents = { };
  };
  routingCommit = "0123456789abcdef0123456789abcdef01234567";
  routingLandedPlan = planFor {
    landedComponents = {
      realm-routing-work-executor-fabric = routingCommit;
    };
  };
  secrets = evaluate manifestBlockedPlan
    "adr0045-w8-integration-secrets-lifecycle"
    [ "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs" ];
  gatewayBlocked = evaluate routingPendingPlan
    "adr0045-w8-integration-gateway-replacement"
    [ "packages/d2b-gateway/src/replacement.rs" ];
  gatewayReady = evaluate routingLandedPlan
    "adr0045-w8-integration-gateway-replacement"
    [ "packages/d2b-gateway/src/replacement.rs" ];
  systemdUser = evaluate manifestReadyPlan
    "adr0045-w8-integration-systemd-user-shell-routing"
    [ "packages/d2bd/src/shell_backend.rs" ];
in
{
  "w8-integration-component-policy/manifest-is-a-global-blocker" = {
    expr = {
      inherit (manifestBlockedPlan.launchSummary) blocked blockedCount ready readyCount;
      inherit (secrets) blockedExternalDependencies valid;
    };
    expected = {
      blocked = [
        "state-lock-authority-contract"
        "secrets-authority-seam"
        "secrets-lifecycle"
        "user-agent-service-seam"
        "systemd-user-shell-routing"
        "gateway-replacement"
        "provider-parity-fallback-removal"
        "restart-observability-audit"
      ];
      blockedCount = 8;
      ready = [ ];
      readyCount = 0;
      blockedExternalDependencies = [ "shared-root-w8-manifest-seam" ];
      valid = false;
    };
  };

  "w8-integration-component-policy/current-manifest-releases-independent-components" = {
    expr = basePlan.launchSummary;
    expected = {
      blocked = [ ];
      blockedCount = 0;
      note = basePlan.launchSummary.note;
      pendingOnDependency = [
        "secrets-authority-seam"
        "secrets-lifecycle"
        "systemd-user-shell-routing"
        "provider-parity-fallback-removal"
      ];
      pendingOnDependencyCount = 4;
      ready = [
        "state-lock-authority-contract"
        "user-agent-service-seam"
        "gateway-replacement"
        "restart-observability-audit"
      ];
      readyCount = 4;
      totalComponents = 9;
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
        "state-lock-authority-contract"
        "user-agent-service-seam"
        "gateway-replacement"
        "restart-observability-audit"
      ];
      launchPending = [
        "secrets-authority-seam"
        "secrets-lifecycle"
        "systemd-user-shell-routing"
        "provider-parity-fallback-removal"
      ];
    };
  };

  "w8-integration-component-policy/existing-user-agent-boundary-is-ready" = {
    expr = {
      inherit (systemdUser) blockedExternalDependencies unmetDependencies valid;
    };
    expected = {
      blockedExternalDependencies = [ ];
      unmetDependencies = [ ];
      valid = true;
    };
  };
}
