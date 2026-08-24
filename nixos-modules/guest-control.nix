{ config, lib, name, d2bInputs ? { }, d2bHostTools ? null, pkgs, ... }:

let
  cfg = config.d2b.guestControl;
  system = pkgs.stdenv.hostPlatform.system;
  self =
    if builtins.isAttrs d2bInputs && builtins.hasAttr "self" d2bInputs
    then d2bInputs.self
    else { };
  flakePackages =
    if builtins.isAttrs self
      && builtins.hasAttr "packages" self
      && builtins.hasAttr system self.packages
    then self.packages.${system}
    else { };
  packageFrom = name:
    if d2bHostTools != null
      && builtins.hasAttr name d2bHostTools
      && builtins.getAttr name d2bHostTools != null
    then builtins.getAttr name d2bHostTools
    else if builtins.hasAttr name flakePackages
      && builtins.getAttr name flakePackages != null
    then builtins.getAttr name flakePackages
      else throw "d2b Guest package '${name}' is unavailable for ${system}";
  d2bdPackage =
    if d2bHostTools != null
      && builtins.hasAttr "d2bd" d2bHostTools
      && builtins.getAttr "d2bd" d2bHostTools != null
    then d2bHostTools.d2bd
    else packageFrom "d2bd-guest-static";
  shellRunnerPackage = packageFrom "d2b-guest-shell-runner-static";
  guestUidDefault =
    let
      digest = builtins.hashString "sha256" "d2b-guest/${name}";
    in
    "${builtins.substring 0 8 digest}-${builtins.substring 8 4 digest}-4${builtins.substring 13 3 digest}-8${builtins.substring 17 3 digest}-${builtins.substring 20 12 digest}";
  # These deterministic values keep standalone per-VM evaluation total. The
  # enrollment owner must replace them with the Zone-issued identity and
  # signed session schema before a Guest can establish a live session.
  schemaFingerprintDefault =
    "sha256:${builtins.hashString "sha256" "d2b-guest-component-session-v3"}";
  runtimePath = value:
    builtins.isString value
    && lib.hasPrefix "/" value
    && value != "/nix/store"
    && !(lib.hasPrefix "/nix/store/" value)
    && !(builtins.elem ".." (lib.splitString "/" value))
    && !(lib.hasSuffix "/" value);
  usernamePattern = "^[a-z][a-z0-9_-]{0,31}$";
  usernameValid = user: builtins.match usernamePattern user != null;
  userExists = user:
    let
      userCfg = config.users.users.${user};
    in
    builtins.hasAttr user config.users.users
    && ((userCfg.isNormalUser or false) || (userCfg.isSystemUser or false));
  # Exec runtime is wired whenever exec is enabled for a workload user.
  # The detached runtime paths + substrate (parent dir + slice) are part
  # of a both-or-neither bundle with the target-local Process runtime.
  execRuntimeEnabled = cfg.exec.enable && cfg.exec.execUser != null;
in
{
  options.d2b.guestControl = {
    enable = lib.mkOption {
      type = lib.types.bool;
      internal = true;
      readOnly = true;
      description = "Whether this Guest target agent is enabled.";
    };

    guestConfigPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      internal = true;
      readOnly = true;
      description = ''
        Absolute in-guest path of the operator-editable guest config
        working copy that the config-nixos service reads back over the
        authenticated ComponentSession. Host-owned, derived from
        `d2b.vms.<vm>.guestConfigFile` independently of any SSH
        metadata. When null, the target agent uses its stable absent-config
        path and the service fails closed on a read.
      '';
    };

    usbipPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      internal = true;
      readOnly = true;
      description = ''
        Provider-local USBIP input retained for the owner-local migration
        surface. It is not passed as a target-agent command-line input.
      '';
    };

    wpctlPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      internal = true;
      description = ''
        Provider-local audio input retained for the owner-local migration
        surface. It is not passed as a target-agent command-line input.
      '';
    };

    componentSession = {
      guestRef = lib.mkOption {
        type = lib.types.strMatching "^Guest/[a-z][a-z0-9-]{0,62}$";
        default = "Guest/${name}";
        internal = true;
      };

      guestUid = lib.mkOption {
        type = lib.types.strMatching "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$";
        default = guestUidDefault;
        internal = true;
      };

      zone = lib.mkOption {
        type = lib.types.strMatching "^[a-z][a-z0-9-]{0,62}$";
        default = "local";
        internal = true;
      };

      purpose = lib.mkOption {
        type = lib.types.enum [ "zone-link" ];
        default = "zone-link";
        internal = true;
      };

      schemaFingerprint = lib.mkOption {
        type = lib.types.strMatching "^sha256:[0-9a-f]{64}$";
        default = schemaFingerprintDefault;
        internal = true;
      };

      reconnectGeneration = lib.mkOption {
        type = lib.types.ints.unsigned;
        default = 1;
        internal = true;
      };

      providerGeneration = lib.mkOption {
        type = lib.types.ints.unsigned;
        default = 1;
        internal = true;
      };

      controllerGeneration = lib.mkOption {
        type = lib.types.ints.unsigned;
        default = 1;
        internal = true;
      };

      assignmentEpoch = lib.mkOption {
        type = lib.types.ints.unsigned;
        default = 1;
        internal = true;
      };

      brokerSocketPath = lib.mkOption {
        type = lib.types.str;
        default = "/run/d2b/guest-broker.sock";
        internal = true;
      };

      brokerUid = lib.mkOption {
        type = lib.types.ints.unsigned;
        default = 997;
        internal = true;
      };

      stateDir = lib.mkOption {
        type = lib.types.str;
        default = "/var/lib/d2b/guest-state";
        internal = true;
      };

      bundlePath = lib.mkOption {
        type = lib.types.str;
        default = "/etc/d2b/guest-bundle.json";
        internal = true;
      };

      localPrivateKeyPath = lib.mkOption {
        type = lib.types.str;
        default = "/var/lib/d2b/component-session/guest.key";
        internal = true;
      };

      parentPublicKeyPath = lib.mkOption {
        type = lib.types.str;
        default = "/var/lib/d2b/component-session/parent.pub";
        internal = true;
      };
    };

    exec = {
      enable = lib.mkOption {
        type = lib.types.bool;
        internal = true;
        readOnly = true;
        description = "Host-owned guest exec policy enable bit.";
      };

      execUser = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        internal = true;
        readOnly = true;
        description = ''
          Host-fixed workload user every guest exec runs as (never root).
          Derived from the per-VM workload user (`ssh.user`). When non-null,
          the target-local Process Provider runs every exec as this user in a
          real PAM login session
          (`systemd-run --property=PAMName=login --uid=<name>`).
        '';
      };

      detachedMaxRuntimeSec = lib.mkOption {
        type = lib.types.ints.unsigned;
        internal = true;
        readOnly = true;
        description = ''
          Host-owned default runtime ceiling (seconds) for detached execs.
          0 means no ceiling (indefinite runtime).
        '';
      };

      interactiveMaxRuntimeSec = lib.mkOption {
        type = lib.types.ints.unsigned;
        internal = true;
        readOnly = true;
        description = ''
          Host-owned default runtime ceiling (seconds) for interactive (TTY)
          execs. 0 means no ceiling (indefinite, connection-owned runtime).
        '';
      };
    };

    shell = {
      enable = lib.mkOption {
        type = lib.types.bool;
        internal = true;
        readOnly = true;
        description = "Host-owned persistent shell policy enable bit.";
      };

      defaultName = lib.mkOption {
        type = lib.types.strMatching "^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$";
        internal = true;
        readOnly = true;
        description = "Host-owned default persistent shell session name.";
      };

      maxSessions = lib.mkOption {
        type = lib.types.ints.between 1 256;
        internal = true;
        readOnly = true;
        description = "Host-owned maximum persistent shell sessions per VM.";
      };

      maxAttached = lib.mkOption {
        type = lib.types.ints.between 1 64;
        internal = true;
        readOnly = true;
        description = "Host-owned maximum attached persistent shell clients per VM.";
      };
    };
  };

  config = {
    assertions = [
      {
        assertion =
          !cfg.exec.enable
          || cfg.enable;
        message = ''
          d2b.guestControl.exec.enable requires d2b.guestControl.enable.
          Set d2b.vms.<vm>.guest.control.enable = true on the host-side VM
          option before enabling guest exec policy.
        '';
      }
      {
        # Exec runs as the workload user; a workload user MUST be configured.
        assertion = !cfg.exec.enable || cfg.exec.execUser != null;
        message = ''
          d2b.vms.<vm>.guest.exec.enable is true, but no workload user is
          configured. Guest exec always runs as the VM's workload user; set
          d2b.vms.<vm>.ssh.user to the in-guest user exec should run as.
        '';
      }
      {
        # The workload user must be a valid, non-root account.
        assertion =
          !cfg.exec.enable
          || cfg.exec.execUser == null
          || (usernameValid cfg.exec.execUser && cfg.exec.execUser != "root");
        message = ''
          d2b.vms.<vm>.ssh.user (used as the guest exec workload user) must
          match ${usernamePattern} and must not be root. Guest exec never runs
          as root; users elevate with sudo inside the session.
        '';
      }
      {
        # The workload user must exist in the guest so login/PAM can resolve it.
        assertion =
          !cfg.exec.enable
          || cfg.exec.execUser == null
          || userExists cfg.exec.execUser;
        message = ''
          d2b.vms.<vm>.ssh.user (the guest exec workload user) is not
          declared as a normal or system user inside the guest. Declare it (or
          enable the desktop/home-manager user) before enabling guest exec.
        '';
      }
      {
        # The workload user must not resolve to UID 0 (root) even under a
        # non-root name. The name check above rejects only the literal "root",
        # but the never-root contract is about the effective UID, so an explicit
        # `uid = 0` alias must also be rejected. The target-local Process
        # Provider performs the runtime half of that defense.
        assertion =
          !cfg.exec.enable
          || cfg.exec.execUser == null
          || !(builtins.hasAttr cfg.exec.execUser config.users.users)
          || (config.users.users.${cfg.exec.execUser}.uid or null) != 0;
        message = ''
          d2b.vms.<vm>.ssh.user (the guest exec workload user) is configured
          with uid = 0. Guest exec never runs as root; assign the workload user
          a non-zero uid.
        '';
      }
      {
        assertion = !cfg.shell.enable || cfg.enable;
        message = ''
          d2b.guestControl.shell.enable requires d2b.guestControl.enable.
          Set d2b.vms.<vm>.guest.control.enable = true on the host-side VM
          option before enabling persistent shell policy.
        '';
      }
      {
        assertion = !cfg.shell.enable || cfg.exec.enable;
        message = ''
          d2b.guestControl.shell.enable requires d2b.guestControl.exec.enable
          because persistent shells reuse the target-local Process terminal
          substrate.
        '';
      }
      {
        assertion = cfg.shell.maxAttached <= cfg.shell.maxSessions;
        message = ''
          d2b.guestControl.shell.maxAttached must be less than or equal to
          d2b.guestControl.shell.maxSessions.
        '';
      }
      {
        assertion = runtimePath cfg.componentSession.brokerSocketPath
          && runtimePath cfg.componentSession.stateDir
          && runtimePath cfg.componentSession.bundlePath
          && runtimePath cfg.componentSession.localPrivateKeyPath
          && runtimePath cfg.componentSession.parentPublicKeyPath;
        message = ''
          d2b.guestControl.componentSession paths must be absolute runtime
          paths outside /nix/store.
        '';
      }
    ];

    environment.systemPackages =
      [ d2bdPackage ]
      ++ lib.optional cfg.shell.enable shellRunnerPackage;

    environment.etc."shpool/config.toml" = lib.mkIf cfg.shell.enable {
      text = ''
        prompt_prefix = ""
      '';
    };

    systemd.services = {
      d2bd-guest = lib.mkIf cfg.enable {
        description = "d2b Guest target agent";
        wantedBy = [ "multi-user.target" ];
        wants = [ "d2b-broker-guest.socket" ];
        after = [ "d2b-broker-guest.socket" "network.target" ];
        serviceConfig = {
          Type = "simple";
          User = "d2bd";
          Group = "d2bd";
          ExecStart =
            let
              session = cfg.componentSession;
              configPath =
                if cfg.guestConfigPath == null
                then "/var/lib/d2b-guest/guest-config.nix"
                else cfg.guestConfigPath;
            in
            "${d2bdPackage}/bin/d2bd guest"
            + " --guest-ref ${lib.escapeShellArg session.guestRef}"
            + " --guest-uid ${lib.escapeShellArg session.guestUid}"
            + " --zone ${lib.escapeShellArg session.zone}"
            + " --purpose ${lib.escapeShellArg session.purpose}"
            + " --schema-fingerprint ${lib.escapeShellArg session.schemaFingerprint}"
            + " --reconnect-generation ${toString session.reconnectGeneration}"
            + " --provider-generation ${toString session.providerGeneration}"
            + " --controller-generation ${toString session.controllerGeneration}"
            + " --assignment-epoch ${toString session.assignmentEpoch}"
            + " --broker-socket ${lib.escapeShellArg session.brokerSocketPath}"
            + " --broker-uid ${toString session.brokerUid}"
            + " --state-dir ${lib.escapeShellArg session.stateDir}"
            + " --bundle-path ${lib.escapeShellArg session.bundlePath}"
            + " --guest-config-path ${lib.escapeShellArg configPath}"
            + " --local-private-key ${lib.escapeShellArg session.localPrivateKeyPath}"
            + " --parent-public-key ${lib.escapeShellArg session.parentPublicKeyPath}";
          NoNewPrivileges = true;
          CapabilityBoundingSet = [ "" ];
          AmbientCapabilities = [ "" ];
          PrivateTmp = true;
          ProtectHome = true;
          ProtectClock = true;
          ProtectProc = "invisible";
          RestrictAddressFamilies = [ "AF_UNIX" "AF_VSOCK" ];
          UMask = "0077";
          Restart = "on-failure";
          RestartSec = "2s";
          StandardOutput = "journal";
          StandardError = "journal";
          SyslogIdentifier = "d2bd-guest";
        };
        restartIfChanged = false;
      };

      d2b-shpool-daemon = lib.mkIf (cfg.shell.enable && cfg.exec.execUser != null) {
        description = "d2b persistent shell pool daemon";
        serviceConfig = {
          Type = "exec";
          User = cfg.exec.execUser;
          PAMName = "d2b-shpool-daemon";
          ExecStart =
            let
              daemonScript = pkgs.writeShellScript "d2b-shpool-daemon-start" ''
                set -eu
                uid="$(${pkgs.coreutils}/bin/id -u)"
                home="$HOME"
                export XDG_RUNTIME_DIR="/run/user/$uid"
                export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
                exec ${shellRunnerPackage}/bin/d2b-guest-shell-runner daemon \
                  --socket "$XDG_RUNTIME_DIR/d2b-shpool.sock" \
                  --home "$home"
              '';
            in
            "${daemonScript}";
          WorkingDirectory = "~";
          KillMode = "control-group";
          Delegate = true;
        };
      };
    };

    security.pam.services.d2b-shpool-daemon = lib.mkIf (cfg.shell.enable && cfg.exec.execUser != null) {
      # Do not start a pam_systemd session here: it migrates the daemon out of
      # the delegated system service cgroup. Linger keeps /run/user/<uid>
      # available while the daemon stays under systemd's service authority.
      startSession = false;
      setEnvironment = true;
      setLoginUid = true;
    };

    users.users = lib.mkIf (cfg.shell.enable && cfg.exec.execUser != null) {
      ${cfg.exec.execUser}.linger = true;
    };

    # Guest target state is boot-scoped. The ComponentSession key files are
    # enrolled inputs and are never generated from writable state here.
    systemd.tmpfiles.rules =
      lib.optionals cfg.enable [
        "d ${cfg.componentSession.stateDir} 0700 d2bd d2bd -"
        "d ${builtins.dirOf cfg.componentSession.localPrivateKeyPath} 0700 d2bd d2bd -"
      ]
      ++ lib.optionals execRuntimeEnabled [
        "D /run/d2b-exec 0700 root root -"
      ];

    # Guest-internal slice that scopes every per-exec transient slot unit
    # (d2b-exec-NN.service). Slot-keyed unit names bound metadata
    # cardinality to <=32 stable values that carry no exec id.
    systemd.slices."d2b-exec" = lib.mkIf execRuntimeEnabled {
      description = "d2b detached guest exec slice";
    };
  };
}
