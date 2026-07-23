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
  stateLockAuthority = evaluate manifestReadyPlan
    "adr0045-w8-integration-state-lock-authority-contract"
    [
      "CHANGELOG.md"
      "packages/Cargo.lock"
      "packages/d2b-core/src/storage_lifecycle.rs"
      "packages/d2b-core/src/sync.rs"
      "packages/d2bd/src/storage_lifecycle.rs"
    ];
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
        "secrets-runtime-integration"
        "component-session-service-seam"
        "user-agent-backend-core"
        "user-agent-service-seam"
        "shell-client-core"
        "systemd-user-shell-routing"
        "gateway-replacement"
        "runtime-state-platform-seam"
        "gateway-runtime-integration"
        "provider-parity-proof"
        "provider-parity-fallback-removal"
        "restart-observability-audit"
        "restart-broker-authority"
        "restart-runtime-integration"
      ];
      blockedCount = 17;
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
        "secrets-runtime-integration"
        "user-agent-service-seam"
        "systemd-user-shell-routing"
        "runtime-state-platform-seam"
        "gateway-runtime-integration"
        "provider-parity-fallback-removal"
        "restart-runtime-integration"
      ];
      pendingOnDependencyCount = 8;
      ready = [
        "state-lock-authority-contract"
        "secrets-lifecycle"
        "component-session-service-seam"
        "user-agent-backend-core"
        "shell-client-core"
        "gateway-replacement"
        "provider-parity-proof"
        "restart-observability-audit"
        "restart-broker-authority"
      ];
      readyCount = 9;
      totalComponents = 18;
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
        "secrets-lifecycle"
        "component-session-service-seam"
        "user-agent-backend-core"
        "shell-client-core"
        "gateway-replacement"
        "provider-parity-proof"
        "restart-observability-audit"
        "restart-broker-authority"
      ];
      launchPending = [
        "secrets-authority-seam"
        "secrets-runtime-integration"
        "user-agent-service-seam"
        "systemd-user-shell-routing"
        "runtime-state-platform-seam"
        "gateway-runtime-integration"
        "provider-parity-fallback-removal"
        "restart-runtime-integration"
      ];
    };
  };

  "w8-integration-component-policy/user-agent-client-waits-for-service-seam" = {
    expr = {
      inherit (systemdUser) blockedExternalDependencies unmetDependencies valid;
    };
    expected = {
      blockedExternalDependencies = [ ];
      unmetDependencies = [ "user-agent-service-seam" "shell-client-core" ];
      valid = false;
    };
  };

  "w8-integration-component-policy/exact-forbidden-edit-exceptions-are-component-scoped" = {
    expr = {
      inherit (stateLockAuthority)
        forbiddenViolations
        invalidForbiddenEditExceptions
        violations
        valid
        ;
    };
    expected = {
      forbiddenViolations = [ ];
      invalidForbiddenEditExceptions = [ ];
      violations = [ ];
      valid = true;
    };
  };
}
