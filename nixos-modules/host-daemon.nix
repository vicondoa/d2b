{ config, lib, pkgs, d2bHostTools, d2bHostToolOverrides ? null, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };
  posture = import ./state-posture-contract.nix { inherit lib; };
  # The per-Device TPM principals the state-directory access lines below are
  # granted to, derived from the same trusted rows the runtime derives them
  # from (`d2bLib.deviceTpmPrincipals`).
  tpmPrincipals = d2bLib.deviceTpmPrincipals cfg;
  prebuilt =
    if cfg.site.usePrebuiltHostTools
    then import ./prebuilt-packages.nix { inherit pkgs lib; }
    else { };

  d2bdSourcePackage = d2bHostTools.d2bd;
  d2bdPackage = d2bLib.selectHostToolPackage {
    overrides = d2bHostToolOverrides;
    key = "d2bd";
    fallback = if prebuilt ? d2bd then prebuilt.d2bd else d2bdSourcePackage;
  };

  d2bCliSourcePackage = d2bHostTools.d2b;
  d2bCliPackage = d2bLib.selectHostToolPackage {
    overrides = d2bHostToolOverrides;
    key = "d2b";
    fallback = if prebuilt ? d2b then prebuilt.d2b else d2bCliSourcePackage;
  };

  activationHelperSourcePackage = d2bHostTools.activationHelper;
  activationHelperPackage = d2bLib.selectHostToolPackage {
    overrides = d2bHostToolOverrides;
    key = "activationHelper";
    fallback =
      if prebuilt ? "d2b-activation-helper"
      then prebuilt."d2b-activation-helper"
      else activationHelperSourcePackage;
  };

  d2bCliShellArtifactsPackage = pkgs.runCommand "d2b-cli-shell-artifacts" { } ''
    install -Dm644 ${../docs/manpages/d2b.1} "$out/share/man/man1/d2b.1"
    ${pkgs.gzip}/bin/gzip -n -c ${../docs/manpages/d2b.1} > "$out/share/man/man1/d2b.1.gz"
    install -Dm644 ${../completions/d2b.bash} "$out/share/bash-completion/completions/d2b"
    install -Dm644 ${../completions/d2b.zsh} "$out/share/zsh/site-functions/_d2b"
    install -Dm644 ${../completions/d2b.fish} "$out/share/fish/vendor_completions.d/d2b.fish"
  '';

  daemonConfigJson = builtins.toJSON {
    publicSocketPath = "/run/d2b/public.sock";
    brokerSocketPath = "/run/d2b/priv.sock";
    stateLockPath = "/run/d2b/daemon.lock";
    locksDir = "/run/d2b/locks";
    daemonUser = "d2bd";
    daemonGroup = "d2bd";
    publicSocketGroup = "d2b";
    unsafeLocalHelperSocketPath = null;
    unsafeLocalHelperSocketGroup = null;
    unsafeLocalHelperUsers = [ ];
    launcherUsers = cfg.site.launcherUsers;
    adminUsers = cfg.site.adminUsers;
    serverVersion = "0.4.0";
    acceptedClientVersionRange = ">=0.4.0, <0.5.0";
    enableResourcePlane = true;
    autostartParallelism = cfg.daemon.autostart.parallelism;
    gracefulShutdownTimeoutSeconds =
      cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds;
    liveActivationTimeoutSeconds =
      cfg.daemon.lifecycle.liveActivation.timeoutSeconds;
  };

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
in
{
  options.d2b.host.usbip.allowlist = lib.mkOption {
    type = lib.types.listOf (lib.types.submodule {
      options = {
        vendor = lib.mkOption {
          type = lib.types.strMatching "^0x[0-9A-Fa-f]{4}$";
          example = "0x1050";
          description = "Hex USB vendor ID allowed by the host broker.";
        };
        product = lib.mkOption {
          type = lib.types.strMatching "^0x[0-9A-Fa-f]{4}$";
          example = "0x0407";
          description = "Hex USB product ID allowed by the host broker.";
        };
      };
    });
    default = [ ];
    example = [ { vendor = "0x1050"; product = "0x0407"; } ];
    description = "Host-wide USBIP vendor/product allowlist.";
  };

  config = lib.mkIf cfg.daemonExperimental.enable {
    users.groups.d2bd = { };
    users.users.d2bd = {
      isSystemUser = true;
      group = "d2bd";
      description = "d2b daemon user";
      extraGroups = [ "d2b" ];
    };

    d2b._hostToolPackages = {
      d2b = d2bCliPackage;
      d2bd = d2bdPackage;
    };

    environment.systemPackages = [
      d2bdPackage
      d2bCliPackage
      d2bCliShellArtifactsPackage
      activationHelperPackage
    ];

    environment.etc."d2b/daemon-config.json" = {
      text = daemonConfigJson;
      mode = "0640";
      user = "root";
      group = "d2bd";
    };

    # The shared runtime root and host state root are declared in
    # `state-posture-contract.json` (`shared-run-dir`, `state-root`); derive
    # their lines instead of restating the posture here. The undeclared
    # children stay literal until they become contract levels.
    systemd.tmpfiles.rules =
      posture.tmpfilesRule "shared-run-dir" "."
      ++ [
        "f /run/d2b/daemon.lock 0640 d2bd d2bd -"
        "d /run/d2b/locks 0700 d2bd d2bd -"
        "d /run/d2b/locks/usbip 0750 root d2bd -"
        "d /run/d2b/state 0700 d2bd d2bd -"
      ]
      ++ posture.tmpfilesRule "state-root" "."
      ++ [
        "d /var/lib/d2b/volume-local-markers 0700 d2bd d2bd -"
        "d /var/lib/d2b/daemon-state 0700 d2bd d2bd -"
        # The TPM state policy root the trusted `path:tpm-state` /
        # `path:swtpm-state:<guest>` storage rows name (the TPM Provider's
        # state Volume resolves `<root>/<volume-name>` under it). The daemon
        # owns it and the volume-local controller creates each Device's
        # subdirectory inside.
        "d ${toString cfg.site.stateDir}/tpm-state 0700 d2bd d2bd -"
        "d /var/cache/d2b 0750 root d2bd -"
        "d /etc/d2b 0750 root d2bd -"
      ]
      # Each Device's TPM worker traverses the daemon-owned root into the
      # state directory the controller creates for it (0700, owned by the
      # Device's TPM principal): the long-lived swtpm worker writes its NVRAM,
      # log, and ctrl socket there, and the one-shot flush connects to that
      # ctrl socket. `d2bLib.deviceTpmPrincipals` derives both principals from
      # the same trusted rows the runtime uses.
      ++ lib.concatMap
        (row: [
          "a+ ${toString cfg.site.stateDir}/tpm-state - - - - u:${row.account}:--x"
          "a+ ${toString cfg.site.stateDir}/tpm-state - - - - u:${row.flushAccount}:--x"
        ])
        tpmPrincipals;

    systemd.services.d2bd = {
      description = "d2b daemon";
      wantedBy = [ "multi-user.target" ];
      wants = [
        "d2b-broker.socket"
        "systemd-tmpfiles-setup.service"
      ];
      after = [
        "systemd-tmpfiles-setup.service"
        "network.target"
        "d2b-broker.socket"
        "d2b-broker.service"
        "dbus.socket"
        "dbus.service"
        "d2b.slice"
      ];
      serviceConfig = {
        Type = "notify";
        NotifyAccess = "main";
        TimeoutStartSec = "5min";
        KillMode = "process";
        User = "d2bd";
        Group = "d2bd";
        ExecStart = "${d2bdPackage}/bin/d2bd host --config /etc/d2b/daemon-config.json";
        ExecStop = "+${hostShutdownHook}";
        TimeoutStopSec =
          lib.mkDefault "${toString cfg.daemon.lifecycle.gracefulShutdown.timeoutSeconds}s";
        Restart = "on-failure";
        RestartSec = "2s";
        NoNewPrivileges = true;
        CapabilityBoundingSet = [ "" ];
        AmbientCapabilities = [ "" ];
        PrivateTmp = true;
        ProtectHome = true;
        RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
        UMask = "0027";
        SupplementaryGroups = [ "d2b" ];
      };
    };
  };
}
