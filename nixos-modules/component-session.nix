{ config, lib, name, d2bInputs ? { }, d2bHostTools ? null, pkgs, ... }:

let
  cfg = config.d2b.componentSession;
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
in
{
  options.d2b.componentSession = {
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
        assertion = !cfg.shell.enable || cfg.enable;
        message = ''
          d2b.componentSession.shell.enable requires d2b.componentSession.enable.
          Set d2b.vms.<vm>.guest.componentSession.enable = true on the host-side VM
          option before enabling persistent shell policy.
        '';
      }
      {
        assertion =
          !cfg.shell.enable
          || (config.d2b.sshUser != null && config.d2b.sshUser != "root");
        message = ''
          d2b.componentSession.shell.enable requires a configured non-root workload user.
          Set d2b.vms.<vm>.ssh.user to a non-root account so d2b.sshUser is populated
          before enabling persistent shell policy.
        '';
      }
      {
        assertion = cfg.shell.maxAttached <= cfg.shell.maxSessions;
        message = ''
          d2b.componentSession.shell.maxAttached must be less than or equal to
          d2b.componentSession.shell.maxSessions.
        '';
      }
      {
        assertion = runtimePath cfg.brokerSocketPath
          && runtimePath cfg.stateDir
          && runtimePath cfg.bundlePath
          && runtimePath cfg.localPrivateKeyPath
          && runtimePath cfg.parentPublicKeyPath;
        message = ''
          d2b.componentSession paths must be absolute runtime
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
              session = cfg;
              configPath =
                if cfg.guestConfigPath == null
                then "/var/lib/d2b/guest-config.nix"
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

      d2b-shpool-daemon = lib.mkIf (cfg.shell.enable && config.d2b.sshUser != null) {
        description = "d2b persistent shell pool daemon";
        serviceConfig = {
          Type = "exec";
          User = config.d2b.sshUser;
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

    security.pam.services.d2b-shpool-daemon = lib.mkIf (cfg.shell.enable && config.d2b.sshUser != null) {
      # Do not start a pam_systemd session here: it migrates the daemon out of
      # the delegated system service cgroup. Linger keeps /run/user/<uid>
      # available while the daemon stays under systemd's service authority.
      startSession = false;
      setEnvironment = true;
      setLoginUid = true;
    };

    users.users = lib.mkIf (cfg.shell.enable && config.d2b.sshUser != null) {
      ${config.d2b.sshUser}.linger = true;
    };

    # Guest target state is boot-scoped. The ComponentSession key files are
    # enrolled inputs and are never generated from writable state here.
    systemd.tmpfiles.rules =
      lib.optionals cfg.enable [
        "d ${cfg.stateDir} 0700 d2bd d2bd -"
        "d ${builtins.dirOf cfg.localPrivateKeyPath} 0700 d2bd d2bd -"
      ]
      ;
  };
}
