# Nix-unit coverage for Wave 6 interaction Provider assertions.
{ mkEval, lib, pkgs, ... }:

let
  artifacts = {
    display-wayland = {
      package = pkgs.writeText "d2b-test-display-wayland" "display-wayland";
      type = "provider";
    };
    notification-desktop = {
      package = pkgs.writeText "d2b-test-notification-desktop" "notification-desktop";
      type = "provider";
    };
    clipboard-wayland = {
      package = pkgs.writeText "d2b-test-clipboard-wayland" "clipboard-wayland";
      type = "provider";
    };
  };

  provider = artifactId: {
    type = "Provider";
    spec = {
      inherit artifactId;
      config = { };
    };
  };

  base = { ... }: {
    boot.loader.grub.enable = false;
    boot.loader.systemd-boot.enable = false;
    boot.initrd.includeDefaultModules = false;
    fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
    environment.etc."machine-id".text = "00000000000000000000000000000000";
    system.stateVersion = "25.11";

    d2b.artifacts = artifacts;
    d2b.zones.work.resources = {
      alice.type = "User";
      host = {
        type = "Host";
        spec.providerRef = "Provider/display-wayland";
      };
      guest = {
        type = "Guest";
        spec.providerRef = "Provider/notification-desktop";
      };
      display-wayland = provider "display-wayland";
      clipboard-wayland = {
        type = "Provider";
        spec = {
          artifactId = "clipboard-wayland";
          config = {
            hostExecutionRef = "Host/host";
            hostUserRef = "User/alice";
            displayWaylandRef = "Provider/display-wayland";
            guestSources = [{ guestRef = "Guest/guest"; }];
          };
        };
      };
      notification-desktop = {
        type = "Provider";
        spec = {
          artifactId = "notification-desktop";
          config = {
            hostExecutionRef = "Host/host";
            hostUserRef = "User/alice";
            displayWaylandRef = "Provider/display-wayland";
            maxPendingNotifications = 64;
            actionNonceTtlSecs = 120;
            actionNonceStoreSize = 256;
            acknowledgeTimeoutSecs = 3600;
            guestSources = [{
              guestRef = "Guest/guest";
              categories = [ "system.info" ];
            }];
          };
        };
      };
    };
  };

  failures = system:
    lib.filter
      (assertion:
        !(assertion.assertion or false)
        && lib.hasInfix "notification-desktop" (assertion.message or ""))
      system.config.assertions;

  failureFor = override:
    let system = mkEval [ base override ];
    in failures system;
in
{
  "interaction-providers/notification-valid-config" = {
    expr = failureFor { };
    expected = [ ];
  };

  "interaction-providers/clipboard-invalid-guest-source" = {
    expr = map (assertion: assertion.message)
      (lib.filter
        (assertion:
          !(assertion.assertion or false)
          && lib.hasInfix "clipboard-wayland" (assertion.message or ""))
        (mkEval [ base {
          d2b.zones.work.resources.clipboard-wayland.spec.config.guestSources = [ ];
        }]).config.assertions);
    expected = [
      "d2b.zones.work.resources.clipboard-wayland.spec.config.guestSources must contain between one and sixteen sources."
    ];
  };

  "interaction-providers/clipboard-missing-display-ref" = {
    expr = map (assertion: assertion.message)
      (lib.filter
        (assertion:
          !(assertion.assertion or false)
          && lib.hasInfix "clipboard-wayland" (assertion.message or ""))
        (mkEval [ base {
          d2b.zones.work.resources.clipboard-wayland.spec.config.displayWaylandRef = null;
        }]).config.assertions);
    expected = [
      "d2b.zones.work.resources.clipboard-wayland.spec.config.displayWaylandRef must select Provider/display-wayland."
    ];
  };

  "interaction-providers/notification-invalid-display-ref" = {
    expr = map (assertion: assertion.message)
      (failureFor {
        d2b.zones.work.resources.notification-desktop.spec.config.displayWaylandRef =
          "Provider/not-display";
      });
    expected = [
      "d2b.zones.work.resources.notification-desktop: every ResourceRef must be canonical and resolve in the same Zone."
      "d2b.zones.work.resources.notification-desktop.spec.config.displayWaylandRef must select Provider/display-wayland."
    ];
  };

  "interaction-providers/notification-invalid-nonce-store-size" = {
    expr = map (assertion: assertion.message)
      (failureFor {
        d2b.zones.work.resources.notification-desktop.spec.config.actionNonceStoreSize = 63;
      });
    expected = [
      "d2b.zones.work.resources.notification-desktop.spec.config.actionNonceStoreSize must be between 64 and 4096."
    ];
  };

  "interaction-providers/notification-invalid-acknowledge-timeout" = {
    expr = map (assertion: assertion.message)
      (failureFor {
        d2b.zones.work.resources.notification-desktop.spec.config.acknowledgeTimeoutSecs = 86401;
      });
    expected = [
      "d2b.zones.work.resources.notification-desktop.spec.config.acknowledgeTimeoutSecs must be between 1 and 86400."
    ];
  };
}
