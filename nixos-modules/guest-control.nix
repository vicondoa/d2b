{ config, lib, name, d2bInputs, pkgs, ... }:

let
  cfg = config.d2b.guestControl;
  guestPackages = d2bInputs.self.packages.${pkgs.stdenv.hostPlatform.system};
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
  # of a both-or-neither bundle with the attached exec runtime; guestd
  # decides at runtime whether to actually serve detached.
  execRuntimeEnabled = cfg.exec.enable && cfg.exec.execUser != null;
in
{
  options.d2b.guestControl = {
    enable = lib.mkOption {
      type = lib.types.bool;
      internal = true;
      readOnly = true;
      description = "Whether d2b's guest-control credential surface is wired in this guest.";
    };

    guestConfigPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      internal = true;
      readOnly = true;
      description = ''
        Absolute in-guest path of the operator-editable guest config
        working copy that `d2b config sync` reads back over the
        authenticated guest-control channel. Host-owned, derived from
        `d2b.vms.<vm>.guestConfigFile` independently of any SSH
        metadata. When non-null, guestd advertises the `ReadGuestFile`
        capability and serves a bounded read of exactly this path; when
        null there is nothing to sync and the capability stays absent
        (config sync fails closed).
      '';
    };

    usbipPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      internal = true;
      readOnly = true;
      description = ''
        Absolute in-guest path to the USBIP CLI. Non-null only for guests
        with the USBIP component enabled; guestd then advertises the
        `UsbipImport` capability and owns guest-side USBIP attach/detach.
      '';
    };

    wpctlPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      internal = true;
      description = ''
        Absolute in-guest path to the `wpctl` binary (from the
        `wireplumber` package). Non-null only for guests with the audio
        component enabled. When non-null and the workload user is
        configured, guestd advertises the `AudioStatus` and `AudioSet`
        capabilities and serves PipeWire queries targeting the workload
        user's session via argv-only wpctl subprocesses.
      '';
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
          guestd is launched with `--exec-user <name>` and every exec runs the
          requested command as this user in a real PAM login session
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
        # `uid = 0` alias must also be rejected. The guestd daemon additionally
        # resolves the effective UID from the guest passwd DB at runtime and
        # refuses 0; this is the eval-time half of that defense.
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
          because persistent shells reuse the guest-control exec terminal substrate.
        '';
      }
      {
        assertion = cfg.shell.maxAttached <= cfg.shell.maxSessions;
        message = ''
          d2b.guestControl.shell.maxAttached must be less than or equal to
          d2b.guestControl.shell.maxSessions.
        '';
      }
    ];

    environment.systemPackages = [
      guestPackages.d2b-guestd-static
      guestPackages.d2b-exec-runner-static
    ];

    environment.etc."shpool/config.toml" = lib.mkIf cfg.shell.enable {
      text = ''
        prompt_prefix = ""
      '';
    };

    systemd.services = {
      d2b-guestd = lib.mkIf cfg.enable {
        description = "d2b guest control daemon";
        wantedBy = [ "multi-user.target" ];
        unitConfig.RequiresMountsFor = [ "/run/d2b-guest-control-host" ];
        serviceConfig = {
          Type = "exec";
          ExecStart =
            let
              execEnabledUser = cfg.exec.enable && cfg.exec.execUser != null;
              execFlags =
                lib.optionalString cfg.exec.enable " --exec-enable"
                + lib.optionalString execEnabledUser
                    " --exec-user ${lib.escapeShellArg cfg.exec.execUser}"
                + lib.optionalString execEnabledUser
                    " --interactive-max-runtime-sec ${toString cfg.exec.interactiveMaxRuntimeSec}";
              # The reachable exec paths (interactive PTY + non-interactive pipe)
              # run the command as the workload user in a PAM login session via
              # systemd-run, using the exec-runner PTY helper, so both binary
              # paths are wired whenever exec is enabled — not only for detached.
              # guestd treats the two paths as a both-or-neither bundle.
              execRuntimeFlags =
                lib.optionalString execEnabledUser (
                  " --systemd-run-path ${pkgs.systemd}/bin/systemd-run"
                  + " --exec-runner-path ${guestPackages.d2b-exec-runner-static}/bin/d2b-exec-runner"
                  + " --detached-max-runtime-sec ${toString cfg.exec.detachedMaxRuntimeSec}"
                );
              # config sync read surface: advertised iff the host
              # declared a guestConfigFile (path threaded independently
              # of ssh.user). guestd gates the ReadGuestFile capability
              # on this flag being present.
              configFlags =
                lib.optionalString (cfg.guestConfigPath != null)
                  " --guest-config-path ${lib.escapeShellArg cfg.guestConfigPath}";
              usbipFlags =
                lib.optionalString (cfg.usbipPath != null)
                  " --usbip-path ${lib.escapeShellArg cfg.usbipPath}";
              audioFlags =
                lib.optionalString (cfg.wpctlPath != null)
                  " --wpctl-path ${lib.escapeShellArg cfg.wpctlPath}";
              activationFlags =
                " --activation-systemd-run-path ${pkgs.systemd}/bin/systemd-run"
                + " --activation-systemctl-path ${pkgs.systemd}/bin/systemctl";
              shellFlags =
                lib.optionalString cfg.shell.enable (
                  " --shell-enable"
                  + " --shell-default-name ${lib.escapeShellArg cfg.shell.defaultName}"
                  + " --shell-max-sessions ${toString cfg.shell.maxSessions}"
                  + " --shell-max-attached ${toString cfg.shell.maxAttached}"
                  + lib.optionalString execEnabledUser
                      " --shell-runner-path ${guestPackages.d2b-guest-shell-runner-static}/bin/d2b-guest-shell-runner"
                  + lib.optionalString execEnabledUser
                      " --shell-systemctl-path ${pkgs.systemd}/bin/systemctl"
                );
            in
            "${guestPackages.d2b-guestd-static}/bin/d2b-guestd --serve --vm-id ${lib.escapeShellArg name}${execFlags}${execRuntimeFlags}${configFlags}${usbipFlags}${audioFlags}${activationFlags}${shellFlags}";
          LoadCredential = [
            "guest_control_token:/run/d2b-guest-control-host/token"
          ];
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
                exec ${guestPackages.d2b-guest-shell-runner-static}/bin/d2b-guest-shell-runner daemon \
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

    # Detached exec runtime substrate (parent dir + slice), declared as
    # part of the both-or-neither exec runtime bundle whenever exec is
    # enabled for a workload user. The parent dir is root-owned, 0700,
    # boot-scoped (D = clear at boot, NOT on every guestd restart) so
    # detached slot state survives a guestd restart for re-adoption. Do
    # NOT make this guestd's RuntimeDirectory without
    # RuntimeDirectoryPreserve, else a restart wipes adoptable state.
    systemd.tmpfiles.rules =
      lib.optionals cfg.enable [
        "d /run/d2b-guestd 0700 root root -"
        "d /run/d2b-guestd/activations 0700 root root -"
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
