{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  prebuilt = if cfg.site.usePrebuiltHostTools then import ./prebuilt-packages.nix { inherit pkgs lib; } else { };

  # filter out `target/` dev caches from the source
  # so the Nix copy stays small (workspace target alone is ~17 GB).
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };

  d2bdSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2bd";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "d2bd" ];
    doCheck = false;
    # strip the dev-only sccache rustc-wrapper (see
    # host-broker.nix for full rationale). Writing the empty
    # rustc-wrapper into .cargo/config.toml shadows the dev
    # value without touching the parent cargo config.
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2bd $out/bin/d2bd 2>/dev/null \
        || install -Dm755 target/release/d2bd $out/bin/d2bd
      runHook postInstall
    '';
  };
  d2bdPackage = if prebuilt ? d2bd then prebuilt.d2bd else d2bdSourcePackage;

  # the user-facing CLI is now the Rust d2b crate
  # (packages/d2b). The pre-v1.0 bash CLI was RETIRED in;
  # ships the daemon-native Rust CLI as the only
  # `d2b` binary on the host.
  d2bCliSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "d2b" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b $out/bin/d2b 2>/dev/null \
        || install -Dm755 target/release/d2b $out/bin/d2b
      runHook postInstall
    '';
  };
  d2bCliPackage = if prebuilt ? d2b then prebuilt.d2b else d2bCliSourcePackage;

  netVmNames = map
    (envName: cfg.envs.${envName}.netName or "sys-${envName}-net")
    (lib.attrNames cfg.envs);
  gracefulTimeoutFor = vm:
    if vm.enable && vm.lifecycle.gracefulShutdown.enable then
      if vm.lifecycle.gracefulShutdown.timeoutSeconds == null
      then cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds
      else vm.lifecycle.gracefulShutdown.timeoutSeconds
    else
      0;
  maxSeconds = values: lib.foldl' (acc: value: lib.max acc value) 0 values;
  maxWorkloadShutdownTimeoutSeconds = maxSeconds (lib.mapAttrsToList
    (name: vm: if builtins.elem name netVmNames then 0 else gracefulTimeoutFor vm)
    cfg.vms);
  maxNetVmShutdownTimeoutSeconds = maxSeconds (lib.mapAttrsToList
    (name: vm: if builtins.elem name netVmNames then gracefulTimeoutFor vm else 0)
    cfg.vms);
  hostLocalRealms =
    lib.filter (realm: realm.placement == "host-local") cfg._index.realms.enabledList;
  brokerMaterializedFor = realm:
    realm.controller.broker.materializedSocket && realm.controller.broker.materializedService;
  serviceAttrName = unitName: lib.removeSuffix ".service" unitName;
  parentDaemonUnitFor = realm:
    if realm.parentPath == null || !(builtins.hasAttr realm.parentPath cfg._index.realms.enabledByPath)
    then null
    else
      let parent = cfg._index.realms.enabledByPath.${realm.parentPath};
      in
      if parent.placement == "host-local"
      then parent.controller.daemon.serviceName
      else null;
  forceFallbackTimeoutSeconds = 30;
  sidecarCleanupGraceSeconds = 120;
  d2bdStopTimeoutSeconds = lib.max 90 (
    maxWorkloadShutdownTimeoutSeconds
    + maxNetVmShutdownTimeoutSeconds
    + 2 * forceFallbackTimeoutSeconds
    + sidecarCleanupGraceSeconds
  );

  hostShutdownHook = pkgs.writeShellScript "d2b-host-shutdown-hook" ''
    set -eu

    manager_state="$(${pkgs.systemd}/bin/busctl get-property \
      org.freedesktop.systemd1 \
      /org/freedesktop/systemd1 \
      org.freedesktop.systemd1.Manager \
      SystemState 2>/dev/null || true)"

    if [ "$manager_state" != 's "stopping"' ]; then
      system_state="$(${pkgs.systemd}/bin/systemctl is-system-running 2>/dev/null || true)"
      if [ "$system_state" != "stopping" ]; then
        exit 0
      fi
    fi

    exec ${d2bCliPackage}/bin/d2b host shutdown-hook --apply
  '';

  # Small fd-safe activation helper that the host activation snippets
  # call instead of `[ -L ] / [ -f ] / find -type f` shell
  # check-then-act patterns. Lives in d2b-host because it
  # only depends on libc + nix; no IPC; no async runtime.
  d2bActivationHelperSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-activation-helper";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "d2b-host" "--bin" "d2b-activation-helper" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-activation-helper $out/bin/d2b-activation-helper 2>/dev/null \
        || install -Dm755 target/release/d2b-activation-helper $out/bin/d2b-activation-helper
      runHook postInstall
    '';
  };
  d2bActivationHelperPackage = if prebuilt ? "d2b-activation-helper" then prebuilt."d2b-activation-helper" else d2bActivationHelperSourcePackage;

  d2bGatewayRuntimeSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-gateway-runtime";
    version = "0.0.0-bootstrap";
    src = packagesSrc;
    inherit cargoLock;
    cargoBuildFlags = [ "--package" "d2b-gateway-runtime" ];
    doCheck = false;
    postPatch = ''
      mkdir -p .cargo
      cat > .cargo/config.toml <<EOF
[build]
rustc-wrapper = ""
EOF
      rm -f .cargo/rustc-wrapper.sh
    '';
    installPhase = ''
      runHook preInstall
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-gateway-enroll $out/bin/d2b-gateway-enroll 2>/dev/null \
        || install -Dm755 target/release/d2b-gateway-enroll $out/bin/d2b-gateway-enroll
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-gateway-relay $out/bin/d2b-gateway-relay 2>/dev/null \
        || install -Dm755 target/release/d2b-gateway-relay $out/bin/d2b-gateway-relay
      runHook postInstall
    '';
  };
  d2bGatewayRuntimePackage = if prebuilt ? "d2b-gateway-runtime" then prebuilt."d2b-gateway-runtime" else d2bGatewayRuntimeSourcePackage;

  d2bCliShellArtifactsPackage = pkgs.runCommand "d2b-cli-shell-artifacts" { } ''
    install -Dm644 ${../docs/manpages/d2b.1} "$out/share/man/man1/d2b.1"
    ${pkgs.gzip}/bin/gzip -n -c ${../docs/manpages/d2b.1} > "$out/share/man/man1/d2b.1.gz"
    install -Dm644 ${../docs/completions/d2b.bash} "$out/share/bash-completion/completions/d2b"
    install -Dm644 ${../docs/completions/d2b.zsh} "$out/share/zsh/site-functions/_d2b"
    install -Dm644 ${../docs/completions/d2b.fish} "$out/share/fish/vendor_completions.d/d2b.fish"
  '';

  daemonConfigJson = builtins.toJSON {
    publicSocketPath = "/run/d2b/public.sock";
    brokerSocketPath = "/run/d2b/priv.sock";
    stateLockPath = "/run/d2b/daemon.lock";
    locksDir = "/run/d2b/locks";
    daemonUser = "d2bd";
    daemonGroup = "d2bd";
    publicSocketGroup = "d2b";
    launcherUsers = cfg.site.launcherUsers;
    adminUsers = cfg.site.adminUsers;
    serverVersion = "0.4.0";
    acceptedClientVersionRange = ">=0.4.0, <0.5.0";
    gatewayConfigPath = "/etc/d2b/gateway.json";
    realmControllersConfigPath = "/etc/d2b/realm-controllers.json";
    autostartParallelism = cfg.daemon.autostart.parallelism;
    gracefulShutdownTimeoutSeconds = cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
    liveActivationTimeoutSeconds = cfg.daemon.lifecycle.liveActivation.timeoutSeconds;
    artifacts = {
      publicManifestPath = "/run/current-system/sw/share/d2b/vms.json";
      bundlePath = "/etc/d2b/bundle.json";
      hostPath = "/etc/d2b/host.json";
      processesPath = "/etc/d2b/processes.json";
      closuresDir = "/etc/d2b/closures";
    };
  };
  realmDaemonConfig = realm: builtins.toJSON {
    publicSocketPath = realm.paths.publicSocket;
    brokerSocketPath = realm.paths.brokerSocket;
    stateLockPath = realm.controller.daemon.stateLockPath;
    locksDir = realm.controller.daemon.locksDir;
    daemonUser = realm.controller.daemon.user;
    daemonGroup = realm.controller.daemon.group;
    publicSocketGroup = realm.controller.daemon.publicSocketGroup;
    launcherUsers = realm.allowedUsers;
    adminUsers = cfg.site.adminUsers;
    serverVersion = "0.4.0";
    acceptedClientVersionRange = ">=0.4.0, <0.5.0";
    gatewayConfigPath = "/etc/d2b/gateway.json";
    realmControllersConfigPath = "/etc/d2b/realm-controllers.json";
    autostartParallelism = cfg.daemon.autostart.parallelism;
    gracefulShutdownTimeoutSeconds = cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
    liveActivationTimeoutSeconds = cfg.daemon.lifecycle.liveActivation.timeoutSeconds;
    artifacts = {
      publicManifestPath = "/run/current-system/sw/share/d2b/vms.json";
      bundlePath = "/etc/d2b/bundle.json";
      hostPath = "/etc/d2b/host.json";
      processesPath = "/etc/d2b/processes.json";
      closuresDir = "/etc/d2b/closures";
    };
  };
  realmDaemonEtc = lib.listToAttrs (map
    (realm: {
      name = lib.removePrefix "/etc/" realm.controller.daemon.configPath;
      value = {
        text = realmDaemonConfig realm;
        mode = "0640";
        user = "root";
        group = realm.controller.daemon.group;
      };
    })
    hostLocalRealms);
  realmTmpfilesFor = realm: [
    "d ${realm.paths.stateDir} 0750 ${realm.controller.daemon.user} ${realm.controller.daemon.group} -"
    "d /var/lib/d2b/audit/realms 0750 root d2bd -"
    "d ${realm.paths.auditDir} 0750 root ${realm.controller.daemon.group} -"
    "d ${realm.paths.runDir} 0710 root ${realm.controller.daemon.publicSocketGroup} -"
    "z ${realm.paths.runDir} 0710 root ${realm.controller.daemon.publicSocketGroup} -"
    "a+ ${realm.paths.runDir} - - - - g::--x"
    "a+ ${realm.paths.runDir} - - - - u:${realm.controller.daemon.user}:rwx"
    "a+ ${realm.paths.runDir} - - - - m::rwx"
    "f ${realm.controller.daemon.stateLockPath} 0640 ${realm.controller.daemon.user} ${realm.controller.daemon.group} -"
    "d ${realm.controller.daemon.locksDir} 0700 ${realm.controller.daemon.user} ${realm.controller.daemon.group} -"
  ];
  realmDaemonService = realm:
    let
      parentDaemonUnit = parentDaemonUnitFor realm;
      brokerSocketUnit =
        if brokerMaterializedFor realm
        then realm.controller.broker.socketUnitName
        else null;
      brokerServiceUnit =
        if brokerMaterializedFor realm
        then realm.controller.broker.serviceUnitName
        else null;
    in
    {
      description = "d2b host-local realm daemon";
      wantedBy = [ "multi-user.target" ];
      wants =
        [ "systemd-tmpfiles-setup.service" ]
        ++ lib.optional (brokerSocketUnit != null) brokerSocketUnit;
      after =
        [
          "systemd-tmpfiles-setup.service"
          "network.target"
          "dbus.socket"
          "dbus.service"
          "d2b.slice"
        ]
        ++ lib.optional (parentDaemonUnit != null) parentDaemonUnit
        ++ lib.optional (brokerSocketUnit != null) brokerSocketUnit
        ++ lib.optional (brokerServiceUnit != null) brokerServiceUnit;
      serviceConfig = {
        Type = "notify";
        NotifyAccess = "main";
        TimeoutStartSec = "5min";
        KillMode = "process";
        User = realm.controller.daemon.user;
        Group = realm.controller.daemon.group;
        ExecStart =
          "${d2bdPackage}/bin/d2bd serve " +
          "--config ${realm.controller.daemon.configPath} " +
          "--daemon-state-dir ${realm.paths.stateDir}";
        TimeoutStopSec = lib.mkDefault "${toString d2bdStopTimeoutSeconds}s";
        Restart = "on-failure";
        RestartSec = "2s";
        NoNewPrivileges = true;
        CapabilityBoundingSet = [ "" ];
        AmbientCapabilities = [ "" ];
        PrivateTmp = true;
        ProtectHome = true;
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        UMask = "0027";
        SupplementaryGroups = [ realm.controller.daemon.publicSocketGroup ];
        Slice = "d2b.slice";
      };
    };
  realmDaemonServices = lib.listToAttrs (map
    (realm: {
      name = serviceAttrName realm.controller.daemon.serviceName;
      value = realmDaemonService realm;
    })
    hostLocalRealms);
  enabledGateways = lib.mapAttrsToList
    (name: gw: { inherit name gw; })
    (lib.filterAttrs (_: gw: gw.enable) cfg.gateways);
  realmEntrypointPath = "/run/current-system/sw/share/d2b/realm-entrypoints.json";
  realmEntrypointData =
    let
      localEntry = {
        name = "local";
        value = {
          mode = "host-resident";
          gateway = null;
        };
      };
      gatewayEntries = map
        (gateway: {
          name = gateway.gw.realm;
          value = {
            mode = "gateway-backed";
            gateway = "${gateway.gw.vmName}.local.d2b";
          };
        })
        enabledGateways;
    in
    {
      schemaVersion = 1;
      entries = lib.listToAttrs ([ localEntry ] ++ gatewayEntries);
    };
  realmEntrypointsPkg = pkgs.writeTextFile {
    name = "d2b-realm-entrypoints";
    text = builtins.toJSON realmEntrypointData;
    destination = "/share/d2b/realm-entrypoints.json";
  };
  hostRealmRelayEgressPolicyData = {
    schemaVersion = 1;
    mode = "host-realm-relay-deny";
    owner = "d2b-host";
    gatewayInterfaces = map
      (gateway: "${gateway.gw.env}-l${toString gateway.gw.index}")
      enabledGateways;
    forbiddenHostEnvPrefixes = [ "D2B_RELAY_" ];
    diagnostics = {
      redacted = true;
      rateLimited = true;
      fields = [ "event" "protocol" "reason" "gatewayInterfaceClass" ];
      omitted = [ "payload" "headers" "token" "endpoint" "credential" ];
    };
    remediation = "enroll credentials inside the gateway guest and route realm traffic through the gateway VM";
  };
in
{
  options.d2b.host.usbip.allowlist = lib.mkOption {
    type = lib.types.listOf (lib.types.submodule ({ ... }: {
      options = {
        vendor = lib.mkOption {
          type = lib.types.strMatching "^0x[0-9A-Fa-f]{4}$";
          example = "0x1050";
          description = "Hex USB vendor ID to allow through the trusted USBIP bundle policy.";
        };
        product = lib.mkOption {
          type = lib.types.strMatching "^0x[0-9A-Fa-f]{4}$";
          example = "0x0407";
          description = "Hex USB product ID to allow through the trusted USBIP bundle policy.";
        };
      };
    }));
    default = [ ];
    example = [
      {
        vendor = "0x1050";
        product = "0x0407";
      }
    ];
    description = ''
      Host-wide USBIP vendor:product allowlist copied into the trusted
      host bundle so the broker can refuse devices outside the approved
      hardware set before bind/attach proceeds.
    '';
  };

  config = lib.mkIf cfg.daemonExperimental.enable {
    # DEPRECATED v1.2: kept as migration tombstone for the
    # d2b-launcher{,s} → d2b rename. No module references the
    # legacy groups; no user is a member. The empty declaration
    # preserves the legacy gid in /etc/group so the
    # d2bGroupMigration helper can match by numeric gid on direct
    # upgrades. Slated for removal in v1.3 after one release of
    # confirmed clean migration.
    users.groups.d2b-launchers = { };
    users.groups.d2bd = { };

    users.users = {
      d2bd = {
        isSystemUser = true;
        group = "d2bd";
        description = "d2b daemon user";
        # d2bd MUST be a supplementary member of d2b so it
        # can `chown(path,
        # None, Some(gid))` the public socket to the launcher
        # group on bind. Without this membership, the chown(2)
        # call returns EPERM (kernel allows chown-to-gid only
        # for one of the caller's groups, real/effective/
        # supplementary). The daemon failed at startup with
        # "internal-io" when chown(public.sock, -1, 1000)
        # returned EPERM.
        extraGroups = [ "d2b" ];
      };
    };

    d2b._hostToolPackages = {
      d2b = d2bCliPackage;
      d2bd = d2bdPackage;
      d2bGatewayRuntime = d2bGatewayRuntimePackage;
    };

    environment.systemPackages = [
      d2bdPackage
      d2bCliPackage
      d2bCliShellArtifactsPackage
      d2bActivationHelperPackage
      realmEntrypointsPkg
    ];
    d2b._computed.realmEntrypoints = realmEntrypointData // {
      path = realmEntrypointPath;
    };
    d2b._computed.hostRealmRelayEgressPolicy = hostRealmRelayEgressPolicyData // {
      path = "/etc/d2b/host-realm-relay-egress-policy.json";
    };

    environment.etc = {
      "d2b/host-realm-relay-egress-policy.json".text =
        builtins.toJSON hostRealmRelayEgressPolicyData;
      "d2b/daemon-config.json" = {
        text = daemonConfigJson;
        mode = "0640";
        user = "root";
        group = "d2bd";
      };
    } // realmDaemonEtc;

    systemd.tmpfiles.rules = [
      # d2bd runs non-root, so it gets an explicit rwx ACL on
      # /run/d2b and owns /run/d2b/locks, /run/d2b/state,
      # and the daemon.lock file. Keep the /run/d2b parent itself
      # root-owned: systemd-tmpfiles refuses to create root-owned
      # descendants such as /run/d2b/vms beneath a daemon-owned
      # parent as an unsafe path transition. /etc/d2b and
      # /var/lib/d2b stay root-owned and group-readable by d2bd
      # (the broker audit log under /var/lib/d2b/audit/ is
      # broker-owned and written by root; the daemon only reads).
      # /etc/d2b/ config + bundle/host/processes are root:d2bd
      # 0640 so the daemon reads without write.
      #
      # /run/d2b is group-owned by `d2b` so launcher users —
      # members of `d2b` via daemon-config.json's `publicSocketGroup` —
      # can traverse the directory to reach `public.sock`. The owning group
      # entry below narrows launcher access to r-x; the 1770 base mode keeps
      # the ACL mask at rwx so the named d2bd ACL can bind/remove the
      # public socket. The public socket itself is mode 0660 group d2b
      # (see bind_public_socket).
      "d /run/d2b 1770 root d2b -"
      "z /run/d2b 1770 root d2b -"
      "a+ /run/d2b - - - - g::r-x"
      "a+ /run/d2b - - - - u:d2bd:rwx"
      "a+ /run/d2b - - - - m::rwx"
      "f /run/d2b/daemon.lock 0640 d2bd d2bd -"
      # /run/d2b/locks holds per-VM `flock(LOCK_EX |
      # LOCK_NB)` files taken by the daemon for the entire `up` /
      # `prepare` / `destroy` mutation window. Mode 0700 d2bd
      # d2bd so only the daemon (and root) can open the lock file.
      # Cleared on every boot via the standard tmpfiles `d` rule
      # semantics.
      "d /run/d2b/locks 0700 d2bd d2bd -"
      # USBIP busid lock claims are broker-written records read by the
      # daemon. Keep the claim root root-owned so d2bd can read
      # and traverse it via the d2bd group but cannot create or
      # replace lock claims itself.
      "d /run/d2b/locks/usbip 0750 root d2bd -"
      "d /run/d2b/state 0700 d2bd d2bd -"
      "d /var/lib/d2b 0750 root d2bd -"
      "d /var/lib/d2b/daemon-state 0700 d2bd d2bd -"
      "d /var/cache/d2b 0750 root d2bd -"
      "d /etc/d2b 0750 root d2bd -"
    ] ++ lib.concatMap realmTmpfilesFor hostLocalRealms;

    systemd.services = {
      d2bd = {
      # d2bd is allowed to restart on switch/update. Running VMs survive
      # because systemd stops only the daemon main process (KillMode=process)
      # and the restarted daemon re-adopts broker-spawned runners by identity.
      description = "d2b daemon skeleton";
      wantedBy = [ "multi-user.target" ];
      wants = [
        "d2b-priv-broker.socket"
        "systemd-tmpfiles-setup.service"
      ];
      after = [
        "systemd-tmpfiles-setup.service"
        "network.target"
        "d2b-priv-broker.socket"
        "d2b-priv-broker.service"
        "dbus.socket"
        "dbus.service"
        "d2b.slice"
      ];
      serviceConfig = {
        # Type=notify makes systemd hold d2bd.service in "activating"
        # until the daemon has completed startup/adoption and is about to
        # accept public.sock frames. This prevents post-switch validation from
        # racing a successful `systemctl restart d2bd.service`.
        Type = "notify";
        NotifyAccess = "main";
        TimeoutStartSec = "5min";
        # A daemon restart is a continuation event: broker-spawned VM runners
        # must survive so the restarted daemon can re-adopt them. The guarded
        # ExecStop hook below still handles host shutdown/reboot teardown.
        KillMode = "process";
        # cgroup v2 delegation requires the long-lived daemon to be
        # non-root so the broker can fchown
        # the d2b.slice subtree to the daemon's uid/gid. Running
        # the daemon as root contradicts the delegation contract in
        # docs/reference/cgroup-delegation.md and ADR 0011 ("the
        # daemon is never uid 0; the broker delegates the cgroup
        # subtree to the daemon user").
        User = "d2bd";
        Group = "d2bd";
        ExecStart = "${d2bdPackage}/bin/d2bd serve --config /etc/d2b/daemon-config.json";
        ExecStop = "+${hostShutdownHook}";
        TimeoutStopSec = lib.mkDefault "${toString d2bdStopTimeoutSeconds}s";
        Restart = "on-failure";
        RestartSec = "2s";
        NoNewPrivileges = true;
        CapabilityBoundingSet = [ "" ];
        AmbientCapabilities = [ "" ];
        PrivateTmp = true;
        ProtectHome = true;
        # AF_UNIX: public.sock + broker IPC + the per-VM guest-control
        # vsock proxy socket the daemon-side authenticated Health probe
        # connects to. AF_INET/AF_INET6 remain for the daemon's other TCP
        # readiness probes (e.g. per-env usbipd backend ports).
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        UMask = "0027";
        # Supplementary group so the daemon can create
        # /run/d2b/public.sock with group ownership
        # d2b (the documented launcher discovery group).
        # The matching publicSocketGroup field in
        # daemon-config.json already declares d2b as
        # the public socket group; this SupplementaryGroups entry
        # gives the systemd unit's primary uid the second gid it
        # needs to chgrp the socket.
        SupplementaryGroups = [ "d2b" ];
      };
      };
    } // realmDaemonServices;
  };
}
