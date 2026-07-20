# Host-session clipboard authority and picker wiring.
{ lib, config, pkgs, ... }:

let
  cfg = config.d2b.site.clipboard;
  site = config.d2b.site;
  d2bLib = import ./lib.nix { inherit lib; };

  mib = n: n * 1024 * 1024;
  nonNegativeInt = lib.types.ints.unsigned;
  systemdExecArg = arg: builtins.replaceStrings [ "%" "$" ] [ "%%" "$$" ] (lib.escapeShellArg arg);
  systemdExecArgs = args: lib.concatMapStringsSep " " systemdExecArg args;
  packagesSrc = d2bLib.cleanRustPackagesSource ../packages;
  clipdSourcePackage = pkgs.rustPlatform.buildRustPackage {
    pname = "d2b-clipd";
    version = "2.0.0";
    src = packagesSrc;
    cargoLock = {
      lockFile = ../packages/Cargo.lock;
      outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";
    };
    cargoBuildFlags = [ "--package" "d2b-clipd" ];
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
      install -Dm755 target/x86_64-unknown-linux-gnu/release/d2b-clipd \
        $out/bin/d2b-clipd 2>/dev/null \
        || install -Dm755 target/release/d2b-clipd $out/bin/d2b-clipd
      runHook postInstall
    '';
  };

  clipdExec =
    if cfg.clipd.executablePath != null then cfg.clipd.executablePath
    else if cfg.clipd.package != null then "${cfg.clipd.package}/bin/d2b-clipd"
    else "${clipdSourcePackage}/bin/d2b-clipd";

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
  indexedWorkloads = config.d2b._index.workloads.enabledList;
  displayBindings = config.d2b._index.providerRegistryV2Mappings.display;
  displayBindingFor = workload:
    lib.findFirst
      (binding: binding.workloadId == workload.workloadId)
      null
      displayBindings;
  bridgeEndpointFor = workload:
    let
      runtime = workload.providerBindings.runtime or null;
      display = displayBindingFor workload;
      sameUid = runtime != null && runtime.implementationId == "systemd-user";
      supportedRuntime =
        runtime != null
        && builtins.elem runtime.implementationId
          [ "cloud-hypervisor" "qemu-media" "systemd-user" ];
      waylandUserAllowed =
        !sameUid
        || (site.waylandUser != null
          && lib.elem site.waylandUser
            config.d2b.realms.${workload.realmName}.allowedUsers);
      ownerPrincipal =
        if sameUid
        then site.waylandUser
        else "d2b-role-${display.ownerRoleId}";
    in
    if display == null || !supportedRuntime || !waylandUserAllowed
    then null
    else {
      inherit (workload) canonicalTarget realmId workloadId;
      runtimeProviderId = runtime.providerId;
      displayProviderId = display.providerId;
      socketComponent = display.endpointIds.proxy;
      inherit ownerPrincipal sameUid;
      expectedUid =
        if sameUid
        then config.users.users.${site.waylandUser}.uid
        else d2bLib.stablePrincipalId ownerPrincipal;
    };
  bridgeEndpoints = lib.filter (endpoint: endpoint != null)
    (map bridgeEndpointFor indexedWorkloads);
  bridgeWorkloads = map (endpoint: endpoint.canonicalTarget) bridgeEndpoints;
  bridgePeers = map (endpoint: {
    inherit (endpoint)
      canonicalTarget realmId workloadId runtimeProviderId displayProviderId expectedUid;
  }) bridgeEndpoints;
  userServiceEndpointUsers = lib.unique (
    site.adminUsers
    ++ site.launcherUsers
    ++ lib.concatMap
      (realm: config.d2b.realms.${realm.realmName}.allowedUsers)
      config.d2b._index.realms.enabledList
  );
  waylandUserHasEndpointTraversal =
    config.d2b.daemonExperimental.enable
    && lib.elem site.waylandUser userServiceEndpointUsers;
  waylandUid = toString config.users.users.${site.waylandUser}.uid;
  waylandGroup = config.users.users.${site.waylandUser}.group;
  userEndpointTmpfiles =
    lib.optional (!waylandUserHasEndpointTraversal)
      "a+ /run/d2b - - - - u:${site.waylandUser}:--x"
    ++ [
      "d /run/d2b/u 0711 root root -"
      "z /run/d2b/u 0711 root root -"
      "d /run/d2b/u/${waylandUid} 0700 ${site.waylandUser} ${waylandGroup} -"
      "z /run/d2b/u/${waylandUid} 0700 ${site.waylandUser} ${waylandGroup} -"
      "d /run/d2b/u/${waylandUid}/clipd 0700 ${site.waylandUser} ${waylandGroup} -"
      "z /run/d2b/u/${waylandUid}/clipd 0700 ${site.waylandUser} ${waylandGroup} -"
    ];
  clipdBridgeRootTmpfiles =
    lib.optionals (cfg.enable && bridgeEndpoints != [ ]) (
      [
        "d ${cfg.runtime.bridgeRoot} 0750 root d2b -"
        "z ${cfg.runtime.bridgeRoot} 0750 root d2b -"
        "a+ ${cfg.runtime.bridgeRoot} - - - - u:${site.waylandUser}:--x"
        "d ${cfg.runtime.bridgeRoot}/${waylandUid} 0710 root root -"
        "z ${cfg.runtime.bridgeRoot}/${waylandUid} 0710 root root -"
        "a+ ${cfg.runtime.bridgeRoot}/${waylandUid} - - - - u:${site.waylandUser}:--x"
        "d ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge 0710 root root -"
        "z ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge 0710 root root -"
        "a+ ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge - - - - u:${site.waylandUser}:--x"
      ]
      ++ lib.concatMap
        (endpoint:
          if endpoint.sameUid
          then [
            "d ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge/${endpoint.socketComponent} 0700 ${site.waylandUser} ${waylandGroup} -"
            "z ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge/${endpoint.socketComponent} 0700 ${site.waylandUser} ${waylandGroup} -"
          ]
          else [
            "d ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge/${endpoint.socketComponent} 0770 ${site.waylandUser} ${endpoint.ownerPrincipal} -"
            "z ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge/${endpoint.socketComponent} 0770 ${site.waylandUser} ${endpoint.ownerPrincipal} -"
            "a+ /run/d2b - - - - u:${endpoint.ownerPrincipal}:--x"
            "a+ ${cfg.runtime.bridgeRoot} - - - - u:${endpoint.ownerPrincipal}:--x"
            "a+ ${cfg.runtime.bridgeRoot}/${waylandUid} - - - - u:${endpoint.ownerPrincipal}:--x"
            "a+ ${cfg.runtime.bridgeRoot}/${waylandUid}/bridge - - - - u:${endpoint.ownerPrincipal}:--x"
          ])
        bridgeEndpoints
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
      bridgeSocketTemplate = "${cfg.runtime.bridgeRoot}/<uid>/bridge/<endpoint>/${cfg.runtime.bridgeSocketName}";
      inherit bridgeWorkloads;
      inherit bridgePeers;
      inherit bridgeEndpoints;
      parentProvisioning = "d2bd-broker-lifecycle";
      staticTmpfilesOnly = false;
    };
  } + "\n";

in
{
  options.d2b.site.clipboard = {
    enable = lib.mkEnableOption ''
      d2b host-session clipboard authority (`d2b-clipd`) and workload
      clipboard bridge wiring
    '';

    clipd = {
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        example = lib.literalExpression "pkgs.d2b-clipd";
        description = ''
          Package providing `bin/d2b-clipd`. When unset, d2b builds the
          authenticated clipboard service from this framework revision.
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
          description = "Retention action for entries attributed to a workload when it locks.";
        };
        onVmPause = lib.mkOption {
          type = lib.types.enum [ "keep" "quarantine" "drop" ];
          default = "quarantine";
          description = "Retention action for entries attributed to a workload when it pauses.";
        };
        onVmStop = lib.mkOption {
          type = lib.types.enum [ "keep" "quarantine" "drop" ];
          default = "drop";
          description = "Retention action for entries attributed to a workload when it stops.";
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
          description = "Per-workload cap on concurrently held transfer FDs.";
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
        description = "Timeout for the explicit paste action to publish and replay the selected item.";
      };
    };

    protocol = {
      pickerToClipdMaxFrameBytes = lib.mkOption {
        type = lib.types.ints.between 256 65536;
        default = 4096;
        description = "Maximum picker-to-daemon JSON line size.";
      };
      clipdToPickerMaxFrameBytes = lib.mkOption {
        type = lib.types.ints.between 4096 67108864;
        default = 23553408;
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
          native-paste popups off unless the explicit d2b paste action is enabled.
        '';
      };
      fallback = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Enable the explicit d2b-owned paste action: a Niri binding opens the
            picker for the focused target, then d2b-clipd publishes the selected
            item and requests paste replay before `ttl.fallbackArmingSeconds`.
          '';
        };
        command = lib.mkOption {
          type = lib.types.str;
          default = "d2b clipboard arm";
          description = "Command operators bind in niri for the explicit paste action.";
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
        description = "Capture allowed workload clipboard selections through the Wayland bridge.";
      };
      hostCrossRealmPicker = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = "Allow host-destination cross-realm picker prompts when trusted intent exists.";
      };
      vmCrossRealmPicker = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Allow workload-destination cross-realm picker prompts when trusted intent exists.";
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
        type = lib.types.strMatching "^/run/d2b/clipd(/[A-Za-z0-9_-]+)*$";
        default = "/run/d2b/clipd";
        description = ''
          Broker-provisioned root for per-user/per-workload clipboard bridge
          sockets. The effective template is
          `<bridgeRoot>/<uid>/bridge/<endpoint>/<bridgeSocketName>`, where the
          endpoint is the opaque proxy endpoint id from the canonical display
          provider binding.
          The user service must not create `/run/d2b` parents; d2bd and the
          broker own parent creation, traversal ACLs, endpoint ACLs, and teardown.
        '';
      };
      bridgeSocketName = lib.mkOption {
        type = lib.types.strMatching "^[A-Za-z0-9_.-]+\\.sock$";
        default = "clip.sock";
        readOnly = true;
        description = "Basename for each per-workload internal clipboard bridge socket.";
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
        assertion = !(lib.hasInfix "/." cfg.runtime.bridgeRoot);
        message = ''
          d2b.site.clipboard.runtime.bridgeRoot must not contain dot path
          segments. Use a broker-owned absolute path under /run/d2b/clipd.
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
    systemd.tmpfiles.rules = clipdBridgeRootTmpfiles ++ userEndpointTmpfiles;

    systemd.user.sockets = lib.genAttrs [
      "d2b-clipd-control"
      "d2b-clipd-picker"
      "d2b-clipd-bridge"
    ] (unitName:
      let purpose = lib.removePrefix "d2b-clipd-" unitName;
      in {
        description = "d2b authenticated clipboard ${purpose} endpoint";
        wantedBy = [ "sockets.target" ];
        unitConfig.ConditionUser = site.waylandUser;
        socketConfig = {
          ListenSequentialPacket = "/run/d2b/u/%U/clipd/${purpose}.sock";
          FileDescriptorName = "clipboard-${purpose}";
          SocketMode = "0600";
          DirectoryMode = "0700";
          RemoveOnStop = true;
          Service = "d2b-clipd.service";
        };
      });

    systemd.user.services.d2b-clipd = {
      description = "d2b clipboard authority daemon";
      documentation = [
        "file:/etc/d2b/clipboard.json"
      ];
      partOf = [ "graphical-session.target" ];
      requires = [
        "d2b-clipd-control.socket"
        "d2b-clipd-picker.socket"
        "d2b-clipd-bridge.socket"
      ];
      after = [
        "graphical-session.target"
        "d2b-clipd-control.socket"
        "d2b-clipd-picker.socket"
        "d2b-clipd-bridge.socket"
      ];
      unitConfig = {
        ConditionUser = site.waylandUser;
        AssertEnvironment = [ "WAYLAND_DISPLAY" ]
          ++ lib.optional cfg.niri.enable "NIRI_SOCKET";
      };
      serviceConfig = {
        Type = "simple";
        ExecStart = systemdExecArgs [ clipdExec ];
        Restart = "on-failure";
        RestartPreventExitStatus = "78";
        RestartSec = "2s";
        UMask = "0077";
        NoNewPrivileges = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
      };
    };
  };
}
