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

  # The verified deployment graph the daemon publishes before it starts any
  # provider. It is derived from the generated provider declarations (see
  # `zone-resources.nix`), so the implementation identities it names are the
  # compiled set and there is no separate configurable allowlist. The daemon
  # re-verifies the self-hash and refuses startup on a mismatch, so the file
  # is installed rather than trusted.
  deploymentBootstrap = cfg._bundle.deploymentBootstrap or { };
  deploymentBootstrapFile =
    pkgs.writeText "d2b-deployment-bootstrap.json"
      (deploymentBootstrap.documentJson or "{}");

  # The daemon and the broker share one deployment root, so both halves
  # bootstrap from the same verified document. This installs it into the
  # deployment root before the daemon starts; it is not a separate unit and
  # it does not widen the root-unit boundary.
  installDeploymentBootstrap = pkgs.writeShellScript
    "d2b-install-deployment-bootstrap" ''
    set -eu
    target_dir=${lib.escapeShellArg "${cfg.site.stateDir}"}
    install -d -m 0750 -o root -g d2bd "$target_dir"
    umask 0077
    tmp="$target_dir/.deployment-bootstrap.json.new"
    cat > "$tmp" < ${deploymentBootstrapFile}
    ${pkgs.coreutils}/bin/chown root:d2bd "$tmp"
    ${pkgs.coreutils}/bin/chmod 0640 "$tmp"
    ${pkgs.coreutils}/bin/mv -f "$tmp" \
      "$target_dir/${deploymentBootstrap.path or "deployment-bootstrap.json"}"
  '';

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
        # The shared parent of the per-guest runtime tree a Device's worker
        # binds its control socket under. A guest name is not known to any
        # static tmpfiles rule, so this rule owns the parent and the
        # broker's socket grant owns each `/run/d2b/vms/<guest>` leaf, which
        # it creates from the `path:vm-run:<guest>` storage row this path is
        # declared by. Without this rule the parent does not exist, the
        # grant refuses every worker that binds a runtime socket - which is
        # every worker except the one-shot flush, so the failure looks like
        # a TPM problem and is not one.
        #
        # Mode and ownership match the storage row exactly (1770 d2bd:d2b):
        # the worker enters as d2bd through the d2b group, and the two
        # declarations of this directory have to agree or the grant refuses
        # on posture rather than on absence.
        "d /run/d2b/vms 1770 d2bd d2b -"
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
      # state directory the controller creates for it (layout owner
      # `User/d2bd`, with the per-Device ACL grants for the Device's two TPM
      # principals): the long-lived swtpm worker writes its NVRAM, log, and
      # ctrl socket there, and the one-shot flush connects to that ctrl
      # socket. `d2bLib.deviceTpmPrincipals` derives both principals from the
      # same trusted rows the runtime uses.
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
        # U10 seam: the broker's envelope forwarder dials the daemon's
        # forward rendezvous; the daemon binds this path at startup.
        #
        # U31 seam: the deployment root both the daemon and the broker
        # bootstrap from. The verified deployment graph is installed into it
        # before the daemon starts; without one the daemon refuses to open
        # its resource plane rather than starting a provider under no
        # accepted graph.
        Environment = [
          "D2B_BROKER_FORWARD_SOCKET=/run/d2b/broker-forward.sock"
          "D2B_DEPLOYMENT_ROOT=${cfg.site.stateDir}"
        ];
        ExecStartPre = "+${installDeploymentBootstrap}";
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
