# Clipboard Nix module option, service, and assertion coverage.
{ mkEval, lib, pkgs, ... }:

let
  fakeClipd = pkgs.writeShellScriptBin "d2b-clipd" "exit 0";
  fakePicker = pkgs.writeShellScriptBin "d2b-clip-picker" "exit 0";

  base = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";
    users.groups.desktop = { };
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
      group = "desktop";
    };
    users.users.bob = {
      isNormalUser = true;
      uid = 1001;
      group = "desktop";
    };
    d2b.site.waylandUser = "alice";
  };

  evalWith = overrides:
    mkEval ([ ../../../../nixos-modules/clipboard.nix base ] ++ overrides);
  failingMessages = nixos:
    map
      (assertion: assertion.message or "")
      (builtins.filter (assertion: !(assertion.assertion or false)) nixos.config.assertions);
  hasFailure = nixos: needle:
    lib.any (message: lib.hasInfix needle message) (failingMessages nixos);

  disabled = evalWith [ ];
  enabled = evalWith [
    ({ ... }: {
      d2b.site.clipboard = {
        enable = true;
        niri.external = true;
        clipd.package = fakeClipd;
        picker.package = fakePicker;
        policy = {
          requirePicker = true;
          crossRealm.enable = true;
        };
      };
    })
  ];
  service = enabled.config.systemd.user.services.d2b-clipd;
  serviceConfig = service.serviceConfig;
  unitConfig = service.unitConfig;
  clipboardEtc = enabled.config.environment.etc."d2b/clipboard.json";
  clipboardJson = builtins.fromJSON clipboardEtc.text;
  unsafeEnabled = evalWith [
    ({ ... }: {
      d2b.site.clipboard = {
        enable = true;
        niri.external = true;
        clipd.package = fakeClipd;
        picker.package = fakePicker;
      };
      d2b.realms.host = {
        allowedUsers = [ "alice" ];
        policy.allowUnsafeLocal = true;
        providers.runtime = {
          type = "runtime";
          implementationId = "systemd-user";
        };
        providers.display = {
          type = "display";
          implementationId = "wayland";
        };
        workloads.tools = {
          providerRefs = {
            runtime = "runtime";
            display = "display";
          };
          display.wayland = true;
          launcher.items.browser = {
            type = "exec";
            name = "Browser";
            argv = [ "firefox" ];
            graphical = true;
          };
        };
      };
    })
  ];
  unsafeClipboardJson =
    builtins.fromJSON unsafeEnabled.config.environment.etc."d2b/clipboard.json".text;

  desktopMetadataEval = lib.evalModules {
    modules = [
      ({ lib, ... }: {
        options = {
          assertions = lib.mkOption {
            type = lib.types.listOf lib.types.anything;
            default = [ ];
          };
          d2b._uiColors = lib.mkOption {
            type = lib.types.anything;
          };
          d2b._bundle.extraArtifacts = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
          d2b._bundle.unsafeLocalWorkloadsJson = lib.mkOption {
            type = lib.types.anything;
          };
          d2b.realms = lib.mkOption {
            type = lib.types.attrsOf lib.types.anything;
            default = { };
          };
        };
        config.d2b = {
          realms.work = {
            name = "Work";
            path = "work.local-root";
            providers.systemd-user = {
              id = "systemd-user";
              type = "runtime";
              implementationId = "systemd-user";
              capabilities = [ "exec" ];
            };
            providers.wayland = {
              id = "wayland";
              type = "display";
              implementationId = "wayland";
              capabilities = [ "display.open" ];
            };
            workloads.tools = {
              id = "tools";
              providerRefs = {
                runtime = "systemd-user";
                display = "wayland";
              };
              display.wayland = true;
              launcher = {
                enable = true;
                label = "Tools";
                icon = {
                  id = "applications-utilities";
                  name = null;
                };
                defaultItem = "browser";
                capabilities = [ "configured-launch" ];
                items.browser = {
                  type = "exec";
                  name = "Browser";
                  graphical = true;
                  icon = {
                    id = "web-browser";
                    name = null;
                  };
                  argv = [ "firefox" "https://example.test/" ];
                };
              };
            };
          };
          _uiColors.realms.work.accent = "#ff6600";
        };
      })
      ../../../../nixos-modules/index.nix
      ../../../../nixos-modules/desktop-metadata-json.nix
      ../../../../nixos-modules/unsafe-local-workloads-json.nix
    ];
  };
  desktopArtifact =
    desktopMetadataEval.config.d2b._bundle.extraArtifacts.desktopMetadataJson;
  desktopMetadata = desktopArtifact.data;
  desktopMetadataJson = builtins.toJSON desktopMetadata;
  privateLauncher =
    builtins.head desktopMetadataEval.config.d2b._bundle.unsafeLocalWorkloadsJson.data.workloads;
  desktopIndex = desktopMetadataEval.config.d2b._index;
  desktopRealmId = builtins.head desktopIndex.realms.ids;
  desktopWorkloadId = builtins.head desktopIndex.workloads.ids;
  desktopSystemdUserProviderId =
    (builtins.head desktopIndex.workloads.enabledList).providerBindings.runtime.providerId;
  unsafeWorkload = builtins.head unsafeEnabled.config.d2b._index.workloads.enabledList;
  unsafeRuntimeProvider = unsafeWorkload.providerBindings.runtime;
  unsafeDisplayBinding =
    builtins.head unsafeEnabled.config.d2b._index.providerRegistryV2Mappings.display;
  unsafePrivateWorkload =
    builtins.head
      unsafeEnabled.config.d2b._bundle.unsafeLocalWorkloadsJson.data.workloads;
  unsafeEndpointRules = unsafeEnabled.config.systemd.tmpfiles.rules;
  hasUnsafeEndpoint = uid:
    lib.any
      (rule: lib.hasInfix "/run/d2b/u/${toString uid} " rule)
      unsafeEndpointRules;
in
{
  "unsafe-local/runtime-user-endpoints-follow-normalized-bindings" = {
    expr = {
      allowedUsers = unsafeEnabled.config.d2b.realms.host.allowedUsers;
      allowUnsafeLocal =
        unsafeEnabled.config.d2b.realms.host.policy.allowUnsafeLocal;
      normalizedRowsExcludeAllowedUsers =
        lib.all
          (realm: !(realm ? allowedUsers))
          unsafeEnabled.config.d2b._index.realms.enabledList;
      aliceEndpoint = hasUnsafeEndpoint 1000;
      bobEndpoint = hasUnsafeEndpoint 1001;
      privateIdentity = {
        inherit (unsafePrivateWorkload.identity)
          canonicalTarget providerId realmId runtimeKind workloadId;
      };
      privateArgv = (builtins.head unsafePrivateWorkload.items).argv;
      runtimeSocket =
        unsafeEnabled.config.systemd.user.sockets.d2b-runtime-systemd-user
          .socketConfig.ListenSequentialPacket;
      userServiceSocket =
        unsafeEnabled.config.systemd.user.sockets.d2b-userd.socketConfig
          .ListenSequentialPacket;
    };
    expected = {
      allowedUsers = [ "alice" ];
      allowUnsafeLocal = true;
      normalizedRowsExcludeAllowedUsers = true;
      aliceEndpoint = true;
      bobEndpoint = false;
      privateIdentity = {
        inherit (unsafeWorkload) canonicalTarget realmId workloadId;
        providerId = unsafeRuntimeProvider.providerId;
        runtimeKind = "systemd-user";
      };
      privateArgv = [ "firefox" ];
      runtimeSocket = "/run/d2b/u/%U/runtime-agent.sock";
      userServiceSocket = "/run/d2b/u/%U/userd.sock";
    };
  };

  "desktop-metadata/artifact-contract" = {
    expr = {
      inherit (desktopArtifact) installFileName classification sensitivity;
      inherit (desktopMetadata) schemaVersion runtimeState;
      privateIdentity = privateLauncher.identity;
      privateArgv = (builtins.head privateLauncher.items).argv;
      publicContainsArgv = lib.hasInfix "\"argv\"" desktopMetadataJson;
    };
    expected = {
      installFileName = "desktop-metadata.json";
      classification = "contractPublic";
      sensitivity = "nonSecret";
      schemaVersion = "v2";
      runtimeState = "presentation-only";
      privateIdentity = {
        canonicalTarget = "tools.work.local-root.d2b";
        providerId = desktopSystemdUserProviderId;
        realmId = desktopRealmId;
        realmPath = [ "work" "local-root" ];
        runtimeKind = "systemd-user";
        workloadId = desktopWorkloadId;
      };
      privateArgv = [ "firefox" "https://example.test/" ];
      publicContainsArgv = false;
    };
  };

  "desktop-metadata/canonical-key-shape" = {
    expr = {
      realmKeys = builtins.attrNames desktopMetadata.realms;
      workloadKeys = builtins.attrNames desktopMetadata.workloads;
      workload = desktopMetadata.workloads."tools.work.local-root.d2b";
    };
    expected = {
      realmKeys = [ desktopRealmId ];
      workloadKeys = [ "tools.work.local-root.d2b" ];
      workload = {
        canonicalTarget = "tools.work.local-root.d2b";
        realmId = desktopRealmId;
        workloadId = desktopWorkloadId;
        providerId = desktopSystemdUserProviderId;
        executionPosture = {
          isolation = "unsafe-local";
          environment = "systemd-user-manager-ambient";
          displayEnvironment = "wayland-proxy-only";
          executionIdentity = "authenticated-requester-uid";
          sessionPersistence = "user-manager-lifetime";
        };
        label = "Tools";
        icon = {
          id = "applications-utilities";
        };
        realmAccentColor = "#ff6600";
        launcherEnabled = true;
        defaultItemId = "browser";
        capabilities = [ "configured-launch" ];
        items = [
          {
            id = "browser";
            type = "exec";
            name = "Browser";
            graphical = true;
            icon = {
              id = "web-browser";
            };
            capabilities = [ "configured-launch" "window-forwarding" ];
          }
        ];
      };
    };
  };

  "desktop-metadata/unsafe-local-uses-systemd-user-provider" = {
    expr = desktopMetadata.providers.${desktopSystemdUserProviderId};
    expected = {
      providerId = desktopSystemdUserProviderId;
      realmId = desktopRealmId;
      canonicalTarget = "systemd-user.work.local-root.d2b";
      implementation = "systemd-user";
      label = "systemd-user";
      capabilities = [ "exec" ];
    };
  };

  "desktop-metadata/presentation-is-non-authoritative" = {
    expr = desktopMetadata.invariants;
    expected = {
      argvPrivate = true;
      canonicalIdsOnly = true;
      canonicalTargetsOnly = true;
      colorsArePresentationOnly = true;
      metadataIsNotAuthorization = true;
      nonAuthoritativeProjection = true;
      noSecretsOrCredentials = true;
    };
  };

  "desktop-metadata/no-legacy-aliases-or-argv" = {
    expr = lib.all
      (field: !(lib.hasInfix field desktopMetadataJson))
      [
        "\"appCommand\""
        "\"argv\""
        "\"legacyVmName\""
        "\"providerKind\""
        "\"runtimeKind\""
        "\"targetAddress\""
        "\"workloadName\""
        "https://example.test/"
      ];
    expected = true;
  };

  "desktop-metadata/assertions-hold" = {
    expr = lib.all (assertion: assertion.assertion)
      desktopMetadataEval.config.assertions;
    expected = true;
  };

  "clipboard/disabled-no-user-service" = {
    expr = builtins.hasAttr "d2b-clipd" disabled.config.systemd.user.services;
    expected = false;
  };

  "clipboard/enabled-user-service" = {
    expr = builtins.hasAttr "d2b-clipd" enabled.config.systemd.user.services;
    expected = true;
  };

  "clipboard/user-service-graphical-target" = {
    expr = {
      wantedBy = service.wantedBy;
      partOf = service.partOf;
      after = service.after;
      conditionUser = unitConfig.ConditionUser;
      socketConditionUsers = map
        (name:
          enabled.config.systemd.user.sockets.${name}.unitConfig.ConditionUser)
        [
          "d2b-clipd-control"
          "d2b-clipd-picker"
          "d2b-clipd-bridge"
        ];
    };
    expected = {
      wantedBy = [ ];
      partOf = [ "graphical-session.target" ];
      after = [
        "graphical-session.target"
        "d2b-clipd-control.socket"
        "d2b-clipd-picker.socket"
        "d2b-clipd-bridge.socket"
      ];
      conditionUser = "alice";
      socketConditionUsers = [ "alice" "alice" "alice" ];
    };
  };

  "clipboard/user-service-assert-environment" = {
    expr = unitConfig.AssertEnvironment;
    expected = [ "WAYLAND_DISPLAY" "NIRI_SOCKET" ];
  };

  "clipboard/user-service-has-no-runtime-directory" = {
    expr = {
      hasDirectory = serviceConfig ? RuntimeDirectory;
      hasMode = serviceConfig ? RuntimeDirectoryMode;
    };
    expected = {
      hasDirectory = false;
      hasMode = false;
    };
  };

  "clipboard/user-service-restart-hardening" = {
    expr = {
      restart = serviceConfig.Restart;
      restartPreventExitStatus = serviceConfig.RestartPreventExitStatus;
      noNewPrivileges = serviceConfig.NoNewPrivileges;
      umask = serviceConfig.UMask;
      lockPersonality = serviceConfig.LockPersonality;
      restrictRealtime = serviceConfig.RestrictRealtime;
      restrictSuidSgid = serviceConfig.RestrictSUIDSGID;
    };
    expected = {
      restart = "on-failure";
      restartPreventExitStatus = "78";
      noNewPrivileges = true;
      umask = "0077";
      lockPersonality = true;
      restrictRealtime = true;
      restrictSuidSgid = true;
    };
  };

  "clipboard/execstart-uses-package-and-config" = {
    expr =
      lib.hasInfix "/bin/d2b-clipd" serviceConfig.ExecStart
      && !(lib.hasInfix "--config" serviceConfig.ExecStart)
      && !(lib.hasInfix "--bridge-root" serviceConfig.ExecStart);
    expected = true;
  };

  "clipboard/execstart-escapes-systemd-percent-and-dollar" = {
    expr =
      let
        weirdExec = evalWith [
          ({ ... }: {
            d2b.site.clipboard = {
              enable = true;
              niri.external = true;
              clipd.executablePath = "/run/current-system/sw/bin/d2b-clipd%";
              picker.executablePath = "/run/current-system/sw/bin/d2b-clip-picker$";
            };
          })
        ];
        execStart = weirdExec.config.systemd.user.services.d2b-clipd.serviceConfig.ExecStart;
      in
      lib.hasInfix "d2b-clipd%%" execStart && !(lib.hasInfix "d2b-clip-picker$$" execStart);
    expected = true;
  };

  "clipboard/bridge-root-allows-safe-child" = {
    expr =
      let
        safe = evalWith [
          ({ ... }: {
            d2b.site.clipboard = {
              enable = true;
              niri.external = true;
              clipd.package = fakeClipd;
              runtime.bridgeRoot = "/run/d2b/clipd/child_dir-1";
            };
          })
        ];
      in
      safe.config.d2b.site.clipboard.runtime.bridgeRoot;
    expected = "/run/d2b/clipd/child_dir-1";
  };

  "clipboard/bridge-root-rejects-dot-segment" = {
    expr =
      (evalWith [
        ({ ... }: {
          d2b.site.clipboard = {
            enable = true;
            niri.external = true;
            clipd.package = fakeClipd;
            runtime.bridgeRoot = "/run/d2b/clipd/..";
          };
        })
      ]).config.d2b.site.clipboard.runtime.bridgeRoot;
    expectedError = { };
  };

  "clipboard/etc-config-mode" = {
    expr = clipboardEtc.mode;
    expected = "0644";
  };

  "clipboard/config-records-socketpair-picker" = {
    expr = {
      version = clipboardJson.version;
      noDefaultPickerInput = clipboardJson.picker.noDefaultPickerInput;
      usesSocketpair = clipboardJson.picker.usesSocketpair;
    };
    expected = {
      version = 1;
      noDefaultPickerInput = true;
      usesSocketpair = true;
    };
  };

  "clipboard/config-records-bridge-template" = {
    expr = clipboardJson.runtime.bridgeSocketTemplate;
    expected = "/run/d2b/clipd/<uid>/bridge/<endpoint>/clip.sock";
  };

  "clipboard/unsafe-local-endpoint-is-canonical-and-same-uid" = {
    expr =
      let endpoint = builtins.head unsafeClipboardJson.runtime.bridgeEndpoints;
      in {
        inherit (endpoint)
          canonicalTarget realmId workloadId runtimeProviderId displayProviderId
          socketComponent expectedUid sameUid;
      };
    expected = {
      canonicalTarget = unsafeWorkload.canonicalTarget;
      inherit (unsafeWorkload) realmId workloadId;
      runtimeProviderId = unsafeRuntimeProvider.providerId;
      displayProviderId = unsafeDisplayBinding.providerId;
      socketComponent = unsafeDisplayBinding.endpointIds.proxy;
      expectedUid = 1000;
      sameUid = true;
    };
  };

  "clipboard/unsafe-local-bridge-dir-is-user-owned" = {
    expr = lib.any
      (rule:
        lib.hasInfix
          "/bridge/${unsafeDisplayBinding.endpointIds.proxy} 0700 alice desktop"
          rule)
      unsafeEnabled.config.systemd.tmpfiles.rules;
    expected = true;
  };

  "clipboard/wayland-user-can-traverse-bridge-parents" = {
    expr =
      let
        traverseRule = "a+ /run/d2b - - - - u:alice:--x";
        endpointParentRule = "d /run/d2b/u 0711 root root -";
        endpointRulesText =
          lib.concatStringsSep "\n" enabled.config.systemd.tmpfiles.rules;
      in
      {
        traversalRules =
          lib.hasInfix "${traverseRule}\n${endpointParentRule}" endpointRulesText
          && lib.all
            (rule: builtins.elem rule unsafeEnabled.config.systemd.tmpfiles.rules)
            [
              traverseRule
              "a+ /run/d2b/clipd - - - - u:alice:--x"
            ];
        normalizedRowsExcludeAllowedUsers =
          lib.all
            (realm: !(realm ? allowedUsers))
            unsafeEnabled.config.d2b._index.realms.enabledList;
      };
    expected = {
      traversalRules = true;
      normalizedRowsExcludeAllowedUsers = true;
    };
  };

  "clipboard/no-static-clipd-tmpfiles" = {
    expr =
      lib.any
        (rule: lib.hasInfix "/run/d2b/clipd" rule)
        (enabled.config.systemd.tmpfiles.rules or [ ]);
    expected = false;
  };

  "clipboard/missing-clipd-fails" = {
    expr = hasFailure
      (evalWith [ ({ ... }: { d2b.site.clipboard = { enable = true; niri.external = true; }; }) ])
      "d2b.site.clipboard.enable requires d2b.site.clipboard.clipd.package";
    expected = true;
  };

  "clipboard/picker-required-fails-without-picker" = {
    expr = hasFailure
      (evalWith [
        ({ ... }: {
          d2b.site.clipboard = {
            enable = true;
            niri.external = true;
            clipd.package = fakeClipd;
            policy.requirePicker = true;
          };
        })
      ])
      "default GPL picker input";
    expected = true;
  };

  "clipboard/invalid-total-cap-fails" = {
    expr = hasFailure
      (evalWith [
        ({ ... }: {
          d2b.site.clipboard = {
            enable = true;
            niri.external = true;
            clipd.package = fakeClipd;
            caps.perItemMaxBytes = 2048;
            caps.totalMemoryMaxBytes = 1024;
          };
        })
      ])
      "totalMemoryMaxBytes must be greater than or";
    expected = true;
  };

  "clipboard/invalid-mime-cap-fails" = {
    expr = hasFailure
      (evalWith [
        ({ ... }: {
          d2b.site.clipboard = {
            enable = true;
            niri.external = true;
            clipd.package = fakeClipd;
            caps.perItemMaxBytes = 2048;
            caps.mimeMaxBytes."text/plain" = 4096;
          };
        })
      ])
      "Every d2b.site.clipboard.caps.mimeMaxBytes value";
    expected = true;
  };

  "clipboard/invalid-frame-cap-fails" = {
    expr = hasFailure
      (evalWith [
        ({ ... }: {
          d2b.site.clipboard = {
            enable = true;
            niri.external = true;
            clipd.package = fakeClipd;
            protocol.pickerToClipdMaxFrameBytes = 4096;
            protocol.clipdToPickerMaxFrameBytes = 4096;
          };
        })
      ])
      "clipdToPickerMaxFrameBytes must be";
    expected = true;
  };

  "clipboard/missing-wayland-user-fails" = {
    expr = hasFailure
      (mkEval [
        ../../../../nixos-modules/clipboard.nix
        ({ ... }: {
          boot.loader.grub.enable = false;
          boot.loader.systemd-boot.enable = false;
          boot.initrd.includeDefaultModules = false;
          fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
          environment.etc."machine-id".text = "00000000000000000000000000000000";
          system.stateVersion = "25.11";
          d2b.site.clipboard = {
            enable = true;
            niri.external = true;
            clipd.package = fakeClipd;
          };
        })
      ])
      "d2b.site.clipboard.enable requires d2b.site.waylandUser";
    expected = true;
  };

  "clipboard/niri-prerequisite-fails" = {
    expr = hasFailure
      (evalWith [
        ({ ... }: {
          d2b.site.clipboard = {
            enable = true;
            clipd.package = fakeClipd;
          };
        })
      ])
      "d2b.site.clipboard.niri.enable requires programs.niri.enable";
    expected = true;
  };
}
