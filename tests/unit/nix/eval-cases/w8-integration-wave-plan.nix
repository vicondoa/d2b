{ landedComponents ? {
    realm-routing-work-executor-fabric =
      "60bfa39d4664fc111f585191e39b6b8a0441450a";
  }
, externalDependenciesOverride ? null
}:

let
  components = {
    "state-lock-authority-contract" = {
      branch = "adr0045-w8-integration-state-lock-authority-contract";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/schemas/v2/sync.json"
        "docs/reference/schemas/v2/sync.md"
        "docs/reference/schemas/v2/storage-lifecycle-report.json"
        "docs/reference/schemas/v2/storage-lifecycle-report.md"
        "nixos-modules/realm-storage-rows.nix"
        "packages/Cargo.lock"
        "packages/d2b-contract-tests/tests/storage_sync_contracts.rs"
        "packages/d2b/src/doctor.rs"
        "packages/d2b-priv-broker/src/ops/storage_contract.rs"
        "packages/d2b-core/src/storage_lifecycle.rs"
        "packages/d2b-core/src/sync.rs"
        "packages/d2b-state/Cargo.toml"
        "packages/d2b-state/src/atomic.rs"
        "packages/d2b-state/src/lock.rs"
        "packages/d2b-state/src/path.rs"
        "packages/d2bd/src/storage_lifecycle.rs"
      ];
      reservedPaths = [
        "packages/d2b-core/src/sync.rs"
        "packages/d2b-state/src/lock.rs"
        "packages/d2b-state/src/path.rs"
      ];
      forbiddenEditExceptions = [
        "packages/Cargo.lock"
        "packages/d2b-core/src/storage_lifecycle.rs"
        "packages/d2b-core/src/sync.rs"
      ];
      deletes = [ ];
      scope = [
        "Make the generated sync row losslessly consumable by d2b-state without invented runtime policy or a second lock namespace."
        "Bind the exact opened lock FD and guarded resource directory identities to one non-forgeable LockGuard capability."
        "Provide anchored no-symlink/no-magic-link path resolution and durable directory creation for later secrets, gateway, and restart authority seams."
      ];
      prompt = ''
        Reconcile the generated d2b-core sync contract with d2b-state's runtime
        lock authority in exactly the owned files. Every runtime field must be
        generated or losslessly derived; never invent order, dependencies,
        cancellation, deadlines, authority, metadata, or resource identity.
        Expose the exact held lock-fd identity and a non-forgeable
        guard-bound resource capability. Resolve generated paths beneath a
        trusted anchor with openat2 no-symlink/no-magic-link semantics. Add
        durable mkdir plus parent fsync. Do not implement secrets, gateway, or
        restart behavior.
      '';
    };

    "secrets-authority-seam" = {
      branch = "adr0045-w8-integration-secrets-authority-seam";
      dependsOn = [ "state-lock-authority-contract" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/secrets-authority.md"
        "nixos-modules/realm-storage-rows.nix"
        "packages/d2b-contract-tests/tests/storage_sync_contracts.rs"
        "packages/d2b-priv-broker/src/ops/mod.rs"
        "packages/d2b-priv-broker/src/ops/secrets_authority.rs"
        "packages/d2b-state/src/lib.rs"
        "packages/d2b-state/src/secret.rs"
      ];
      reservedPaths = [
        "packages/d2b-priv-broker/src/ops/mod.rs"
        "packages/d2b-state/src/secret.rs"
        "packages/d2b-priv-broker/src/ops/secrets_authority.rs"
      ];
      deletes = [ ];
      scope = [
        "Freeze the typed generated authority consumed by secrets lifecycle: WorkloadId, pre-opened AnchoredDir, the existing workload-keys storage/lock rows, caller-held d2b-state LockGuard, and ownership epoch."
        "Add the missing bounded zeroizing secret-leaf metadata/read primitive to d2b-state instead of duplicating locks, atomic JSON, generation fencing, or quarantine."
        "Do not implement lifecycle provision/rotate/rollback/retire behavior in this component."
      ];
      prompt = ''
        Implement the secrets authority seam in exactly the owned files. Reuse
        the existing generated workload-keys storage path and OFD lock when
        they satisfy the contract; change realm-storage-rows.nix only if an
        invariant is genuinely missing. Expose a typed pre-opened authority
        bound to WorkloadId, AnchoredDir, caller-held LockGuard, resource id,
        and ownership epoch. Add the bounded zeroizing secret-leaf primitive
        to d2b-state. Do not implement lifecycle operations or add a private
        lock/path namespace.
      '';
    };

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
        "Implement the pure provision/rotate/rollback/retire transaction state machine and closed audit/channel vocabulary behind an injected authority port."
        "Use DurableState generation/checksum/ownership-epoch fences conceptually; do not implement raw paths, locks, atomic JSON, or broker dispatch here."
        "Keep tests hermetic so the transaction core can proceed while the authority seam lands."
      ];
      prompt = ''
        Implement the pure secrets transaction core in exactly the owned
        files, using an injected authority trait and fake state port for tests.
        Retain the closed audit/channel contracts but delete hand-rolled path,
        lock, atomic JSON, and recovery authority. Do not wire broker dispatch
        or generated storage in this component.
      '';
    };

    "secrets-runtime-integration" = {
      branch = "adr0045-w8-integration-secrets-runtime-integration";
      dependsOn = [ "secrets-lifecycle" "secrets-authority-seam" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "packages/d2b-priv-broker/src/ops/audit_op.rs"
        "packages/d2b-priv-broker/src/ops/mod.rs"
        "packages/d2b-priv-broker/src/runtime.rs"
        "packages/d2b-sk-frontend/src/lib.rs"
      ];
      reservedPaths = [
        "packages/d2b-priv-broker/src/runtime.rs"
      ];
      forbiddenEditExceptions = [
        "packages/d2b-priv-broker/src/runtime.rs"
      ];
      deletes = [ ];
      scope = [
        "Wire the accepted secrets transaction core to the generated authority, broker dispatch, audit sink, startup recovery, and guest channel."
        "No lifecycle semantics or storage authority are invented here."
      ];
      prompt = ''
        Integrate the landed secrets transaction and authority components in
        exactly the owned files. Wire broker dispatch/audit/startup recovery
        and guest channel registration without raw paths or a second lock.
      '';
    };

    "component-session-service-seam" = {
      branch = "adr0045-w8-integration-component-session-service-seam";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/component-session-v2-vectors.json"
        "packages/d2b-client/src/client.rs"
        "packages/d2b-client/src/host_socket.rs"
        "packages/d2b-contracts/src/v2_component_session.rs"
        "packages/d2b-contracts/src/v2_services.rs"
        "packages/d2b-session/src/inbound_call.rs"
        "packages/d2b-session/src/cancellation.rs"
        "packages/d2b-session/src/driver.rs"
        "packages/d2b-session/src/engine.rs"
        "packages/d2b-session/src/lib.rs"
        "packages/d2b-session/src/server.rs"
        "packages/d2b-session-unix/src/adapter.rs"
        "packages/d2b-session-unix/src/descriptor.rs"
        "packages/d2b-session-unix/src/lib.rs"
      ];
      reservedPaths = [
        "packages/d2b-contracts/src/v2_component_session.rs"
        "packages/d2b-session/src/engine.rs"
        "packages/d2b-session/src/server.rs"
        "packages/d2b-session-unix/src/adapter.rs"
      ];
      forbiddenEditExceptions = [
        "packages/d2b-contracts/src/v2_component_session.rs"
        "packages/d2b-contracts/src/v2_services.rs"
      ];
      deletes = [ ];
      scope = [
        "Freeze an authenticated runtime-systemd-user service composition fingerprint covering runtime, shell, and tty while retaining one fixed listener/session."
        "Provide one per-connection negotiated descriptor resolver and one shared inbound-call registration/cancellation wrapper."
        "Align d2b-client with the composition session before helper or d2bd cutover."
      ];
      prompt = ''
        Implement the ComponentSession service seam in exactly the owned files:
        canonical runtime+shell+tty composition fingerprint, per-connection
        method/index descriptor policy, exact SCM_RIGHTS binding, shared
        register-dispatch-complete cancellation, canonical cross-uid channel
        binding, and d2b-client composition support. Do not implement helper
        business logic or d2bd routing in this component.
      '';
    };

    "user-agent-service-seam" = {
      branch = "adr0045-w8-integration-user-agent-service-seam";
      dependsOn = [ "component-session-service-seam" "user-agent-backend-core" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/component-session-v2-vectors.json"
        "nixos-modules/unsafe-local-helper.nix"
        "packages/d2b-contracts/src/v2_component_session.rs"
        "packages/d2b-unsafe-local-helper/src/server.rs"
        "packages/d2b-unsafe-local-helper/src/services/runtime_systemd_user/mod.rs"
        "tests/host-integration/unsafe-local-helper.nix"
      ];
      reservedPaths = [
        "packages/d2b-contracts/src/v2_component_session.rs"
        "packages/d2b-unsafe-local-helper/src/server.rs"
        "nixos-modules/unsafe-local-helper.nix"
      ];
      deletes = [ ];
      scope = [
        "Make the existing deployed d2b-unsafe-local-helper a production-ready responder for the runtime-systemd-user and co-located shell services."
        "Freeze one canonical directional channel binding, narrow controller authorization, real systemd-user/shell backend wiring, one exact terminal attachment/named stream, and request-bound cancellation."
        "Do not implement the d2bd initiator cutover in this component."
      ];
      prompt = ''
        Implement the two-sided user-agent service seam in exactly the owned
        files. Use one shared channel-binding helper/vector on both ends,
        authorize only the exact controller identity, wire the real
        systemd-user scope and persistent-shell backend, preserve one
        runtime-agent socket/session with co-located services, transfer exactly
        one validated CLOEXEC terminal stream, and bind cancellation to the
        exact in-flight request. Never add a root unit, broad group access,
        SSH path, or success-shaped unavailable backend.
      '';
    };

    "user-agent-backend-core" = {
      branch = "adr0045-w8-integration-user-agent-backend-core";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/explanation/systemd-user-shell-backend.md"
        "packages/d2b-unsafe-local-helper/src/controller_allowlist.rs"
        "packages/d2b-unsafe-local-helper/src/services/mod.rs"
        "packages/d2b-unsafe-local-helper/src/shell_runtime.rs"
        "packages/d2b-unsafe-local-helper/src/shell_socket.rs"
        "packages/d2b-unsafe-local-helper/src/shell_supervisor.rs"
        "packages/d2b-unsafe-local-helper/src/systemd.rs"
        "packages/d2b-unsafe-local-helper/tests/shell_supervisor.rs"
      ];
      reservedPaths = [
        "packages/d2b-unsafe-local-helper/src/shell_supervisor.rs"
        "packages/d2b-unsafe-local-helper/src/systemd.rs"
      ];
      deletes = [ ];
      scope = [
        "Implement the real per-user scope/shell backend independently of ComponentSession transport."
        "Eliminate fork-before-scope-placement, target confusion, and blocking pump teardown while preserving reconnectable PTY/output-ring semantics."
      ];
      prompt = ''
        Implement the real helper backend core in exactly the owned files.
        Launch inside verified transient USER scopes without an escape window,
        key state by authenticated target, and make pumps nonblocking,
        bounded, wakeable, and lock-safe. Do not edit session transport/server
        composition.
      '';
    };

    "shell-client-core" = {
      branch = "adr0045-w8-integration-shell-client-core";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/explanation/systemd-user-shell-client.md"
        "packages/d2bd/src/unsafe_local_helper.rs"
        "packages/d2bd/src/unsafe_local_terminal.rs"
      ];
      reservedPaths = [
        "packages/d2bd/src/unsafe_local_helper.rs"
      ];
      deletes = [ ];
      scope = [
        "Make the d2bd shell client async-first, operation-scoped, target-bound, cancellation-correct, and free of dummy terminal drains."
        "Depend on an injected session/terminal port until the ComponentSession composition lands."
      ];
      prompt = ''
        Implement the d2bd shell client core in exactly the owned files behind
        an injected async port. Remove block_in_place/current-thread panic,
        per-uid reconnectable sessions, dummy drains, and wrong request-id
        cancellation. Do not select the final ComponentSession adapter here.
      '';
    };

    "systemd-user-shell-routing" = {
      branch = "adr0045-w8-integration-systemd-user-shell-routing";
      dependsOn = [ "user-agent-service-seam" "shell-client-core" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/explanation/systemd-user-shell-routing.md"
        "packages/d2bd/src/shell_backend.rs"
        "packages/d2bd/src/workload_dispatch.rs"
      ];
      reservedPaths = [
        "packages/d2bd/src/shell_backend.rs"
        "packages/d2bd/src/workload_dispatch.rs"
      ];
      deletes = [       ];
      scope = [
        "Route d2bd unsafe-local persistent-shell dispatch through the existing deployed d2b-unsafe-local-helper ComponentSession services instead of the legacy JSON client path."
        "Keep the existing per-session PTY, output-ring, and attach/detach ownership model from AGENTS.md's 'Unsafe-local persistent shells' row; do not add a root unit, per-VM service, or SSH path."
        "Reuse the W6 per-user runtime agent and co-located shell service; do not add duplicate runtime-agent or shell-supervisor crates or processes."
      ];
      prompt = ''
        Implement the d2bd ComponentSession client routing to the existing
        d2b-unsafe-local-helper runtime-systemd-user and shell services in
        exactly the owned files. Replace the legacy JSON helper/terminal
        client only after parity is proven. Preserve the existing
        session-table admission and audit shape in shell_backend.rs and
        workload_dispatch.rs; do not add a duplicate agent/supervisor process,
        root service, per-VM unit, SSH path, or direct compositor fallback.
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

    "runtime-state-platform-seam" = {
      branch = "adr0045-w8-integration-runtime-state-platform-seam";
      dependsOn = [ "state-lock-authority-contract" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "nixos-modules/realm-storage-rows.nix"
        "packages/Cargo.lock"
        "packages/d2b-contract-tests/tests/storage_sync_contracts.rs"
        "packages/d2b-gateway-runtime/Cargo.toml"
        "packages/d2bd/Cargo.toml"
        "packages/d2b-state/src/audit.rs"
        "packages/d2b-state/src/path.rs"
      ];
      reservedPaths = [
        "packages/d2b-state/src/audit.rs"
        "nixos-modules/realm-storage-rows.nix"
      ];
      forbiddenEditExceptions = [
        "packages/Cargo.lock"
      ];
      deletes = [ ];
      scope = [
        "Freeze shared d2b-state dependencies, durable directory creation, segmented audit, and generated gateway ledger/audit storage+sync rows."
        "Do not implement gateway or restart business logic."
      ];
      prompt = ''
        Implement the shared runtime-state platform in exactly the owned files
        after state-lock authority lands. Add gateway ledger/audit rows,
        d2b-state dependencies, durable mkdir, segmented audit/checkpoint/gap
        support, and real contract tests.
      '';
    };

    "gateway-runtime-integration" = {
      branch = "adr0045-w8-integration-gateway-runtime-integration";
      dependsOn = [ "gateway-replacement" "runtime-state-platform-seam" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "packages/d2b-gateway-runtime/src/audit_jsonl.rs"
        "packages/d2b-gateway-runtime/src/production.rs"
        "packages/d2bd/src/lib.rs"
      ];
      reservedPaths = [
        "packages/d2b-gateway-runtime/src/production.rs"
        "packages/d2bd/src/lib.rs"
      ];
      forbiddenEditExceptions = [
        "packages/d2bd/src/lib.rs"
      ];
      deletes = [ ];
      scope = [
        "Replace free-form gateway files with generated d2b-state ledger/audit authority and wire the authenticated gateway into d2bd/WorkExecutor."
      ];
      prompt = ''
        Integrate the accepted gateway core with runtime-state authority and
        d2bd in exactly the owned files. Remove JsonlGatewayAudit/free-form
        paths and wire the authenticated GatewayPort/RemotePeerClient.
      '';
    };

    "provider-parity-proof" = {
      branch = "adr0045-w8-integration-provider-parity-proof";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/verify-provider-parity.md"
        "packages/d2b-realm-provider/src/mock.rs"
        "packages/d2b-realm-provider/src/parity.rs"
      ];
      reservedPaths = [
        "packages/d2b-realm-provider/src/parity.rs"
      ];
      deletes = [ ];
      scope = [
        "Build the exhaustive provider-parity inventory and tests independently of daemon fallback deletion."
      ];
      prompt = ''
        Implement provider parity proof in exactly the owned files. Inventory
        every integrated provider operation and prove no W5/W6/W7 behavior is
        missing. Do not edit daemon fallback wiring.
      '';
    };

    "provider-parity-fallback-removal" = {
      branch = "adr0045-w8-integration-provider-parity-fallback-removal";
      dependsOn = [ "gateway-runtime-integration" "provider-parity-proof" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "packages/d2bd/src/realm_stubs.rs"
      ];
      reservedPaths = [
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
        "packages/d2bd/src/lib.rs"
      ];
      reservedPaths = [
        "packages/d2bd/src/lib.rs"
        "packages/d2bd/src/restart_continuity.rs"
        "packages/d2bd/src/storage_lifecycle.rs"
      ];
      forbiddenEditExceptions = [
        "packages/d2bd/src/lib.rs"
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
        packages/d2b-priv-broker/src/runtime.rs or any undeclared
        frozen/global-protected path.
      '';
    };

    "restart-broker-authority" = {
      branch = "adr0045-w8-integration-restart-broker-authority";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/privileges.md"
        "packages/d2b-contracts/src/broker_wire.rs"
        "packages/d2b-core/src/bundle_resolver.rs"
        "packages/d2b-priv-broker/src/live_handlers.rs"
        "packages/d2b-priv-broker/src/runtime.rs"
      ];
      reservedPaths = [
        "packages/d2b-priv-broker/src/live_handlers.rs"
        "packages/d2b-priv-broker/src/runtime.rs"
      ];
      forbiddenEditExceptions = [
        "packages/d2b-contracts/src/broker_wire.rs"
        "packages/d2b-core/src/bundle_resolver.rs"
        "packages/d2b-priv-broker/src/runtime.rs"
      ];
      deletes = [ ];
      scope = [
        "Implement broker-authoritative exact-cgroup candidate discovery, pidfd handoff, shared population lock, and consuming Missing cleanup lease."
        "The verified OwnedFd must flow unchanged from broker discovery to daemon registration."
      ];
      prompt = ''
        Implement restart broker authority in exactly the owned files:
        resolve cgroup contracts, lock the exact leaf, discover 0/1/many
        candidates, open and hand off one pidfd with identity, and authorize
        consuming Missing cleanup. Do not implement daemon reporting.
      '';
    };

    "restart-runtime-integration" = {
      branch = "adr0045-w8-integration-restart-runtime-integration";
      dependsOn = [
        "restart-observability-audit"
        "restart-broker-authority"
        "runtime-state-platform-seam"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "packages/d2bd/src/lib.rs"
        "packages/d2bd/src/supervisor/pidfd_table.rs"
      ];
      reservedPaths = [
        "packages/d2bd/src/lib.rs"
        "packages/d2bd/src/supervisor/pidfd_table.rs"
      ];
      forbiddenEditExceptions = [
        "packages/d2bd/src/lib.rs"
      ];
      deletes = [ ];
      scope = [
        "Wire contract-first broker adoption/cleanup, exact pidfd-consuming registration, report validation, d2b-state audit, and provider polling into live startup."
      ];
      prompt = ''
        Integrate accepted restart daemon and broker components in exactly the
        owned files. Validate contracts before effects, consume the exact
        verified pidfd into registration, and persist/audit only complete
        coverage.
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
    "state-lock-authority-contract"
    "secrets-authority-seam"
    "secrets-lifecycle"
    "secrets-runtime-integration"
    "component-session-service-seam"
    "user-agent-backend-core"
    "user-agent-service-seam"
    "shell-client-core"
    "systemd-user-shell-routing"
    "realm-routing-work-executor-fabric"
    "gateway-replacement"
    "runtime-state-platform-seam"
    "gateway-runtime-integration"
    "provider-parity-proof"
    "provider-parity-fallback-removal"
    "restart-observability-audit"
    "restart-broker-authority"
    "restart-runtime-integration"
  ];

  globalExternalDependencies = [
    "shared-root-w8-manifest-seam"
  ];

  baseExternalDependencies = {
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

  pathExternalDependencies = [ ];

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
      members today. They remain future contract placeholders, not W8
      implementation requirements. W8 reuses the deployed
      d2b-unsafe-local-helper per-user agent and its co-located generated
      runtime-systemd-user and shell services.
    '';
    notYetCreatedCrates = [
      "packages/d2b-activation-helper/"
      "packages/d2b-one-shot-helper/"
      "packages/d2b-provider-agent/"
      "packages/d2b-security-key-helper/"
      "packages/d2b-tty-helper/"
      "packages/d2b-wlcontrol/"
    ];
    requiredForThisWave = [ ];
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
