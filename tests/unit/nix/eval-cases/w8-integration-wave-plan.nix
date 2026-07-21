{ landedComponents ? {
    realm-routing-work-executor-fabric =
      "afd519cfb6aaaa9f8d77d6f4d5002dcbde457fab";
  }
, externalDependenciesOverride ? null
}:

let
  components = {
    "secrets-lifecycle" = {
      branch = "adr0045-w8-integration-secrets-lifecycle";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/rotate-secrets.md"
        "docs/reference/secrets-lifecycle.md"
        "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs"
        "packages/d2b-priv-broker/src/ops/secrets_rotation_audit.rs"
        "packages/d2b-sk-frontend/src/secrets_channel.rs"
        "tests/unit/nix/cases/w8-secrets-lifecycle-eval.nix"
      ];
      reservedPaths = [
        "docs/reference/secrets-lifecycle.md"
        "packages/d2b-priv-broker/src/ops/secrets_lifecycle.rs"
      ];
      deletes = [ ];
      scope = [
        "Provision, rotate, and retire per-realm secrets material (TPM-bound credentials, guest signing keys, security-key channel state) through the existing broker ops surface."
        "Keep rotation audited through the existing broker audit_op.rs op-emission path; do not add a new broker op family without an explicit follow-up."
        "Do not touch swtpm_dir.rs, security_key.rs, or guest_material_* files directly; extend them only through the new owned files."
      ];
      prompt = ''
        Implement realm secrets lifecycle (provision/rotate/retire) in exactly
        the owned files. Route every mutation through the existing broker
        audit-op emission path and existing swtpm/security-key state
        directories without touching them directly. Do not create a new
        broker op enum family, edit packages/d2b-priv-broker/src/runtime.rs,
        packages/d2b-priv-broker/src/lib.rs, or any frozen/global-protected
        path. Do not implement systemd-user shell routing, gateway,
        provider-parity, or restart/observability work here.
      '';
    };

    "systemd-user-shell-routing" = {
      branch = "adr0045-w8-integration-systemd-user-shell-routing";
      dependsOn = [ ];
      externalDependsOn = [
        "workspace-crate-registration-seam"
      ];
      ownedFiles = [
        "docs/explanation/systemd-user-shell-routing.md"
        "packages/d2bd/src/shell_backend.rs"
        "packages/d2bd/src/unsafe_local_helper.rs"
        "packages/d2bd/src/unsafe_local_terminal.rs"
        "packages/d2bd/src/workload_dispatch.rs"
        "packages/d2b-runtime-systemd-user/src/lib.rs"
        "packages/d2b-shell-supervisor/src/lib.rs"
        "packages/d2b-systemd-user-agent/src/lib.rs"
      ];
      reservedPaths = [
        "packages/d2bd/src/shell_backend.rs"
        "packages/d2bd/src/workload_dispatch.rs"
      ];
      deletes = [ ];
      scope = [
        "Route unsafe-local persistent-shell dispatch through a real systemd-user-scoped supervisor instead of the current daemon-owned shell backend stand-in."
        "Keep the existing per-session PTY, output-ring, and attach/detach ownership model from AGENTS.md's 'Unsafe-local persistent shells' row; do not add a root unit, per-VM service, or SSH path."
        "New crates are additive workspace members only; they must not become required for any wave already landed on main."
      ];
      prompt = ''
        Implement systemd-user-scoped shell supervision in exactly the owned
        files, including three new crates
        (d2b-systemd-user-agent, d2b-runtime-systemd-user,
        d2b-shell-supervisor). Do not create these crate directories or their
        Cargo.toml files until the workspace-crate-registration-seam external
        dependency is ready (packages/Cargo.toml membership +
        packages/Cargo.lock regeneration land through shared-root/integrator
        prep first, never through this component). Preserve the existing
        session-table admission and audit shape in shell_backend.rs and
        workload_dispatch.rs; do not add a root service, per-VM unit, or
        direct compositor fallback.
      '';
    };

    "realm-routing-work-executor-fabric" = {
      branch = "adr0045-w8-integration-realm-routing-work-executor-fabric";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/realm-work-executor.md"
        "packages/d2b-exec-runner/src/service_mode.rs"
        "packages/d2b-exec-runner/src/spec.rs"
        "packages/d2b-realm-router/src/execution.rs"
        "packages/d2b-realm-router/src/remote_node.rs"
        "packages/d2b-realm-router/src/session_lifecycle.rs"
        "packages/d2b-realm-router/src/target_resolver.rs"
        "packages/d2b-realm-router/src/work_executor.rs"
        "packages/d2b-realm-transport/src/fabric.rs"
        "packages/d2b-realm-transport/src/local_tcp.rs"
      ];
      reservedPaths = [
        "packages/d2b-realm-router/src/work_executor.rs"
        "packages/d2b-realm-transport/src/fabric.rs"
      ];
      deletes = [ ];
      scope = [
        "Implement the integrated realm routing target resolution, remote-node dispatch, and shared transport fabric that W5/W6/W7's per-wave stand-ins approximated separately."
        "Keep realm relay/session/provider credentials inside the gateway guest boundary per ADR 0032; the router/fabric never holds them locally."
        "Do not touch packages/d2b-realm-core/src/allocator.rs or allocator_engine.rs; consume the allocator's existing typed API only."
      ];
      prompt = ''
        Implement the integrated realm work executor and shared transport
        fabric in exactly the owned files. Build one coherent
        routing/dispatch/transport surface across d2b-realm-router,
        d2b-realm-transport, and d2b-exec-runner. Do not add realm relay or
        provider credentials to the host daemon/broker, do not edit the
        allocator engine, and do not implement gateway, secrets, provider
        parity, or restart/observability work here.
      '';
    };

    "gateway-replacement" = {
      branch = "adr0045-w8-integration-gateway-replacement";
      dependsOn = [ "realm-routing-work-executor-fabric" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/explanation/gateway-replacement.md"
        "packages/d2b-gateway-runtime/src/aca_workload.rs"
        "packages/d2b-gateway-runtime/src/production.rs"
        "packages/d2b-gateway-runtime/src/provider_agent.rs"
        "packages/d2b-gateway/src/audit.rs"
        "packages/d2b-gateway/src/ledger.rs"
        "packages/d2b-gateway/src/orchestrator.rs"
        "packages/d2b-gateway/src/replacement.rs"
      ];
      reservedPaths = [
        "packages/d2b-gateway/src/replacement.rs"
        "packages/d2b-gateway/src/orchestrator.rs"
      ];
      deletes = [ ];
      scope = [
        "Replace the W6/W7-era gateway orchestration stand-in with the integrated per-realm gateway guest model consuming the W8 routing/fabric surface."
        "Keep gateway-held relay/provider credentials and remote node registries entirely inside the gateway guest boundary per ADR 0032."
        "Consume packages/d2b-realm-router and packages/d2b-realm-transport through their existing public API only; do not fork or duplicate routing logic here."
      ];
      prompt = ''
        Implement the replacement gateway orchestration in exactly the owned
        files, consuming the realm-routing-work-executor-fabric component's
        landed API. Do not add a second gateway credential store, do not map
        relay identity to local Admin authority, and do not edit
        packages/d2bd/src/lib.rs, provider_effects.rs, or provider_registry.rs.
      '';
    };

    "provider-parity-fallback-removal" = {
      branch = "adr0045-w8-integration-provider-parity-fallback-removal";
      dependsOn = [ "gateway-replacement" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/verify-provider-parity.md"
        "packages/d2b-realm-provider/src/mock.rs"
        "packages/d2b-realm-provider/src/parity.rs"
        "packages/d2bd/src/realm_stubs.rs"
      ];
      reservedPaths = [
        "packages/d2b-realm-provider/src/parity.rs"
        "packages/d2bd/src/realm_stubs.rs"
      ];
      deletes = [ ];
      scope = [
        "Prove the integrated provider surface has full parity with every W5/W6/W7 per-wave provider stand-in before removing daemon-side fallback/stub wiring."
        "Delete only the daemon-side realm_stubs.rs fallback paths once the replacement gateway (gateway-replacement) and provider parity checks are both green."
        "Do not edit packages/d2bd/src/provider_effects.rs or provider_registry.rs; those stay shared-root/integrator territory."
      ];
      prompt = ''
        Implement provider parity verification and daemon fallback removal in
        exactly the owned files, after the gateway-replacement component has
        landed. Prove parity before deleting realm_stubs.rs fallback paths;
        do not remove a fallback that still has a live daemon call site. Do
        not edit provider_effects.rs, provider_registry.rs, or any frozen
        provider crate (packages/d2b-provider-*).
      '';
    };

    "restart-observability-audit" = {
      branch = "adr0045-w8-integration-restart-observability-audit";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/explanation/restart-observability-audit.md"
        "packages/d2b-daemon-access/src/relay.rs"
        "packages/d2bd/src/daemon_audit.rs"
        "packages/d2bd/src/observability_export.rs"
        "packages/d2bd/src/provider_shutdown.rs"
        "packages/d2bd/src/restart_continuity.rs"
        "packages/d2bd/src/storage_lifecycle.rs"
      ];
      reservedPaths = [
        "packages/d2bd/src/restart_continuity.rs"
        "packages/d2bd/src/storage_lifecycle.rs"
      ];
      deletes = [ ];
      scope = [
        "Integrate restart-adoption continuity (pidfd re-verification, quarantine/degrade reporting), observability export, and audit shape across the merged W5/W6/W7 daemon surface."
        "Treat normal daemon restarts as continuation events per ADR 0034: never broad-sweep /run/d2b state, and never persist pidfd authority across process lifetimes."
        "Keep metric/audit label cardinality and redaction rules from AGENTS.md's critical-subsystems table; no secrets, argv, cmd output, or store paths in telemetry."
      ];
      prompt = ''
        Implement the integrated restart-continuity, observability, and audit
        surface in exactly the owned files. Preserve continuation-event
        semantics for daemon restarts, keep pidfd authority unpersisted, and
        keep telemetry label/redaction rules intact. Do not edit
        packages/d2bd/src/lib.rs, packages/d2b-priv-broker/src/runtime.rs, or
        any frozen/global-protected path.
      '';
    };
  };

  hasPrefix = prefix: value:
    builtins.stringLength value >= builtins.stringLength prefix
    && builtins.substring 0 (builtins.stringLength prefix) value == prefix;
  inventoryFor = exactPaths: prefixes:
    builtins.filter
      (row: row.paths != [ ] || row.reservedPaths != [ ])
      (builtins.map
        (owner: {
          inherit owner;
          paths = builtins.filter
            (path:
              builtins.elem path exactPaths
              || builtins.any (prefix: hasPrefix prefix path) prefixes)
            components.${owner}.ownedFiles;
          reservedPaths = builtins.filter
            (path:
              builtins.elem path exactPaths
              || builtins.any (prefix: hasPrefix prefix path) prefixes)
            components.${owner}.reservedPaths;
        })
        componentOrder);

  componentOrder = [
    "secrets-lifecycle"
    "systemd-user-shell-routing"
    "realm-routing-work-executor-fabric"
    "gateway-replacement"
    "provider-parity-fallback-removal"
    "restart-observability-audit"
  ];

  globalExternalDependencies = [
    "shared-root-w8-manifest-seam"
  ];

  baseExternalDependencies = {
    workspace-crate-registration-seam = {
      owner = "adr0045-w8-integration";
      status = "blocked";
      requiredRebase = true;
      contractFiles = [
        "packages/Cargo.toml"
        "packages/Cargo.lock"
      ];
      acceptance = [
        "packages/Cargo.toml workspace members add d2b-systemd-user-agent, d2b-runtime-systemd-user, and d2b-shell-supervisor in base-first sorted order."
        "packages/Cargo.lock is regenerated for the new members by the shared-root/integrator, never by the component branch."
        "The systemd-user-shell-routing branch is rebased onto the accepted crate-registration commit before creating any new crate directory."
      ];
    };
    shared-root-w8-manifest-seam = {
      owner = "adr0045-w8-integration";
      status = "ready";
      requiredRebase = false;
      acceptance = [
        "delivery/manifests/w8.json exists, fingerprints itself, and declares wave = \"w8\" once the draft PR number is known."
        "The manifest is created by the integrator immediately after opening the draft PR, never speculatively during phase A."
        "cargo xtask wave-policy check --candidate-root <worktree> succeeds end-to-end for the w8 branch only after this manifest lands."
      ];
    };
  };
  externalDependencies =
    if externalDependenciesOverride == null
    then baseExternalDependencies
    else externalDependenciesOverride;

  pathExternalDependencies = [
    {
      dependency = "workspace-crate-registration-seam";
      paths = [
        "packages/d2b-runtime-systemd-user/src/lib.rs"
        "packages/d2b-shell-supervisor/src/lib.rs"
        "packages/d2b-systemd-user-agent/src/lib.rs"
      ];
    }
  ];

  readyComponents = builtins.filter
    (name:
      !(builtins.hasAttr name landedComponents)
      && builtins.all
        (dependency: builtins.hasAttr dependency landedComponents)
        components.${name}.dependsOn
      && builtins.all
        (dependency: externalDependencies.${dependency}.status == "ready")
        (globalExternalDependencies ++ components.${name}.externalDependsOn))
    componentOrder;
  blockedComponents = builtins.filter
    (name:
      !(builtins.hasAttr name landedComponents)
      && builtins.any
        (dependency: externalDependencies.${dependency}.status != "ready")
        (globalExternalDependencies ++ components.${name}.externalDependsOn))
    componentOrder;
  pendingComponents = builtins.filter
    (name:
      !(builtins.hasAttr name landedComponents)
      && builtins.any
        (dependency: !(builtins.hasAttr dependency landedComponents))
        components.${name}.dependsOn
      && !(builtins.elem name blockedComponents))
    componentOrder;
in
{
  schemaVersion = 1;
  wave = "w8";
  sharedRoot = "5ba02876";
  branch = "adr0045-w8-integration";
  pullRequestBase = "main";
  inherit componentOrder components;

  dispatch = {
    trustedBranch = "adr0045-w8-integration";
    gate =
      "tests/unit/nix/tools/w8-integration-component-diff.sh --candidate-root <component-worktree>";
    commonPrompt = ''
      Start from the current clean adr0045-w8-integration prep head, use the
      component's exact branch and ownedFiles, and do not edit another
      component's files, a reserved shared integration sink
      (packages/d2bd/src/lib.rs, packages/d2b-priv-broker/src/runtime.rs), or
      any forbiddenEdits path. Commit before validation, then run the
      component diff gate from the trusted prep worktree. A blocked external
      dependency is a hard stop, not permission to edit shared-root, W4-W7
      frozen implementation, or Cargo workspace files.
    '';
  };

  launchSummary = {
    totalComponents = builtins.length componentOrder;
    readyCount = builtins.length readyComponents;
    ready = readyComponents;
    blockedCount = builtins.length blockedComponents;
    blocked = blockedComponents;
    pendingOnDependencyCount = builtins.length pendingComponents;
    pendingOnDependency = pendingComponents;
    note = ''
      Ready components may launch immediately in separate worktrees against
      the file-overlap graph below. Blocked components require the named
      external dependency to reach status = "ready" first. Pending components
      are not externally blocked but must wait for their dependsOn component
      to land because they consume its landed module surface directly
      (gateway-replacement consumes the routing/fabric API;
      provider-parity-fallback-removal proves parity against the replaced
      gateway before deleting fallback code).
    '';
  };

  affectedInventory = {
    docs = inventoryFor [ ] [
      "docs/explanation/"
      "docs/how-to/"
      "docs/reference/"
    ];
    tests = inventoryFor [ ] [
      "tests/unit/nix/"
    ];
  };

  inherit
    externalDependencies
    globalExternalDependencies
    landedComponents
    pathExternalDependencies
    ;

  futureSharedContractPrepNeeds = {
    note = ''
      These crates are referenced by allowed/foreign prefixes already in
      delivery/shared-contracts.json (inherited by w8 through
      inherits_prefixes_from) but do not exist on disk and are not workspace
      members today. Phase A intentionally does not create them, edit
      packages/Cargo.toml, or edit packages/Cargo.lock. Before
      systemd-user-shell-routing (or any future component needing them) can
      begin, the integrator/shared-root must:
        1. Add each crate as a packages/Cargo.toml workspace member in the
           existing base-first sorted position.
        2. Scaffold each crate's own Cargo.toml + src/lib.rs stub.
        3. Regenerate packages/Cargo.lock for the new members.
        4. Flip workspace-crate-registration-seam to status = "ready" after the
           registration commit lands on the trusted W8 root, then have the
           component rebase onto it.
    '';
    notYetCreatedCrates = [
      "packages/d2b-activation-helper/"
      "packages/d2b-one-shot-helper/"
      "packages/d2b-provider-agent/"
      "packages/d2b-runtime-systemd-user/"
      "packages/d2b-security-key-helper/"
      "packages/d2b-shell-supervisor/"
      "packages/d2b-systemd-user-agent/"
      "packages/d2b-tty-helper/"
      "packages/d2b-wlcontrol/"
    ];
    requiredForThisWave = [
      "packages/d2b-runtime-systemd-user/"
      "packages/d2b-shell-supervisor/"
      "packages/d2b-systemd-user-agent/"
    ];
  };

  forbiddenEdits = [
    "delivery/manifests/w5.json"
    "delivery/manifests/w6.json"
    "delivery/manifests/w7.json"
    "delivery/manifests/w8.json"
    "delivery/shared-contracts.json"
    "docs/adr/0045-provider-and-transport-framework.md"
    "docs/reference/v2-provider-implementations.md"
    "packages/Cargo.lock"
    "packages/Cargo.toml"
    "packages/d2b-contracts/"
    "packages/d2b-core/"
    "packages/d2b-priv-broker/src/runtime.rs"
    "packages/d2b-realm-core/src/allocator.rs"
    "packages/d2b-realm-core/src/allocator_engine.rs"
    "packages/d2bd/src/lib.rs"
    "packages/d2bd/src/provider_effects.rs"
    "packages/d2bd/src/provider_registry.rs"
    "packages/xtask/"
  ];
}
