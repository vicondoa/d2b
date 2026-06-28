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
    users.users.alice = { isNormalUser = true; uid = 1000; };
    d2b.site.waylandUser = "alice";
  };

  evalWith = overrides: mkEval ([ base ] ++ overrides);
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
in
{
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
    };
    expected = {
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
    };
  };

  "clipboard/user-service-assert-environment" = {
    expr = unitConfig.AssertEnvironment;
    expected = [ "WAYLAND_DISPLAY" "NIRI_SOCKET" ];
  };

  "clipboard/user-service-runtime-directory" = {
    expr = {
      directory = serviceConfig.RuntimeDirectory;
      mode = serviceConfig.RuntimeDirectoryMode;
    };
    expected = {
      directory = "d2b-clipd";
      mode = "0700";
    };
  };

  "clipboard/user-service-restart-hardening" = {
    expr = {
      restart = serviceConfig.Restart;
      noNewPrivileges = serviceConfig.NoNewPrivileges;
      privateTmp = serviceConfig.PrivateTmp;
      lockPersonality = serviceConfig.LockPersonality;
      restrictRealtime = serviceConfig.RestrictRealtime;
      restrictSuidSgid = serviceConfig.RestrictSUIDSGID;
    };
    expected = {
      restart = "on-failure";
      noNewPrivileges = true;
      privateTmp = true;
      lockPersonality = true;
      restrictRealtime = true;
      restrictSuidSgid = true;
    };
  };

  "clipboard/execstart-uses-package-and-config" = {
    expr =
      lib.hasInfix "/bin/d2b-clipd" serviceConfig.ExecStart
      && lib.hasInfix "--config /etc/d2b/clipboard.json" serviceConfig.ExecStart
      && lib.hasInfix "--bridge-root /run/d2b/clipd" serviceConfig.ExecStart;
    expected = true;
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
    expected = "/run/d2b/clipd/<uid>/bridge/<vm>/clip.sock";
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
