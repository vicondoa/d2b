{ config, lib, pkgs, ... }:

let
  cfg = config.d2b;
  localRoot = cfg._realmPrincipals.localRoot;
  controllerPrincipal = localRoot.controller;
  publicSocketPrincipal = localRoot.socketPrincipals.public;
  d2bLib = import ./lib.nix { inherit lib; };
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  cargoLock = {
    lockFile = ../packages/Cargo.lock;
    outputHashes."wl-proxy-0.1.2" =
      "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
  };
  workspaceVersion =
    (builtins.fromTOML (builtins.readFile ../packages/Cargo.toml))
      .workspace.package.version;

  mkWorkspacePackage = {
    pname,
    cargoBuildFlags,
    installPhase ? null,
  }:
    pkgs.rustPlatform.buildRustPackage ({
      inherit pname cargoBuildFlags cargoLock;
      version = workspaceVersion;
      src = packagesSrc;
      doCheck = false;
      postPatch = ''
        mkdir -p .cargo
        cat > .cargo/config.toml <<EOF
        [build]
        rustc-wrapper = ""
        EOF
        rm -f .cargo/rustc-wrapper.sh
      '';
    } // lib.optionalAttrs (installPhase != null) {
      inherit installPhase;
    });

  sourcePackages = {
    d2bd = mkWorkspacePackage {
      pname = "d2bd";
      cargoBuildFlags = [ "--package" "d2bd" ];
      installPhase = ''
        runHook preInstall
        install -Dm755 target/x86_64-unknown-linux-gnu/release/d2bd \
          "$out/bin/d2bd" 2>/dev/null \
          || install -Dm755 target/release/d2bd "$out/bin/d2bd"
        runHook postInstall
      '';
    };
    d2b = mkWorkspacePackage {
      pname = "d2b";
      cargoBuildFlags = [ "--package" "d2b" ];
      installPhase = ''
        runHook preInstall
        install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b \
          "$out/bin/d2b" 2>/dev/null \
          || install -Dm755 target/release/d2b "$out/bin/d2b"
        runHook postInstall
      '';
    };
    activationHelper = mkWorkspacePackage {
      pname = "d2b-activation-helper";
      cargoBuildFlags = [
        "--package"
        "d2b-host"
        "--bin"
        "d2b-activation-helper"
      ];
      installPhase = ''
        runHook preInstall
        install -Dm755 \
          target/x86_64-unknown-linux-gnu/release/d2b-activation-helper \
          "$out/bin/d2b-activation-helper" 2>/dev/null \
          || install -Dm755 target/release/d2b-activation-helper \
            "$out/bin/d2b-activation-helper"
        runHook postInstall
      '';
    };
  };
  packageFor = name: fallback:
    if builtins.hasAttr name prebuilt then prebuilt.${name} else fallback;
  d2bdPackage = packageFor "d2bd" sourcePackages.d2bd;
  d2bCliPackage = packageFor "d2b" sourcePackages.d2b;
  activationHelperPackage =
    packageFor "d2b-activation-helper" sourcePackages.activationHelper;

  cliShellArtifactsPackage = pkgs.runCommand "d2b-cli-shell-artifacts" { } ''
    install -Dm644 ${../docs/manpages/d2b.1} "$out/share/man/man1/d2b.1"
    ${pkgs.gzip}/bin/gzip -n -c ${../docs/manpages/d2b.1} \
      > "$out/share/man/man1/d2b.1.gz"
    install -Dm644 ${../docs/completions/d2b.bash} \
      "$out/share/bash-completion/completions/d2b"
    install -Dm644 ${../docs/completions/d2b.zsh} \
      "$out/share/zsh/site-functions/_d2b"
    install -Dm644 ${../docs/completions/d2b.fish} \
      "$out/share/fish/vendor_completions.d/d2b.fish"
  '';

  daemonConfigJson = builtins.toJSON {
    publicSocketPath = "/run/d2b/root.sock";
    brokerSocketPath = "/run/d2b/broker.sock";
    stateLockPath = "/run/d2b/daemon.lock";
    locksDir = "/run/d2b/locks";
    daemonUser = controllerPrincipal;
    daemonGroup = controllerPrincipal;
    publicSocketGroup = localRoot.publicGroup;
    unsafeLocalHelperSocketPath = null;
    unsafeLocalHelperSocketGroup = null;
    unsafeLocalHelperUsers = [ ];
    launcherUsers = cfg.site.launcherUsers;
    adminUsers = cfg.site.adminUsers;
    serverVersion = "0.4.0";
    acceptedClientVersionRange = ">=0.4.0, <0.5.0";
    gatewayConfigPath = "/etc/d2b/gateway.json";
    realmControllersConfigPath = "/etc/d2b/realm-controllers.json";
    realmIdentityConfigPath = "/etc/d2b/realm-identity.json";
    autostartParallelism = cfg.daemon.autostart.parallelism;
    gracefulShutdownTimeoutSeconds =
      cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
    liveActivationTimeoutSeconds =
      cfg.daemon.lifecycle.liveActivation.timeoutSeconds;
    artifacts = {
      publicManifestPath = "/run/current-system/sw/share/d2b/vms.json";
      bundlePath = "/etc/d2b/bundle.json";
      hostPath = "/etc/d2b/host.json";
      processesPath = "/etc/d2b/processes.json";
      closuresDir = "/etc/d2b/closures";
    };
  };

  shutdownHook = pkgs.writeShellScript "d2b-host-shutdown-hook" ''
    set -eu

    manager_state="$(${pkgs.systemd}/bin/busctl get-property \
      org.freedesktop.systemd1 \
      /org/freedesktop/systemd1 \
      org.freedesktop.systemd1.Manager \
      SystemState 2>/dev/null || true)"

    if [ "$manager_state" != 's "stopping"' ]; then
      system_state="$(${pkgs.systemd}/bin/systemctl \
        is-system-running 2>/dev/null || true)"
      if [ "$system_state" != "stopping" ]; then
        exit 0
      fi
    fi

    exec ${d2bCliPackage}/bin/d2b host shutdown-hook --apply
  '';
  stopTimeoutSeconds =
    lib.max 90 (cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds + 180);
in
{
  config = {
    users.groups.${controllerPrincipal} = { };
    users.users.${controllerPrincipal} = {
      isSystemUser = true;
      group = controllerPrincipal;
      extraGroups = [ localRoot.publicGroup ];
      description = "d2b local-root controller";
    };

    d2b._hostToolPackages = {
      d2b = d2bCliPackage;
      d2bd = d2bdPackage;
    };

    environment.systemPackages = [
      d2bdPackage
      d2bCliPackage
      cliShellArtifactsPackage
      activationHelperPackage
    ];
    environment.etc."d2b/daemon-config.json" = {
      text = daemonConfigJson;
      mode = "0640";
      user = "root";
      group = controllerPrincipal;
    };

    systemd.tmpfiles.rules = [
      "d /run/d2b 1770 root ${localRoot.publicGroup} -"
      "z /run/d2b 1770 root ${localRoot.publicGroup} -"
      "a+ /run/d2b - - - - g::r-x"
      "a+ /run/d2b - - - - u:${controllerPrincipal}:rwx"
      "a+ /run/d2b - - - - m::rwx"
      "f /run/d2b/daemon.lock 0640 ${controllerPrincipal} ${controllerPrincipal} -"
      "d /run/d2b/locks 0700 ${controllerPrincipal} ${controllerPrincipal} -"
      "d /run/d2b/state 0700 ${controllerPrincipal} ${controllerPrincipal} -"
      "d /var/lib/d2b 0750 root ${controllerPrincipal} -"
      "d /var/lib/d2b/daemon-state 0700 ${controllerPrincipal} ${controllerPrincipal} -"
      "d /var/cache/d2b 0750 root ${controllerPrincipal} -"
      "d /etc/d2b 0750 root ${controllerPrincipal} -"
    ];

    systemd.sockets.d2bd = {
      description = "d2b local-root public socket";
      wantedBy = [ "sockets.target" ];
      requires = [ "systemd-tmpfiles-setup.service" ];
      after = [ "systemd-tmpfiles-setup.service" ];
      socketConfig = {
        ListenSequentialPacket = "/run/d2b/root.sock";
        SocketUser = publicSocketPrincipal.owner;
        SocketGroup = publicSocketPrincipal.group;
        SocketMode = publicSocketPrincipal.mode;
        Accept = false;
        FileDescriptorName = "public.sock";
        Service = "d2bd.service";
        RemoveOnStop = true;
      };
    };

    systemd.services.d2bd = {
      description = "d2b local-root controller";
      wantedBy = [ "multi-user.target" ];
      requires = [ "d2bd.socket" ];
      wants = [
        "d2b-priv-broker.socket"
        "systemd-tmpfiles-setup.service"
      ];
      after = [
        "systemd-tmpfiles-setup.service"
        "network.target"
        "d2bd.socket"
        "d2b-priv-broker.socket"
        "d2b-priv-broker.service"
        "dbus.socket"
        "dbus.service"
        "d2b.slice"
      ];
      restartTriggers = [
        cfg._bundle.bundle.path
        cfg._bundle.providerRegistryV2Json.path
      ];
      serviceConfig = {
        Type = "notify";
        NotifyAccess = "main";
        TimeoutStartSec = "5min";
        KillMode = "process";
        User = controllerPrincipal;
        Group = controllerPrincipal;
        ExecStart =
          "${d2bdPackage}/bin/d2bd serve --config /etc/d2b/daemon-config.json";
        ExecStop = "+${shutdownHook}";
        TimeoutStopSec = lib.mkDefault "${toString stopTimeoutSeconds}s";
        Restart = "on-failure";
        RestartSec = "2s";
        NoNewPrivileges = true;
        CapabilityBoundingSet = [ "" ];
        AmbientCapabilities = [ "" ];
        PrivateTmp = true;
        ProtectHome = true;
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        UMask = "0027";
        SupplementaryGroups = [ localRoot.publicGroup ];
        Slice = "d2b.slice";
      };
    };
  };
}
