{ config, lib, name, nixlingInputs, pkgs, ... }:

let
  cfg = config.nixling.guestControl;
  guestPackages = nixlingInputs.self.packages.${pkgs.stdenv.hostPlatform.system};
  usernamePattern = "^[a-z][a-z0-9_-]{0,31}$";
  unique = xs: lib.length xs == lib.length (lib.unique xs);
  usernameValid = user: builtins.match usernamePattern user != null;
  userExists = user:
    let
      userCfg = config.users.users.${user};
    in
    builtins.hasAttr user config.users.users
    && ((userCfg.isNormalUser or false) || (userCfg.isSystemUser or false));
  # Detached exec is served only when root exec is allowed; the guest
  # mirror gates it on exec.enable && exec.allowRoot, matching guestd's
  # own usability check (both abs flags present => detached runtime).
  detachedEnabled = cfg.exec.enable && cfg.exec.allowRoot;
  userdServices =
    if cfg.exec.enable then
      lib.listToAttrs (map (user: lib.nameValuePair "nixling-userd-${user}" {
        description = "nixling guest user daemon for ${user}";
        wantedBy = [ ];
        serviceConfig = {
          Type = "exec";
          User = user;
          RuntimeDirectory = "nixling-userd-${user}";
          RuntimeDirectoryMode = "0700";
          ExecStart = "${guestPackages.nixling-userd-static}/bin/nixling-userd";
        };
      }) cfg.exec.users)
    else
      { };
in
{
  options.nixling.guestControl = {
    enable = lib.mkOption {
      type = lib.types.bool;
      internal = true;
      readOnly = true;
      description = "Whether nixling's guest-control credential surface is wired in this guest.";
    };

    guestConfigPath = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      internal = true;
      readOnly = true;
      description = ''
        Absolute in-guest path of the operator-editable guest config
        working copy that `nixling config sync` reads back over the
        authenticated guest-control channel. Host-owned, derived from
        `nixling.vms.<vm>.guestConfigFile` independently of any SSH
        metadata. When non-null, guestd advertises the `ReadGuestFile`
        capability and serves a bounded read of exactly this path; when
        null there is nothing to sync and the capability stays absent
        (config sync fails closed).
      '';
    };

    exec = {
      enable = lib.mkOption {
        type = lib.types.bool;
        internal = true;
        readOnly = true;
        description = "Host-owned guest exec policy enable bit.";
      };

      allowRoot = lib.mkOption {
        type = lib.types.bool;
        internal = true;
        readOnly = true;
        description = "Host-owned root exec policy gate.";
      };

      users = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        internal = true;
        readOnly = true;
        description = "Host-owned non-root guest exec user allowlist.";
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
  };

  config = {
    warnings =
      lib.optional (cfg.exec.enable && cfg.exec.users != [ ]) ''
        nixling.guestControl.exec.users is set, but non-root guest exec is not
        yet served by the guest exec runtime. Until non-root exec lands, the
        guestd runtime only honours root exec (guest.exec.allowRoot = true) and
        rejects every non-root request. The user allowlist is reserved for a
        future wave and has no runtime effect today.
      '';

    assertions = [
      {
        assertion =
          !cfg.exec.enable
          || cfg.enable;
        message = ''
          nixling.guestControl.exec.enable requires nixling.guestControl.enable.
          Set nixling.vms.<vm>.guest.control.enable = true on the host-side VM
          option before enabling guest exec policy.
        '';
      }
      {
        assertion =
          cfg.exec.enable
          || (!cfg.exec.allowRoot && cfg.exec.users == [ ]);
        message = ''
          nixling.guestControl.exec.allowRoot/users were set while exec policy
          is disabled. Use the host-side nixling.vms.<vm>.guest.exec options
          instead of overriding internal guest-control policy.
        '';
      }
      {
        assertion =
          !cfg.exec.enable
          || cfg.exec.allowRoot
          || cfg.exec.users != [ ];
        message = ''
          nixling.guestControl.exec.enable is true, but no exec target is
          allowed. Add at least one host-side guest.exec.users entry or set
          guest.exec.allowRoot = true.
        '';
      }
      {
        assertion = unique cfg.exec.users;
        message = "nixling.guestControl.exec.users must not contain duplicates.";
      }
      {
        assertion = lib.all usernameValid cfg.exec.users;
        message = ''
          nixling.guestControl.exec.users entries must match ${usernamePattern}.
          Wildcards, root-like names, path separators, and systemd specifiers
          are not accepted.
        '';
      }
      {
        assertion = !(builtins.elem "root" cfg.exec.users);
        message = ''
          nixling.guestControl.exec.users must not include root. Use the
          host-side guest.exec.allowRoot option for the separate root policy.
        '';
      }
    ] ++ map (user: {
      assertion = userExists user;
      message = ''
        nixling.guestControl.exec.users contains ${user}, but that user is not
        declared as a normal or system user inside the guest.
      '';
    }) cfg.exec.users;

    environment.systemPackages = [
      guestPackages.nixling-guestd-static
      guestPackages.nixling-userd-static
      guestPackages.nixling-exec-runner-static
    ];

    systemd.services = {
      nixling-guestd = lib.mkIf cfg.enable {
        description = "nixling guest control daemon";
        wantedBy = [ ];
        unitConfig.RequiresMountsFor = [ "/run/nixling-guest-control-host" ];
        serviceConfig = {
          Type = "exec";
          ExecStart =
            let
              execFlags =
                lib.optionalString cfg.exec.enable " --exec-enable"
                + lib.optionalString (cfg.exec.enable && cfg.exec.allowRoot) " --exec-allow-root"
                + lib.optionalString (cfg.exec.enable && cfg.exec.allowRoot)
                    " --interactive-max-runtime-sec ${toString cfg.exec.interactiveMaxRuntimeSec}";
              detachedFlags =
                lib.optionalString detachedEnabled (
                  " --systemd-run-path ${pkgs.systemd}/bin/systemd-run"
                  + " --exec-runner-path ${guestPackages.nixling-exec-runner-static}/bin/nixling-exec-runner"
                  + " --detached-max-runtime-sec ${toString cfg.exec.detachedMaxRuntimeSec}"
                );
              # config sync read surface: advertised iff the host
              # declared a guestConfigFile (path threaded independently
              # of ssh.user). guestd gates the ReadGuestFile capability
              # on this flag being present.
              configFlags =
                lib.optionalString (cfg.guestConfigPath != null)
                  " --guest-config-path ${lib.escapeShellArg cfg.guestConfigPath}";
            in
            "${guestPackages.nixling-guestd-static}/bin/nixling-guestd --serve --vm-id ${lib.escapeShellArg name}${execFlags}${detachedFlags}${configFlags}";
          LoadCredential = [
            "guest_control_token:/run/nixling-guest-control-host/token"
          ];
        };
      };
    } // userdServices;

    # Detached exec parent dir: root-owned, 0700, boot-scoped so it
    # survives a guestd restart for re-adoption (D = clear at boot, NOT
    # on every guestd restart). Do NOT make this guestd's RuntimeDirectory
    # without RuntimeDirectoryPreserve, else a restart wipes adoptable state.
    systemd.tmpfiles.rules = lib.mkIf detachedEnabled [
      "D /run/nixling-exec 0700 root root -"
    ];

    # Guest-internal slice that scopes every per-exec transient unit
    # (nixling-exec-NN.service). Slot-keyed unit names bound metadata
    # cardinality to <=32 stable values that carry no exec id.
    systemd.slices."nixling-exec" = lib.mkIf detachedEnabled {
      description = "nixling detached guest exec slice";
    };
  };
}
