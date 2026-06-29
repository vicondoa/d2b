# Host-session clipboard authority and picker wiring.
{ lib, config, pkgs, ... }:

let
  cfg = config.d2b.site.clipboard;
  site = config.d2b.site;
  d2bLib = import ./lib.nix { inherit lib; };

  mib = n: n * 1024 * 1024;
  nonNegativeInt = lib.types.ints.unsigned;
  optionalArg = cond: arg: lib.optionalString cond " ${arg}";

  clipdExec =
    if cfg.clipd.executablePath != null then cfg.clipd.executablePath
    else if cfg.clipd.package != null then "${cfg.clipd.package}/bin/d2b-clipd"
    else "${pkgs.coreutils}/bin/false";

  pickerExec =
    if cfg.picker.executablePath != null then cfg.picker.executablePath
    else if cfg.picker.package != null then "${cfg.picker.package}/bin/d2b-clip-picker"
    else null;

  pickerConfigured = pickerExec != null;
  pickerExecForJson =
    if pickerExec == null then null else builtins.unsafeDiscardStringContext pickerExec;
  pickerRequired =
    cfg.policy.requirePicker
    || (cfg.policy.crossRealm.enable && cfg.policy.crossRealm.requirePicker)
    || (cfg.policy.crossRealm.enable && (cfg.modes.hostCrossRealmPicker || cfg.modes.vmCrossRealmPicker));

  niriProgramEnabled = config.programs.niri.enable or false;
  bridgeVmSet =
    (lib.filterAttrs (_name: vm:
      vm.enable && vm.graphics.enable && vm.graphics.crossDomainTrusted && vm.graphics.waylandProxy.enable
    ) (d2bLib.normalNixosVms config.d2b.vms))
    // (d2bLib.qemuMediaVms config.d2b.vms);
  bridgeVms = lib.attrNames bridgeVmSet;
  bridgePeers = map (vm: {
    vmName = vm;
    expectedUid = d2bLib.stablePrincipalId "d2b-${vm}-wlproxy";
  }) bridgeVms;
  clipdBridgeRootTmpfiles =
    lib.optionals (cfg.enable && bridgeVms != [ ]) (
      [
        "d ${cfg.runtime.bridgeRoot} 0750 root d2b -"
        "z ${cfg.runtime.bridgeRoot} 0750 root d2b -"
      ]
      ++ lib.concatMap (vm: [
        "d ${cfg.runtime.bridgeRoot}/${toString (config.users.users.${site.waylandUser}.uid)}/bridge/${vm} 0770 ${site.waylandUser} d2b-${vm}-wlproxy -"
        "z ${cfg.runtime.bridgeRoot}/${toString (config.users.users.${site.waylandUser}.uid)}/bridge/${vm} 0770 ${site.waylandUser} d2b-${vm}-wlproxy -"
      ]) bridgeVms
    );

  configJson = builtins.toJSON {
    version = 1;
    clipd = {
      runtimeDirectory = "d2b-clipd";
    };
    picker = {
      executable = pickerExecForJson;
      usesSocketpair = true;
      noDefaultPickerInput = true;
    };
    policy = cfg.policy;
    caps = cfg.caps;
    ttl = cfg.ttl;
    protocol = cfg.protocol;
    niri = cfg.niri;
    modes = cfg.modes;
    runtime = {
      bridgeRoot = cfg.runtime.bridgeRoot;
      bridgeSocketTemplate = "${cfg.runtime.bridgeRoot}/<uid>/bridge/<vm>/${cfg.runtime.bridgeSocketName}";
      inherit bridgeVms;
      inherit bridgePeers;
      parentProvisioning = "d2bd-broker-lifecycle";
      staticTmpfilesOnly = false;
    };
  } + "\n";

  serviceArgs =
    "--config /etc/d2b/clipboard.json"
    + optionalArg (pickerExec != null) "--picker ${lib.escapeShellArg pickerExec}"
    + " --bridge-root ${lib.escapeShellArg cfg.runtime.bridgeRoot}";
in
{
  options.d2b.site.clipboard = {
    enable = lib.mkEnableOption ''
      d2b host-session clipboard authority (`d2b-clipd`) and VM
      clipboard bridge wiring
    '';

    clipd = {
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        example = lib.literalExpression "pkgs.d2b-clipd";
        description = ''
          Package providing `bin/d2b-clipd`. The crate/package may be
          supplied by a later integration lane; this module intentionally
          accepts an external package reference and does not build the daemon
          itself.
        '';
      };

      executablePath = lib.mkOption {
        type = lib.types.nullOr (lib.types.strMatching "^/.*");
        default = null;
        example = "/run/current-system/sw/bin/d2b-clipd";
        description = ''
          Absolute path to the `d2b-clipd` executable. Overrides
          `clipd.package` when set and lets operators test a locally packaged
          daemon before a flake package exists.
        '';
      };
    };

    picker = {
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        example = lib.literalExpression "inputs.d2b-clip-picker.packages.${"$"}{pkgs.system}.default";
        description = ''
          Optional package providing `bin/d2b-clip-picker`. There is no
          default because the picker is a separate GPL-licensed UI project and
          is not a d2b flake input.
        '';
      };

      executablePath = lib.mkOption {
        type = lib.types.nullOr (lib.types.strMatching "^/.*");
        default = null;
        example = "/run/current-system/sw/bin/d2b-clip-picker";
        description = ''
          Absolute path to the picker binary. Use this instead of
          `picker.package` when the picker is installed by a separate flake or
          profile.
        '';
      };
    };

    policy = {
      requirePicker = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Require an operator-configured picker for all UI-mediated transfers.
          Enabling this without `picker.package` or `picker.executablePath`
          fails evaluation.
        '';
      };

      sameRealmDefaultPaste = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Allow policy-compatible same-realm paste without a picker prompt.";
      };

      crossRealm = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = "Allow cross-realm clipboard transfers when explicit rules and paste intent are present.";
        };

        requirePicker = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Require the picker for cross-realm transfers.";
        };

        requirePasteIntent = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Require a trusted paste-intent token for cross-realm transfers.";
        };
      };

      cleanup = {
        onVmLock = lib.mkOption {
          type = lib.types.enum [ "keep" "quarantine" "drop" ];
          default = "quarantine";
          description = "Retention action for entries attributed to a VM when it locks.";
        };
        onVmPause = lib.mkOption {
          type = lib.types.enum [ "keep" "quarantine" "drop" ];
          default = "quarantine";
          description = "Retention action for entries attributed to a VM when it pauses.";
        };
        onVmStop = lib.mkOption {
          type = lib.types.enum [ "keep" "quarantine" "drop" ];
          default = "drop";
          description = "Retention action for entries attributed to a VM when it stops.";
        };
        onVmDestroy = lib.mkOption {
          type = lib.types.enum [ "drop" ];
          default = "drop";
          description = "Entries attributed to destroyed VMs are always dropped.";
        };
      };
    };

    caps = {
      perItemMaxBytes = lib.mkOption {
        type = lib.types.ints.positive;
        default = mib 8;
        description = "Maximum bytes retained for one clipboard item.";
      };
      totalMemoryMaxBytes = lib.mkOption {
        type = lib.types.ints.positive;
        default = mib 64;
        description = "Maximum in-memory clipboard payload bytes across all entries.";
      };
      mimeMaxBytes = lib.mkOption {
        type = lib.types.attrsOf lib.types.ints.positive;
        default = {
          "text/plain;charset=utf-8" = mib 1;
          "text/plain" = mib 1;
          "text/html" = mib 2;
          "image/png" = mib 8;
        };
        description = "Per-MIME materialization byte caps for the initial allowlist.";
      };
      maxCandidates = lib.mkOption {
        type = lib.types.ints.between 1 500;
        default = 50;
        description = "Maximum picker candidates in one request.";
      };
      maxPreviewBytes = lib.mkOption {
        type = lib.types.ints.between 0 8192;
        default = 1024;
        description = "Maximum redacted preview bytes exposed to the picker.";
      };
      maxThumbnailBytes = lib.mkOption {
        type = nonNegativeInt;
        default = 0;
        description = "Maximum thumbnail bytes exposed to the picker; zero disables thumbnails.";
      };
      heldFds = {
        global = lib.mkOption {
          type = lib.types.ints.between 1 4096;
          default = 256;
          description = "Global cap on concurrently held Wayland transfer FDs.";
        };
        perUid = lib.mkOption {
          type = lib.types.ints.between 1 1024;
          default = 128;
          description = "Per-host-UID cap on concurrently held transfer FDs.";
        };
        perVm = lib.mkOption {
          type = lib.types.ints.between 1 512;
          default = 64;
          description = "Per-VM cap on concurrently held transfer FDs.";
        };
      };
      materializationPerMinute = lib.mkOption {
        type = lib.types.ints.between 1 6000;
        default = 120;
        description = "Per-source eager copy materialization rate cap.";
      };
    };

    ttl = {
      historySeconds = lib.mkOption {
        type = lib.types.ints.positive;
        default = 1800;
        description = "Default in-memory history TTL.";
      };
      pickerRequestSeconds = lib.mkOption {
        type = lib.types.ints.between 1 120;
        default = 15;
        description = "Maximum time a picker request may stay open.";
      };
      pasteIntentSeconds = lib.mkOption {
        type = lib.types.ints.between 1 10;
        default = 2;
        description = "Trusted paste-intent token TTL.";
      };
      pendingFdSeconds = lib.mkOption {
        type = lib.types.ints.between 1 120;
        default = 10;
        description = "Maximum time a pending paste transfer FD may be held.";
      };
      fallbackArmingSeconds = lib.mkOption {
        type = lib.types.ints.between 1 30;
        default = 5;
        description = "Timeout for explicit fallback arming before the native paste request arrives.";
      };
    };

    protocol = {
      pickerToClipdMaxFrameBytes = lib.mkOption {
        type = lib.types.ints.between 256 65536;
        default = 4096;
        description = "Maximum picker-to-daemon JSON line size.";
      };
      clipdToPickerMaxFrameBytes = lib.mkOption {
        type = lib.types.ints.between 4096 1048576;
        default = 262144;
        description = "Maximum daemon-to-picker JSON line size.";
      };
      bridgeMaxFrameBytes = lib.mkOption {
        type = lib.types.ints.between 4096 1048576;
        default = 65536;
        description = "Maximum internal bridge message size excluding SCM_RIGHTS payload FDs.";
      };
    };

    niri = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Enable direct `$NIRI_SOCKET` integration for focused-window labels and paste-intent hooks.";
      };
      external = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Set to true when niri is provided outside the NixOS `programs.niri`
          module. When false, enabling clipboard requires
          `programs.niri.enable = true`.
        '';
      };
      pasteIntentHook = lib.mkOption {
        type = lib.types.enum [ "disabled" "niri-hook" "upstream-ipc" ];
        default = "disabled";
        description = ''
          Trusted host paste-intent source. `disabled` keeps host cross-realm
          native-paste popups off unless the explicit fallback is enabled.
        '';
      };
      fallback = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Enable the explicit two-step fallback: a d2b-owned action opens the
            picker, then the user performs the normal app paste within
            `ttl.fallbackArmingSeconds`.
          '';
        };
        command = lib.mkOption {
          type = lib.types.str;
          default = "d2b clipboard arm";
          description = "Command operators bind in niri for the explicit fallback action.";
        };
        notification = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Show content-free fallback ready/timeout notifications.";
        };
      };
    };

    modes = {
      captureHostClipboard = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Capture allowed host clipboard selections into d2b in-memory history.";
      };
      captureVmClipboard = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Capture allowed VM clipboard selections through the Wayland bridge.";
      };
      hostCrossRealmPicker = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Allow host-destination cross-realm picker prompts when trusted intent exists.";
      };
      vmCrossRealmPicker = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Allow VM-destination cross-realm picker prompts when trusted intent exists.";
      };
      primarySelection = lib.mkOption {
        type = lib.types.enum [ "deny" ];
        default = "deny";
        description = "Primary selection is denied in clipboard protocol v1.";
      };
      dragAndDrop = lib.mkOption {
        type = lib.types.enum [ "deny" ];
        default = "deny";
        description = "Wayland drag-and-drop is denied in clipboard protocol v1.";
      };
    };

    runtime = {
      bridgeRoot = lib.mkOption {
        type = lib.types.strMatching "^/run/d2b/clipd(/.*)?$";
        default = "/run/d2b/clipd";
        description = ''
          Broker-provisioned root for per-user/per-VM clipboard bridge
          sockets. The effective template is
          `<bridgeRoot>/<uid>/bridge/<vm>/<bridgeSocketName>`.
          The user service must not create `/run/d2b` parents; d2bd and the
          broker own parent creation, traversal ACLs, per-VM ACLs, and teardown.
        '';
      };
      bridgeSocketName = lib.mkOption {
        type = lib.types.strMatching "^[A-Za-z0-9_.-]+\\.sock$";
        default = "clip.sock";
        description = "Basename for each per-VM internal clipboard bridge socket.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.clipd.package != null || cfg.clipd.executablePath != null;
        message = ''
          d2b.site.clipboard.enable requires d2b.site.clipboard.clipd.package
          or d2b.site.clipboard.clipd.executablePath. The d2b-clipd crate is
          wired by package reference here rather than implemented by this Nix
          module.
        '';
      }
      {
        assertion = !pickerRequired || pickerConfigured;
        message = ''
          d2b.site.clipboard policy requires a picker, but neither
          d2b.site.clipboard.picker.package nor
          d2b.site.clipboard.picker.executablePath is set. D2b does not ship a
          default GPL picker input; install d2b-clip-picker from its separate
          flake and pass the package or executable path explicitly.
        '';
      }
      {
        assertion = cfg.caps.totalMemoryMaxBytes >= cfg.caps.perItemMaxBytes;
        message = ''
          d2b.site.clipboard.caps.totalMemoryMaxBytes must be greater than or
          equal to caps.perItemMaxBytes.
        '';
      }
      {
        assertion = lib.all (bytes: bytes <= cfg.caps.perItemMaxBytes)
          (lib.attrValues cfg.caps.mimeMaxBytes);
        message = ''
          Every d2b.site.clipboard.caps.mimeMaxBytes value must be less than
          or equal to caps.perItemMaxBytes.
        '';
      }
      {
        assertion = cfg.protocol.clipdToPickerMaxFrameBytes > cfg.protocol.pickerToClipdMaxFrameBytes;
        message = ''
          d2b.site.clipboard.protocol.clipdToPickerMaxFrameBytes must be
          larger than pickerToClipdMaxFrameBytes because OpenRequest frames are
          intentionally larger than picker Select/Cancel frames.
        '';
      }
      {
        assertion = site.waylandUser != null;
        message = ''
          d2b.site.clipboard.enable requires d2b.site.waylandUser because
          d2b-clipd runs in the host Wayland session and uses WAYLAND_DISPLAY.
          NIRI_SOCKET is additionally required when d2b.site.clipboard.niri.enable
          is true.
        '';
      }
      {
        assertion = !cfg.niri.enable || cfg.niri.external || niriProgramEnabled;
        message = ''
          d2b.site.clipboard.niri.enable requires programs.niri.enable = true,
          unless niri is managed outside NixOS and
          d2b.site.clipboard.niri.external = true is set. d2b-clipd requires a
          live NIRI_SOCKET and does not shell out to `niri msg`.
        '';
      }
      {
        assertion = cfg.niri.pasteIntentHook != "disabled" || !cfg.modes.hostCrossRealmPicker || cfg.niri.fallback.enable;
        message = ''
          d2b.site.clipboard.modes.hostCrossRealmPicker requires a trusted
          niri paste-intent hook, an upstream-equivalent IPC event, or the
          explicit fallback workflow. Focused-window metadata alone is not a
          paste-intent token.
        '';
      }
      {
        assertion = !(cfg.policy.crossRealm.enable && cfg.policy.crossRealm.requirePasteIntent)
          || cfg.niri.pasteIntentHook != "disabled"
          || cfg.niri.fallback.enable;
        message = ''
          d2b.site.clipboard.policy.crossRealm.enable with requirePasteIntent
          needs a trusted niri paste-intent hook, an upstream-equivalent IPC
          event, or the explicit fallback workflow. Otherwise cross-realm
          transfers would be denied without an operator-visible path.
        '';
      }
    ];

    environment.etc."d2b/clipboard.json" = {
      text = configJson;
      mode = "0644";
    };

    systemd.tmpfiles.rules = clipdBridgeRootTmpfiles;

    systemd.user.services.d2b-clipd = {
      description = "d2b clipboard authority daemon";
      documentation = [
        "file:/etc/d2b/clipboard.json"
      ];
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      unitConfig.AssertEnvironment = [ "WAYLAND_DISPLAY" ]
        ++ lib.optional cfg.niri.enable "NIRI_SOCKET";
      serviceConfig = {
        Type = "simple";
        ExecStart = "${clipdExec} ${serviceArgs}";
        Restart = "on-failure";
        RestartSec = "2s";
        UMask = "0000";
        RuntimeDirectory = "d2b-clipd";
        RuntimeDirectoryMode = "0700";
        NoNewPrivileges = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
      };
    };
  };
}
