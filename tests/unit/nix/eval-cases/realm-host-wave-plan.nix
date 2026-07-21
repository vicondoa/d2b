let
  components = {
    "realm-schema" = {
      branch = "adr0045-w7-realm-schema";
      dependsOn = [ ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/configure-realms.md"
        "docs/reference/realm-options.md"
        "nixos-modules/options-daemon.nix"
        "nixos-modules/options-envs.nix"
        "nixos-modules/options-gateway.nix"
        "nixos-modules/options-host.nix"
        "nixos-modules/options-observability.nix"
        "nixos-modules/options-realms-network.nix"
        "nixos-modules/options-realms-providers.nix"
        "nixos-modules/options-realms-workloads.nix"
        "nixos-modules/options-realms.nix"
        "nixos-modules/options-site.nix"
        "nixos-modules/options-vms.nix"
        "nixos-modules/options.nix"
        "tests/unit/nix/cases/assertions.nix"
        "tests/unit/nix/cases/realms.nix"
        "tests/unit/nix/cases/requested-vm-config.nix"
        "tests/unit/nix/eval-cases/assertions.nix"
      ];
      reservedPaths = [
        "docs/how-to/configure-realms.md"
        "nixos-modules/options-realms-providers.nix"
      ];
      deletes = [
        "nixos-modules/options-envs.nix"
        "nixos-modules/options-gateway.nix"
        "nixos-modules/options-vms.nix"
      ];
      scope = [
        "Declare only the destructive-cutover acknowledgement and realm, workload, and provider configuration."
        "Remove transitional env, VM, gateway, relay, legacyVmName, inherit-env, and provider-placeholder option shapes without tombstones."
        "Keep provider selection typed by primary authority and implementation rather than free-form placeholder kinds."
      ];
      prompt = ''
        Implement the realm-only public option schema in exactly the owned files.
        Remove d2b.vms, d2b.envs, d2b.gateways, relay compatibility, legacyVmName,
        inherit-env, and provider-placeholder declarations without aliases or
        removed-option modules. Do not edit the index, emitters, Cargo files,
        generated service contracts, allocator API, or delivery tooling.
      '';
    };

    "normalized-index" = {
      branch = "adr0045-w7-normalized-index";
      dependsOn = [ "realm-schema" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/naming-conventions.md"
        "nixos-modules/assertions.nix"
        "nixos-modules/index-realms.nix"
        "nixos-modules/index-resources.nix"
        "nixos-modules/index-workloads.nix"
        "nixos-modules/index.nix"
        "nixos-modules/lib.nix"
        "nixos-modules/v2-identity.nix"
        "tests/unit/nix/cases/index.nix"
        "tests/unit/nix/cases/realm-workloads.nix"
        "tests/unit/nix/cases/v2-identity.nix"
      ];
      reservedPaths = [
        "nixos-modules/index-realms.nix"
        "nixos-modules/index-resources.nix"
        "nixos-modules/index-workloads.nix"
      ];
      deletes = [ ];
      scope = [
        "Build a recursion-safe normalized realm, workload, provider, role, and resource index."
        "Derive every realm, workload, provider, and role short ID through the canonical pure-Nix v2 identity implementation."
        "Reject collisions and raw human, provider, device, or bus identifiers in runtime path components."
      ];
      prompt = ''
        Replace the transitional VM/env index with a recursion-safe realm-only
        normalized index in exactly the owned files. Derive and collision-check
        canonical 20-character IDs, enumerate roles/providers/resources, and
        keep human names only in metadata and canonical targets. Do not emit
        processes or mutate provider-registry, allocator, bundle, or Rust files.
      '';
    };

    "realm-principals" = {
      branch = "adr0045-w7-realm-principals";
      dependsOn = [ "normalized-index" ];
      externalDependsOn = [
        "shared-root-deletion-contract-test-seam"
      ];
      ownedFiles = [
        "docs/explanation/realm-controller-boundaries.md"
        "docs/reference/realm-principals.md"
        "nixos-modules/host-polkit.nix"
        "nixos-modules/host-users.nix"
        "nixos-modules/realm-access.nix"
        "nixos-modules/realm-users.nix"
        "tests/migration-state.d/polkit-allowlist-eval.toml"
        "tests/unit/nix/cases/principal-uid-collision.nix"
      ];
      reservedPaths = [
        "docs/reference/realm-principals.md"
        "nixos-modules/realm-access.nix"
        "nixos-modules/realm-users.nix"
      ];
      deletes = [
        "nixos-modules/host-polkit.nix"
      ];
      scope = [
        "Emit distinct d2bd-r, d2bbr-r, d2bcg-r, and d2b-r principals per host-local child realm."
        "Keep controller, broker, cgroup-internal, and public access identities separate."
        "Remove legacy lifecycle-group migration and polkit compatibility surfaces."
      ];
      prompt = ''
        Implement per-realm users and groups in exactly the owned files. Emit
        distinct controller, broker, internal cgroup, and public socket
        principals from canonical realm IDs, prove collision freedom, and
        delete legacy polkit/group migration wiring. Do not create units,
        listeners, cgroups, namespaces, or runtime allocator behavior.
      '';
    };

    "local-root-endpoints" = {
      branch = "adr0045-w7-local-root-endpoints";
      dependsOn = [
        "normalized-index"
        "realm-principals"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "nixos-modules/host-broker.nix"
        "nixos-modules/host-daemon.nix"
        "nixos-modules/host-otel-relay-acl.nix"
        "nixos-modules/prebuilt-packages.nix"
        "nixos-modules/unsafe-local-helper.nix"
        "tests/unit/nix/cases/broker-bundle-path.nix"
        "tests/unit/nix/cases/broker-caps.nix"
        "tests/unit/nix/cases/broker-service-posture.nix"
        "tests/unit/nix/cases/broker-socket-activation.nix"
        "tests/unit/nix/cases/d2bd-startup-smoke.nix"
        "tests/unit/nix/cases/daemon-autostart.nix"
        "tests/unit/nix/cases/daemon-default-compat.nix"
        "tests/unit/nix/cases/restart-policy.nix"
      ];
      reservedPaths = [ ];
      deletes = [
        "nixos-modules/host-otel-relay-acl.nix"
      ];
      scope = [
        "Emit only d2bd.socket, d2bd.service, d2b-priv-broker.socket, and d2b-priv-broker.service at PID1."
        "Keep child realm controllers and brokers out of systemd service and socket unit namespaces."
        "Make fixed local-root endpoint provenance explicit and listener-FD-only."
      ];
      prompt = ''
        Rewrite the fixed local-root unit and package declarations in exactly
        the owned files. PID1 must expose only the fixed public and broker
        socket/service pairs; no child realm or workload unit may be emitted.
        Delete specialized host helper/relay stubs. Do not implement allocator
        dispatch, listener binding, child spawn, or pidfd supervision.
      '';
    };

    "allocator-emission" = {
      branch = "adr0045-w7-allocator-emission";
      dependsOn = [
        "normalized-index"
        "realm-principals"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/inspect-host-realm-isolation.md"
        "docs/reference/local-root-allocator.md"
        "docs/reference/realm-controller-config.md"
        "docs/reference/realm-identity-lifecycle.md"
        "nixos-modules/allocator-json.nix"
        "nixos-modules/realm-allocator-rows.nix"
        "nixos-modules/realm-controller-config-json.nix"
        "nixos-modules/realm-endpoint-rows.nix"
        "nixos-modules/realm-identity-config-json.nix"
        "nixos-modules/realm-process-rows.nix"
        "nixos-modules/realm-resource-rows.nix"
        "tests/unit/nix/cases/autostart-wiring.nix"
        "tests/unit/nix/cases/realm-allocator-emission.nix"
      ];
      reservedPaths = [
        "nixos-modules/realm-allocator-rows.nix"
        "nixos-modules/realm-endpoint-rows.nix"
        "nixos-modules/realm-process-rows.nix"
        "nixos-modules/realm-resource-rows.nix"
        "tests/unit/nix/cases/realm-allocator-emission.nix"
      ];
      deletes = [ ];
      scope = [
        "Emit generated child listener rows, lease requests, process launch records, ordering records, cgroup records, namespace records, and ownership rows."
        "Cover the home, dev, and work child realms without creating child units."
        "Keep realm roots and workload interiors process-free and describe direct controller, broker, and role-leaf placement."
      ];
      prompt = ''
        Implement only declarative allocator input and child process/resource
        records in exactly the owned files. Emit home/dev/work public+broker
        listener rows, typed lease requests, separate controller/broker launch
        records, namespace/cgroup/ownership records, and deterministic ordering.
        Own the declarative local-root allocator and realm identity lifecycle
        references; W5 owns distinct runtime/service references.
        Never implement allocation, bind, spawn, pidfd, adoption, supervision,
        reconciliation, or lease execution; those are W5 runtime ownership.
      '';
    };

    "realm-network" = {
      branch = "adr0045-w7-realm-network";
      dependsOn = [
        "normalized-index"
        "realm-schema"
      ];
      externalDependsOn = [
        "shared-root-deletion-contract-test-seam"
      ];
      ownedFiles = [
        "docs/how-to/host-prepare.d/network.md"
        "docs/how-to/route-conflicts.md"
        "docs/reference/realm-network.md"
        "examples/multi-env/README.md"
        "examples/multi-env/configuration.nix"
        "examples/multi-env/flake.nix"
        "nixos-modules/gateway-vm.nix"
        "nixos-modules/net-mdns.nix"
        "nixos-modules/net.nix"
        "nixos-modules/network.nix"
        "nixos-modules/provider-registry-v2-extensions/network.nix"
        "nixos-modules/realm-network-rows.nix"
        "packages/d2b-contract-tests/tests/policy_host_realm_relay.rs"
        "packages/d2b-contract-tests/tests/policy_source.rs"
        "packages/d2b-contract-tests/tests/storage_sync_contracts.rs"
        "tests/unit/nix/cases/bridge-ipv6-boot-sysctl.nix"
        "tests/unit/nix/cases/gateway-vm.nix"
        "tests/unit/nix/cases/ifname-nix-rust-parity.nix"
        "tests/unit/nix/cases/multi-env-daemon-backed.nix"
        "tests/unit/nix/cases/net-vm-network.nix"
      ];
      reservedPaths = [
        "docs/reference/realm-network.md"
        "nixos-modules/provider-registry-v2-extensions/network.nix"
        "nixos-modules/realm-network-rows.nix"
      ];
      deletes = [
        "nixos-modules/gateway-vm.nix"
        "tests/unit/nix/cases/gateway-vm.nix"
      ];
      scope = [
        "Emit realm-scoped bridge, veth, TAP, nftables partition, namespace, and network-provider rows."
        "Replace env and generated gateway VM ownership with realm/workload resource records."
        "Keep global claims as allocator requests and namespace-local effects as child-broker lease use."
      ];
      prompt = ''
        Implement realm-scoped declarative networking in exactly the owned
        files. Remove env/gateway ownership, emit short-ID network and allocator
        resource rows, preserve default isolation, and provide only the network
        provider-registry fragment. Do not edit the registry composer, allocator
        runtime, provider Rust crates, broker runtime, or shared DTOs.
      '';
    };

    "realm-storage" = {
      branch = "adr0045-w7-realm-storage";
      dependsOn = [
        "normalized-index"
        "realm-schema"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/reference/key-lifecycle.md"
        "docs/reference/per-vm-state-ownership.md"
        "docs/reference/ssh-host-key-preflight.md"
        "docs/reference/state-storage-sync-audit-v2.md"
        "docs/reference/store-lifecycle.md"
        "docs/reference/store-sync.md"
        "docs/reference/store-virtiofs.md"
        "nixos-modules/components/audit.nix"
        "nixos-modules/host-activation.nix"
        "nixos-modules/host-keys.nix"
        "nixos-modules/options-ownership-matrix.nix"
        "nixos-modules/provider-registry-v2-extensions/storage.nix"
        "nixos-modules/realm-storage-rows.nix"
        "nixos-modules/storage-json.nix"
        "nixos-modules/store.nix"
        "nixos-modules/sync-json.nix"
        "tests/unit/nix/cases/activation-runtime-tmpfiles.nix"
        "tests/unit/nix/cases/per-vm-state-ownership.nix"
        "tests/unit/nix/cases/store-overlay-emit.nix"
        "tests/unit/nix/cases/umask-roundtrip.nix"
        "tests/unit/nix/cases/volume-mounts.nix"
      ];
      reservedPaths = [
        "nixos-modules/provider-registry-v2-extensions/storage.nix"
        "nixos-modules/realm-storage-rows.nix"
      ];
      deletes = [ ];
      scope = [
        "Emit the complete short-ID /etc, /var/lib, /var/cache, and /run realm/workload storage layout."
        "Make the broker the sole creator and repair owner below fixed anchors."
        "Emit state, audit, store-view, key, lock, lease, and storage-provider rows without activation repair."
      ];
      prompt = ''
        Implement realm/workload storage, sync, state, audit, key, and store-view
        emission in exactly the owned files. Use short-ID paths and broker-owned
        opaque repair rows; remove activation/tmpfiles repair below fixed
        anchors. Add only the storage provider-registry fragment. Do not change
        bundle v12, storage DTOs, broker runtime, or allocator runtime.
      '';
    };

    "realm-devices" = {
      branch = "adr0045-w7-realm-devices";
      dependsOn = [
        "normalized-index"
        "realm-schema"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/migrate-usbip-yubikey-to-security-key.md"
        "docs/how-to/troubleshoot-usbip.md"
        "docs/how-to/use-usb-security-key.md"
        "docs/reference/components-graphics.md"
        "docs/reference/components-tpm.md"
        "docs/reference/components-usb-security-key.md"
        "docs/reference/components-usbip.md"
        "docs/reference/components-video.md"
        "examples/graphics-workstation/configuration.nix"
        "examples/graphics-workstation/README.md"
        "nixos-modules/components/graphics.nix"
        "nixos-modules/components/security-key-guest.nix"
        "nixos-modules/components/tpm.nix"
        "nixos-modules/components/usbip.nix"
        "nixos-modules/components/video/guest.nix"
        "nixos-modules/provider-registry-v2-extensions/device.nix"
        "nixos-modules/realm-device-rows.nix"
        "tests/unit/nix/cases/security-key-gating.nix"
        "tests/unit/nix/cases/usb-security-key.nix"
        "tests/unit/nix/cases/usbip-gating.nix"
        "tests/unit/nix/cases/video-contract.nix"
        "tests/fixtures/runner-shape-swtpm.snap"
        "tests/golden/runner-shape/gpu-argv-minimal.txt"
        "tests/golden/runner-shape/swtpm-argv-minimal.txt"
        "tests/golden/runner-shape/usbip-argv-minimal.txt"
        "tests/golden/runner-shape/video-argv-minimal.txt"
        "tests/golden/runner-shape/virtgpu-ioctl-values.txt"
        "tests/unit/smoke/smoke-eval-graphics.nix"
        "tests/unit/smoke/smoke-eval-tpm.nix"
      ];
      reservedPaths = [
        "nixos-modules/provider-registry-v2-extensions/device.nix"
        "nixos-modules/realm-device-rows.nix"
      ];
      deletes = [ ];
      scope = [
        "Emit realm/workload-scoped TPM, USBIP, FIDO, GPU, video, render-node, and device lease rows."
        "Keep global device access allocator-mediated and child access FD-only."
        "Provide only the device provider-registry fragment."
      ];
      prompt = ''
        Migrate declarative device resources in exactly the owned files. Emit
        short-ID workload/role rows and allocator lease requests for TPM, USBIP,
        FIDO, GPU, and video resources, plus the device registry fragment.
        Preserve closed mediation and no raw device/bus IDs in paths. Do not edit
        registry bindings/composer, broker/runtime code, or shared contracts.
      '';
    };

    "realm-audio" = {
      branch = "adr0045-w7-realm-audio";
      dependsOn = [
        "normalized-index"
        "realm-schema"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/use-console-and-audio.md"
        "docs/reference/components-audio.md"
        "nixos-modules/components/audio/guest.nix"
        "nixos-modules/components/audio/host.nix"
        "nixos-modules/provider-registry-v2-extensions/audio.nix"
        "nixos-modules/realm-audio-rows.nix"
        "tests/golden/runner-shape/audio-argv-minimal.txt"
        "tests/unit/nix/cases/realm-audio-resources.nix"
      ];
      reservedPaths = [
        "nixos-modules/provider-registry-v2-extensions/audio.nix"
        "nixos-modules/realm-audio-rows.nix"
        "tests/unit/nix/cases/realm-audio-resources.nix"
      ];
      deletes = [ ];
      scope = [
        "Emit realm/workload-scoped vhost-user audio process, endpoint, storage, and lease rows."
        "Keep PipeWire access mediated and ambient host endpoints out of bundle metadata."
        "Provide only the audio provider-registry fragment."
      ];
      prompt = ''
        Migrate declarative audio resources in exactly the owned files. Emit
        short-ID audio role/process/socket/storage/lease rows and the audio
        registry fragment while preserving PipeWire mediation and bounded state.
        Do not edit the central registry, process composer, broker/runtime code,
        shared DTOs, Cargo.lock, or tooling.
      '';
    };

    "realm-observability" = {
      branch = "adr0045-w7-realm-observability";
      dependsOn = [
        "normalized-index"
        "realm-schema"
      ];
      externalDependsOn = [ ];
      ownedFiles = [
        "docs/how-to/enable-observability.md"
        "docs/reference/components-observability.md"
        "examples/with-observability/configuration.nix"
        "examples/with-observability/README.md"
        "nixos-modules/components/observability/default.nix"
        "nixos-modules/components/observability/guest.nix"
        "nixos-modules/components/observability/host.nix"
        "nixos-modules/components/observability/stack.nix"
        "nixos-modules/observability-host-secrets.nix"
        "nixos-modules/observability-vm.nix"
        "nixos-modules/realm-observability-rows.nix"
        "tests/golden/runner-shape/otel-host-bridge-argv-minimal.txt"
        "tests/unit/nix/cases/examples-with-observability.nix"
        "tests/unit/nix/cases/observability.nix"
        "tests/unit/nix/eval-cases/observability.nix"
      ];
      reservedPaths = [
        "nixos-modules/realm-observability-rows.nix"
      ];
      deletes = [
        "nixos-modules/observability-vm.nix"
      ];
      scope = [
        "Replace the host-singleton observability VM declaration with realm/workload resource composition."
        "Preserve the frozen local-observability registry mapping and bounded projection contract."
        "Do not create a second observability registration path."
      ];
      prompt = ''
        Migrate only observability-side Nix resource composition in exactly the
        owned files. Remove the singleton VM declaration and emit realm/workload
        rows while consuming the frozen local-observability mapping unchanged.
        Do not add a registry fragment, alter its binding, or edit provider,
        daemon, broker, allocator, bundle-contract, Cargo, or tooling code.
      '';
    };

    "platform-provider-mappings" = {
      branch = "adr0045-w7-platform-provider-mappings";
      dependsOn = [ "normalized-index" ];
      externalDependsOn = [ ];
      ownedFiles = [
        "nixos-modules/provider-registry-v2-extensions/display.nix"
        "nixos-modules/provider-registry-v2-extensions/substrate.nix"
        "nixos-modules/provider-registry-v2-extensions/transport.nix"
        "tests/unit/nix/cases/platform-provider-mappings.nix"
      ];
      reservedPaths = [
        "nixos-modules/provider-registry-v2-extensions/display.nix"
        "nixos-modules/provider-registry-v2-extensions/substrate.nix"
        "nixos-modules/provider-registry-v2-extensions/transport.nix"
        "tests/unit/nix/cases/platform-provider-mappings.nix"
      ];
      deletes = [ ];
      scope = [
        "Emit closed transport, substrate, and display provider entry fragments."
        "Use existing first-party implementations and canonical short IDs."
        "Do not compose the registry or alter bindings."
      ];
      prompt = ''
        Add only the transport, substrate, and display provider entry fragments
        and focused eval coverage in exactly the owned files. Consume existing
        W4 provider implementations and normalized IDs. Do not edit the central
        emitter, Rust binding enum, generated schema/docs, allocator, runtime,
        Cargo.lock, workspace dependencies, or delivery tooling.
      '';
    };

    "workload-processes" = {
      branch = "adr0045-w7-workload-processes";
      dependsOn = [
        "allocator-emission"
        "normalized-index"
        "realm-audio"
        "realm-devices"
        "realm-network"
        "realm-observability"
        "realm-storage"
      ];
      externalDependsOn = [
        "shared-root-deletion-contract-test-seam"
      ];
      ownedFiles = [
        "docs/how-to/edit-vm-config-from-inside.md"
        "docs/how-to/qemu-media.md"
        "docs/reference/components-home-manager.md"
        "docs/reference/qemu-media.md"
        "docs/reference/runner-shape-audit.md"
        "docs/explanation/vhost-user-video-status.md"
        "examples/qemu-media-dark-live.nix"
        "nixos-modules/base.nix"
        "nixos-modules/closures-json.nix"
        "nixos-modules/components/home-manager.nix"
        "nixos-modules/d2b-ch-vsock-connect.nix"
        "nixos-modules/guest-control-host.nix"
        "nixos-modules/guest-control.nix"
        "nixos-modules/guest-sshd-host-keys.nix"
        "nixos-modules/host-ssh-host-keys.nix"
        "nixos-modules/host.nix"
        "nixos-modules/minijail-profiles.nix"
        "nixos-modules/processes-json.nix"
        "nixos-modules/role-process-rows.nix"
        "nixos-modules/vm-evaluator.nix"
        "nixos-modules/vm-guest-base.nix"
        "nixos-modules/vm-options.nix"
        "nixos-modules/vm-submodule.nix"
        "nixos-modules/workload-process-rows.nix"
        "packages/d2b-contract-tests/tests/policy_misc.rs"
        "packages/d2b-contract-tests/tests/policy_modules.rs"
        "tests/migration-state.d/vm-submodule-cutover-eval.toml"
        "tests/migration-state.d/vm-submodule-eval.toml"
        "tests/unit/nix/cases/external-vm-kind.nix"
        "tests/unit/nix/cases/guest-config-containment.nix"
        "tests/unit/nix/cases/guest-control-auth.nix"
        "tests/unit/nix/cases/guest-control-vsock.nix"
        "tests/unit/nix/cases/guest-exec-policy.nix"
        "tests/unit/nix/cases/guest-shell-policy.nix"
        "tests/unit/nix/cases/readiness-waves.nix"
        "tests/unit/nix/cases/vm-eval-overlays.nix"
        "tests/unit/nix/eval-cases/guest-control-auth-eval.nix"
        "tests/unit/nix/eval-cases/guest-control-vsock-eval.nix"
        "tests/unit/nix/eval-cases/guest-exec-policy-eval.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/clean-guest.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/evil-microvm.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/imports-microvm.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/reads-standard-option.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/sets-d2b.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/sets-microvm.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/spoof-file.nix"
        "tests/unit/nix/eval-cases/guest-fixtures/tofile-microvm.nix"
        "tests/unit/nix/eval-cases/guest-shell-policy-eval.nix"
        "tests/unit/nix/eval-cases/processes-dag-order.nix"
        "tests/fixtures/runner-shape-cloud-hypervisor.snap"
        "tests/fixtures/runner-shape-virtiofsd.snap"
        "tests/golden/runner-shape/cloud-hypervisor-argv-minimal.txt"
        "tests/golden/runner-shape/examples-minimal-declaredRunner.txt"
        "tests/golden/runner-shape/virtiofsd-argv-minimal.txt"
        "tests/golden/runner-shape/vsock-relay-argv-minimal.txt"
        "tests/unit/smoke/guest-static-consumption-eval.nix"
        "tests/unit/smoke/smoke-eval-extraspecialargs.nix"
        "tests/unit/smoke/smoke-eval-home-manager.nix"
      ];
      reservedPaths = [
        "nixos-modules/role-process-rows.nix"
        "nixos-modules/workload-process-rows.nix"
      ];
      deletes = [
        "nixos-modules/vm-options.nix"
        "nixos-modules/vm-submodule.nix"
      ];
      scope = [
        "Emit realm-owned workload and role DAG records without per-workload systemd units."
        "Compose allocator-declared child records and resource fragments into processes.json."
        "Remove VM-name-derived runtime paths and legacy VM evaluator option ownership."
      ];
      prompt = ''
        Migrate workload/role process and guest composition in exactly the owned
        files. Use the normalized index and declarative allocator/resource rows,
        emit no per-workload units, and replace VM-name paths with canonical IDs.
        Do not implement allocator/spawn/supervision, alter provider-registry
        contracts, or edit W5/W6 crates, Cargo.lock, or delivery tooling.
      '';
    };

    "desktop-metadata" = {
      branch = "adr0045-w7-desktop-metadata";
      dependsOn = [
        "normalized-index"
        "realm-schema"
      ];
      externalDependsOn = [
        "shared-root-deletion-contract-test-seam"
      ];
      ownedFiles = [
        "docs/how-to/configure-desktop-terminal-integration.md"
        "docs/how-to/migrate-to-wayland-proxy.md"
        "docs/how-to/niri-vm-borders.md"
        "docs/reference/desktop-wrapper.md"
        "docs/reference/ui-colors.md"
        "docs/reference/wayland-proxy-warnings.md"
        "nixos-modules/clipboard.nix"
        "nixos-modules/desktop-metadata-json.nix"
        "nixos-modules/manifest.nix"
        "nixos-modules/niri-vm-borders.nix"
        "nixos-modules/notifications.nix"
        "nixos-modules/realm-workloads-launcher-json.nix"
        "nixos-modules/realm-workloads-launcher-v2-json.nix"
        "nixos-modules/ui-colors.nix"
        "nixos-modules/unsafe-local-workloads-json.nix"
        "packages/d2b-contract-tests/tests/realm_workload_schema_contract.rs"
        "tests/unit/nix/cases/clipboard.nix"
        "tests/unit/nix/cases/niri-vm-borders.nix"
      ];
      reservedPaths = [
        "nixos-modules/desktop-metadata-json.nix"
      ];
      deletes = [
        "nixos-modules/manifest.nix"
        "nixos-modules/realm-workloads-launcher-json.nix"
      ];
      scope = [
        "Emit bounded presentation metadata keyed only by canonical targets and canonical IDs."
        "Keep configured argv private and presentation projections non-authoritative."
        "Remove VM-name, gateway, and compatibility launcher metadata."
      ];
      prompt = ''
        Implement canonical-target-only desktop, launcher, color, clipboard, and
        notification metadata in exactly the owned files. Keep argv private,
        projections non-authoritative, and unsafe-local posture mapped to the
        systemd-user provider. Remove old launcher compatibility metadata. Do
        not edit W6 runtime/helper crates or provider-registry bindings.
      '';
    };

    "provider-registry-composition" = {
      branch = "adr0045-w7-provider-registry-composition";
      dependsOn = [
        "normalized-index"
        "platform-provider-mappings"
        "realm-audio"
        "realm-devices"
        "realm-network"
        "realm-storage"
      ];
      externalDependsOn = [
        "shared-root-provider-registry-open-consumer-seam"
      ];
      ownedFiles = [
        "docs/reference/schemas/v2/provider-registry-v2.json"
        "docs/reference/schemas/v2/provider-registry-v2.md"
        "nixos-modules/provider-registry-v2-json.nix"
        "packages/d2b-contracts/src/provider_registry_v2.rs"
        "tests/unit/nix/cases/provider-registry-v2.nix"
      ];
      reservedPaths = [
        "tests/unit/nix/cases/provider-registry-v2.nix"
      ];
      deletes = [ ];
      scope = [
        "Extend the existing provider-registry-v2 artifact with transport, substrate, display, network, storage, device, and audio bindings."
        "Preserve the frozen local-runtime and local-observability entries byte-for-byte in meaning."
        "Use only the shared-root-approved protected emitter, binding, schema, reference, and flake seams."
      ];
      prompt = ''
        The shared-root provider-registry open-consumer seam is present. Compose
        the existing provider-registry-v2 family in exactly the owned files
        after every fragment owner is unblocked and lands. Extend bindings for
        transport, substrate, display, network, storage, device, and audio while
        preserving local-runtime/local-observability. Never edit W5 d2bd
        consumers, create a second registry, or edit other contracts,
        Cargo.lock, or tooling.
      '';
    };

    "bundle-integration" = {
      branch = "adr0045-w7-bundle-integration";
      dependsOn = [
        "allocator-emission"
        "desktop-metadata"
        "local-root-endpoints"
        "normalized-index"
        "platform-provider-mappings"
        "provider-registry-composition"
        "realm-audio"
        "realm-devices"
        "realm-network"
        "realm-observability"
        "realm-principals"
        "realm-schema"
        "realm-storage"
        "workload-processes"
      ];
      externalDependsOn = [
        "shared-root-deletion-contract-test-seam"
      ];
      ownedFiles = [
        "CHANGELOG.md"
        "README.md"
        "delivery/manifests/w7.json"
        "docs/explanation/design.md"
        "docs/reference/filesystem-endpoint-layout.md"
        "docs/reference/manifest-bundle.md"
        "docs/reference/manifest-schema.json"
        "docs/reference/manifest-schema.md"
        "examples/README.md"
        "examples/minimal/configuration.nix"
        "examples/minimal/README.md"
        "examples/with-entra-id/configuration.nix"
        "examples/with-entra-id/flake.nix"
        "examples/with-entra-id/README.md"
        "examples/with-entra-id/work-entra.nix"
        "flake.nix"
        "nixos-modules/bundle-artifacts.nix"
        "nixos-modules/bundle.nix"
        "nixos-modules/default.nix"
        "nixos-modules/host-json.nix"
        "nixos-modules/privileges-json.nix"
        "templates/default/configuration.nix"
        "templates/default/flake.nix"
        "templates/default/README.md"
        "tests/migration-ledger.toml"
        "tests/fixtures/deny-unknown/bundle-invalid.json"
        "tests/fixtures/deny-unknown/bundle-valid.json"
        "tests/fixtures/deny-unknown/host-invalid.json"
        "tests/fixtures/deny-unknown/host-valid.json"
        "tests/golden/manifest_v04/baseline-vms.json"
        "tests/golden/runner-shape/parity-drift.json"
        "tests/golden/vms.json-91d69b0"
        "tests/golden/vms.json-p2-v3"
        "tests/golden/vms.json-signoz-v4"
        "tests/golden/vms.json-signoz-v6"
        "tests/golden/vms.json-signoz-v7"
        "tests/unit/nix/cases/bundle-artifacts.nix"
        "tests/unit/nix/eval-cases/realm-host-component-policy.nix"
        "tests/unit/nix/eval-cases/realm-host-wave-plan.nix"
        "tests/unit/nix/eval-cases/shared.nix"
        "tests/unit/nix/pinned/aarch64-linux.txt"
        "tests/unit/nix/pinned/common.txt"
        "tests/unit/nix/pinned/x86_64-linux.txt"
        "tests/unit/nix/tools/realm-host-component-diff.sh"
        "tests/unit/smoke/smoke-eval-aarch64.nix"
        "tests/unit/smoke/smoke-eval.nix"
      ];
      reservedPaths = [
        "docs/reference/filesystem-endpoint-layout.md"
        "tests/unit/nix/eval-cases/realm-host-component-policy.nix"
        "tests/unit/nix/tools/realm-host-component-diff.sh"
      ];
      deletes = [ ];
      scope = [
        "Wire all file-disjoint modules into one complete bundle-v12 realm-only configuration."
        "Delete remaining VM, env, gateway, relay, placeholder, and host-singleton wiring from examples, templates, bundle, host, and privilege artifacts."
        "Keep delivery authority and cross-component validation integrator-owned."
      ];
      prompt = ''
        Integrate the completed components in exactly the owned files. Preserve
        bundleVersion 12 and schemaVersion v2, wire every required private
        artifact, finish old-surface deletion in examples/templates, and run
        cross-component eval/drift/policy validation. Do not absorb component
        implementation, edit frozen contracts, Cargo.lock, workspace
        dependencies, delivery tooling, or another wave's manifest.
      '';
    };
  };

  componentOrder = [
    "realm-schema"
    "normalized-index"
    "realm-principals"
    "local-root-endpoints"
    "allocator-emission"
    "realm-network"
    "realm-storage"
    "realm-devices"
    "realm-audio"
    "realm-observability"
    "platform-provider-mappings"
    "workload-processes"
    "desktop-metadata"
    "provider-registry-composition"
    "bundle-integration"
  ];

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
in
{
  schemaVersion = 1;
  wave = "w7";
  sharedRoot = "c5816944fdbcbb3c8985040742b267b1372b0ede";
  branch = "adr0045-w7-realm-host";
  pullRequestBase = "adr0045-post-w4-contracts";
  inherit componentOrder components;

  dispatch = {
    trustedBranch = "adr0045-w7-realm-host";
    gate =
      "tests/unit/nix/tools/realm-host-component-diff.sh --candidate-root <component-worktree>";
    commonPrompt = ''
      Start from the current clean adr0045-w7-realm-host prep head, use the
      component's exact branch and ownedFiles, and do not edit another
      component's files. Commit before validation, then run the component diff
      gate from the trusted prep worktree. A blocked external dependency is a
      hard stop, not permission to edit shared-root or W5 files.
    '';
  };

  affectedInventory = {
    contractTests = inventoryFor [ ] [
      "packages/d2b-contract-tests/"
    ];
    docs = inventoryFor [ "README.md" ] [
      "docs/explanation/"
      "docs/how-to/"
      "docs/reference/"
    ];
    examples = inventoryFor [ ] [
      "examples/"
      "templates/"
    ];
    fixtures = inventoryFor [ ] [
      "tests/fixtures/"
      "tests/golden/"
    ];
    tests = inventoryFor [
      "tests/migration-ledger.toml"
      "tests/migration-state.d/polkit-allowlist-eval.toml"
      "tests/migration-state.d/vm-submodule-cutover-eval.toml"
      "tests/migration-state.d/vm-submodule-eval.toml"
    ] [
      "tests/unit/nix/"
      "tests/unit/smoke/"
    ];
  };
  sharedRootRetainedTests = {
    owner = "shared-root";
    paths = [
      "tests/static.sh"
    ];
  };

  externalDependencies = {
    shared-root-provider-registry-open-consumer-seam = {
      owner = "adr0045-post-w4-contracts";
      status = "ready";
      requiredRebase = false;
      landedCommit = "fa18c34741b8a898b4786a14e19e86e395d37325";
      contractFiles = [
        "packages/d2b-contracts/src/provider_registry_v2.rs"
      ];
      foreignConsumerFiles = [
        "packages/d2bd/src/lib.rs"
        "packages/d2bd/src/provider_effects.rs"
        "packages/d2bd/src/provider_registry.rs"
      ];
      acceptance = [
        "ProviderBindingV2 exposes a non-exhaustive or equivalent forward-compatible consumer seam."
        "W5 d2bd consumers use that seam or explicit unknown-axis handling without W7 edits."
        "The W7 branch is rebased onto the accepted shared-root seam commit."
      ];
    };
    shared-root-deletion-contract-test-seam = {
      owner = "adr0045-post-w4-contracts";
      status = "ready";
      requiredRebase = false;
      landedCommit = "c5816944fdbcbb3c8985040742b267b1372b0ede";
      acceptance = [
        "The trusted wave policy grants W7 only the enumerated d2b-contract-tests, migration ledger, and migration-state paths."
        "Every granted path remains assigned to exactly one W7 component."
        "No Cargo lock, workspace, runtime consumer, or other frozen test path is opened."
        "W7 rebases onto the accepted narrow extension-seam commit before any affected component starts."
      ];
    };
    shared-root-w7-fixture-path-ownership = {
      owner = "adr0045-post-w4-contracts";
      status = "blocked";
      requiredRebase = true;
      acceptance = [
        "The trusted wave policy grants W7 exact fixture and golden ownership."
        "W5 and frozen consumers retain code ownership; W7 changes only declared expected artifacts."
      ];
    };
    w5-runtime-document-split = {
      owner = "adr0045-w5-control";
      status = "blocked";
      requiredRebase = false;
      acceptance = [
        "W5 runtime/service docs use their distinct reserved paths."
        "W7 retains declarative local-root allocator and realm identity lifecycle docs."
      ];
    };
  };

  pathExternalDependencies = [
    {
      dependency = "w5-runtime-document-split";
      paths = [
        "docs/reference/local-root-allocator.md"
        "docs/reference/realm-identity-lifecycle.md"
      ];
    }
    {
      dependency = "shared-root-w7-fixture-path-ownership";
      paths = builtins.concatLists (builtins.map
        (row: row.paths)
        (inventoryFor [ ] [ "tests/fixtures/" "tests/golden/" ]));
    }
  ];

  crossWaveOwnership = {
    source = "shared ownership inventory";
    w5Owner = "w5";
    w5FixturePath =
      "packages/d2bd/tests/fixtures/control-service-slices.json";
    w7DeclarativeDocs = [
      "docs/reference/local-root-allocator.md"
      "docs/reference/realm-identity-lifecycle.md"
    ];
    w5RuntimeDocs = [
      "docs/how-to/configure-work-gateway.md"
      "docs/reference/broker-w2-dispositions.md"
      "docs/reference/cli-contract.md"
      "docs/reference/daemon-api.md"
      "docs/reference/daemon-audit-check.md"
      "docs/reference/daemon-metrics.md"
      "docs/reference/guest-control-exec-interactive-tty.md"
      "docs/reference/guest-control-exec-io-chunked-stdio.md"
      "docs/reference/guest-control-exec-io-credit-window.md"
      "docs/reference/guest-control-persistent-shell.md"
      "docs/reference/local-root-allocator-service.md"
      "docs/reference/privileges.md"
      "docs/reference/provider-contract-v2.md"
      "docs/reference/provider-managed-sandboxes.md"
      "docs/reference/realm-access-resolver.md"
      "docs/reference/realm-controller-service.md"
      "docs/reference/realm-core.md"
      "docs/reference/realm-identity-service.md"
      "docs/reference/realm-policy.md"
      "docs/reference/realm-routing.md"
      "docs/reference/remote-full-host-nodes.md"
      "docs/reference/v2-provider-implementations.md"
    ];
    w5ConsumerFiles = [
      "packages/d2bd/src/lib.rs"
      "packages/d2bd/src/provider_effects.rs"
      "packages/d2bd/src/provider_registry.rs"
    ];
    deferredPurgeDocs = {
      owner = "w10";
      paths = [
        "docs/how-to/migrate-d2b-v0-to-v1.md"
        "docs/how-to/migrate-d2b-v1-0-to-v1-1.md"
        "docs/how-to/migrate-d2b-v1-2-to-v1-3.md"
        "docs/how-to/migrate-d2b-v1-2-to-v2.md"
        "docs/how-to/migrate-nixos-to-daemon.md"
        "docs/how-to/migrating-from-microvm.md"
      ];
    };
    w6RuntimeDocs = {
      owner = "w6";
      paths = [
        "docs/how-to/configure-clipboard-picker.md"
        "docs/how-to/configure-unsafe-local-launchers.md"
        "docs/how-to/use-persistent-shells.md"
        "docs/reference/components-shell.md"
        "docs/reference/unsafe-local-provider.md"
      ];
    };
    frozenContractDocs = {
      owner = "shared-root";
      paths = [
        "docs/reference/schemas/v2/allocator.json"
        "docs/reference/schemas/v2/allocator.md"
        "docs/reference/schemas/v2/host.json"
        "docs/reference/schemas/v2/processes.json"
        "docs/reference/schemas/v2/realm-controllers.json"
        "docs/reference/schemas/v2/realm-workloads-launcher-v2.json"
        "docs/reference/schemas/v2/realm-workloads-launcher-v2.md"
        "docs/reference/schemas/v2/unsafe-local-helper-wire.json"
        "docs/reference/schemas/v2/unsafe-local-workloads.json"
        "docs/reference/schemas/v2/unsafe-local-workloads.md"
      ];
    };
    foreignW5Fixtures = [
      "tests/fixtures/gen-w3-cli-goldens.py"
      "tests/golden/cli-output/host-check-ifname-collision.json"
      "tests/golden/cli-output/host-check-ifname-collision.txt"
      "tests/golden/cli-output/host-check-ifname-too-long.json"
      "tests/golden/cli-output/host-check-ifname-too-long.txt"
      "tests/golden/cli-output/host-check-manifest-skew.json"
      "tests/golden/cli-output/host-check-manifest-skew.txt"
      "tests/golden/cli-output/host-check-runner-shape-drift.json"
      "tests/golden/cli-output/host-check-runner-shape-drift.txt"
    ];
  };

  frozenParentContracts = {
    bundle = {
      version = 12;
      schemaVersion = "v2";
      rule = "emit and compose only; no version, DTO, or generation-model change";
    };
    providerRegistry = {
      schemaVersion = "v2";
      preservedAxes = [
        "local-observability"
        "local-runtime"
      ];
      rule = "extend the existing artifact only through approved seams";
    };
    allocator = {
      owner = "w5";
      apiFiles = [
        "packages/d2b-contracts/proto/v2/broker.proto"
        "packages/d2b-realm-core/src/allocator.rs"
        "packages/d2b-realm-core/src/allocator_engine.rs"
      ];
      w7Owns = [
        "declarative child listener rows"
        "declarative lease requests"
        "declarative process and ordering records"
        "declarative cgroup, namespace, resource, and ownership records"
      ];
      w5Owns = [
        "allocator service dispatch"
        "runtime listener creation and binding"
        "typed child controller and broker spawn"
        "pidfd supervision and adoption"
        "lease allocation, reconciliation, revocation, and execution"
      ];
    };
  };

  providerRegistryExtensionSeams = {
    owner = "provider-registry-composition";
    approvedProtectedFiles = [
      "docs/reference/schemas/v2/provider-registry-v2.json"
      "docs/reference/schemas/v2/provider-registry-v2.md"
      "nixos-modules/provider-registry-v2-json.nix"
      "packages/d2b-contracts/src/provider_registry_v2.rs"
    ];
    integrationOwner = "bundle-integration";
    integrationProtectedFiles = [
      "flake.nix"
    ];
    sharedRootConsumerDependency =
      "shared-root-provider-registry-open-consumer-seam";
    fragments = {
      audio = {
        owner = "realm-audio";
        path = "nixos-modules/provider-registry-v2-extensions/audio.nix";
      };
      device = {
        owner = "realm-devices";
        path = "nixos-modules/provider-registry-v2-extensions/device.nix";
      };
      display = {
        owner = "platform-provider-mappings";
        path = "nixos-modules/provider-registry-v2-extensions/display.nix";
      };
      network = {
        owner = "realm-network";
        path = "nixos-modules/provider-registry-v2-extensions/network.nix";
      };
      storage = {
        owner = "realm-storage";
        path = "nixos-modules/provider-registry-v2-extensions/storage.nix";
      };
      substrate = {
        owner = "platform-provider-mappings";
        path = "nixos-modules/provider-registry-v2-extensions/substrate.nix";
      };
      transport = {
        owner = "platform-provider-mappings";
        path = "nixos-modules/provider-registry-v2-extensions/transport.nix";
      };
    };
  };

  deletionContractTestExtensionSeam = {
    dependency = "shared-root-deletion-contract-test-seam";
    owners = [
      {
        component = "realm-principals";
        deletedFiles = [ "nixos-modules/host-polkit.nix" ];
        extensionPaths = [
          "tests/migration-state.d/polkit-allowlist-eval.toml"
        ];
      }
      {
        component = "realm-network";
        deletedFiles = [ "nixos-modules/gateway-vm.nix" ];
        extensionPaths = [
          "packages/d2b-contract-tests/tests/policy_host_realm_relay.rs"
          "packages/d2b-contract-tests/tests/policy_source.rs"
          "packages/d2b-contract-tests/tests/storage_sync_contracts.rs"
        ];
      }
      {
        component = "workload-processes";
        deletedFiles = [ "nixos-modules/vm-submodule.nix" ];
        extensionPaths = [
          "packages/d2b-contract-tests/tests/policy_misc.rs"
          "packages/d2b-contract-tests/tests/policy_modules.rs"
          "tests/migration-state.d/vm-submodule-cutover-eval.toml"
          "tests/migration-state.d/vm-submodule-eval.toml"
        ];
        coupledRetirement =
          "policy_misc.rs also retires the host-polkit assertion after realm-principals lands";
      }
      {
        component = "desktop-metadata";
        deletedFiles = [
          "nixos-modules/realm-workloads-launcher-json.nix"
        ];
        extensionPaths = [
          "packages/d2b-contract-tests/tests/realm_workload_schema_contract.rs"
        ];
      }
      {
        component = "bundle-integration";
        deletedFiles = [ ];
        extensionPaths = [
          "tests/migration-ledger.toml"
        ];
        coupledRetirement =
          "integrator regenerates the shared retirement ledger after all four deletions";
      }
    ];
  };

  deletionInventory = {
    publicOptionPaths = [
      "d2b._envMeta"
      "d2b.envs"
      "d2b.gateways"
      "d2b.vms"
    ];
    transitionalRealmFields = [
      "env"
      "gateway-vm placement"
      "legacyVmName"
      "network.envs"
      "network inherit-env mode"
      "provider-placeholder kind"
      "relay compatibility block"
    ];
    generatedSurfaces = [
      "gateway VM declarations"
      "host-singleton observability VM declaration"
      "legacy realm-workloads-launcher artifact"
      "per-VM and per-env paths, principals, process rows, and resource rows"
      "specialized unsafe-local helper host wiring"
    ];
  };

  requiredDeclarativeInventory = {
    allocator = {
      owner = "allocator-emission";
      outputs = [
        "home/dev/work child process and total ordering records"
        "pre-bound child public and broker listener requests"
        "typed lease requests and resource acquisition order"
        "controller and broker cgroup leaf records"
        "realm-root and workloads process-free invariants"
        "user, mount, network, IPC, PID, and cgroup namespace records"
      ];
    };
    boundaries = {
      owner = "realm-principals";
      outputs = [
        "distinct controller and broker users"
        "internal cgroup group"
        "public socket access group"
        "per-realm state, runtime, cache, and audit ownership"
      ];
    };
    desktop = {
      owner = "desktop-metadata";
      outputs = [
        "canonical-target-only launcher rows"
        "bounded non-authoritative color, clipboard, and notification projections"
        "private configured argv kept out of public metadata"
      ];
    };
    resources = {
      owners = [
        "realm-audio"
        "realm-devices"
        "realm-network"
        "realm-storage"
      ];
      outputs = [
        "realm-scoped network resources"
        "realm/workload storage and store views"
        "typed mediated device resources"
        "realm/workload audio resources"
      ];
    };
    storageRepair = {
      owner = "realm-storage";
      outputs = [
        "fixed anchors only from PID1/tmpfiles"
        "allocator-owned child listeners"
        "broker-only dynamic path creation and repair"
        "opaque IDs and anchored fd-relative repair authority"
      ];
    };
  };

  forbiddenEdits = [
    "delivery/shared-contracts.json"
    "delivery/manifests/w5.json"
    "delivery/manifests/w6.json"
    "docs/adr/0045-provider-and-transport-framework.md"
    "packages/Cargo.lock"
    "packages/Cargo.toml"
    "packages/d2b-contracts/proto/v2/"
    "packages/d2b-contracts/src/generated_v2_services/"
    "packages/d2b-core/"
    "packages/d2b-realm-core/src/allocator.rs"
    "packages/d2b-realm-core/src/allocator_engine.rs"
    "packages/d2bd/src/lib.rs"
    "packages/d2bd/src/provider_effects.rs"
    "packages/d2bd/src/provider_registry.rs"
    "packages/d2bd/tests/fixtures/control-service-slices.json"
    "packages/xtask/"
  ];
}
