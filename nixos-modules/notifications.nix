# d2b notification/event mechanism.
#
# Provides a reusable desktop notification and status layer for d2b
# capabilities. The first consumer is the USB security-key proxy feature;
# subsequent capabilities should extend this module rather than adding
# per-capability notification wiring.
#
# Architecture:
#   - The host runtime (d2bd / broker) writes durable JSON state to
#     `${runtime.stateDir}/sk-state.json` when security-key ceremony state
#     changes.
#   - `d2b-sk-waybar-helper` (enabled via `integrations.waybar.enable`) reads
#     the state file and emits a Waybar JSON block to stdout; the Waybar
#     `custom/d2b-sk` module polls it.
#   - Desktop notifications are emitted by d2bd when ceremonies transition.
#     Notification actions (e.g. "Cancel request") carry single-use nonces;
#     the CLI / daemon validates them before acting.
#   - `d2b-wlcontrol` (future) reads the state file via the
#     `WlcontrolSkStatus` data contract (see `packages/d2b-notify/src/wlcontrol.rs`).
{ lib, config, pkgs, ... }:

let
  cfg = config.d2b.notifications;

  waybarHelperExec =
    if cfg.statusHelper.executablePath != null
    then cfg.statusHelper.executablePath
    else if cfg.statusHelper.package != null
    then "${cfg.statusHelper.package}/bin/d2b-sk-waybar-helper"
    else "${pkgs.coreutils}/bin/false";

in
{
  options.d2b.notifications = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Enable the d2b notification/event layer.  When enabled, the
        framework creates the runtime state directory and activates any
        enabled sub-features (status helper, Waybar integration).
      '';
    };

    statusHelper = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Enable the `d2b-sk-waybar-helper` binary wrapper unit.  Requires
          `d2b.notifications.enable = true`.
        '';
      };

      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        description = ''
          Package providing `d2b-sk-waybar-helper`.  When `null` and
          `executablePath` is also `null`, the helper is disabled.
        '';
      };

      executablePath = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          Absolute path to the `d2b-sk-waybar-helper` binary, for
          development builds that do not go through a Nix package derivation.
          Takes precedence over `package` when set.
        '';
      };
    };

    integrations.waybar = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Enable Waybar integration.  Adds a `custom/d2b-sk` module
          configuration fragment (written as a Home Manager option comment
          here; operators add the snippet to their Waybar config).

          When enabled, the framework installs the `d2b-sk-waybar-helper`
          binary on the system path so Waybar's `exec` directive can find it
          without an absolute path.

          Requires `d2b.notifications.statusHelper.enable = true`.
        '';
      };
    };

    securityKey = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Enable security-key ceremony notifications.  When true, d2bd
          will emit desktop notifications for security-key events (started,
          touch-needed, busy, timed-out, failed, canceled) and update the
          durable state file read by the Waybar helper and wlcontrol.

          Requires `d2b.notifications.enable = true`.
        '';
      };

      staleEntryTtlSecs = lib.mkOption {
        type = lib.types.ints.positive;
        default = 300;
        description = ''
          Number of seconds after the last event after which a terminal
          ceremony entry may be pruned from the durable state file.
          Does not affect active (non-terminal) ceremonies.
        '';
      };
    };

    runtime = {
      stateDir = lib.mkOption {
        type = lib.types.path;
        default = "/run/d2b/notify";
        readOnly = true;
        description = ''
          Directory where the notification layer writes durable JSON state
          files.  Cleaned at boot by a systemd-tmpfiles `D` rule.
          Currently read-only; the path is reserved so future modules can
          co-locate state files here.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.integrations.waybar.enable -> cfg.statusHelper.enable;
        message = "d2b.notifications.integrations.waybar.enable requires d2b.notifications.statusHelper.enable = true";
      }
      {
        assertion = cfg.securityKey.enable -> cfg.enable;
        message = "d2b.notifications.securityKey.enable requires d2b.notifications.enable = true";
      }
    ];

    # Transient per-boot state directory for notification state files.
    systemd.tmpfiles.rules = [
      "D ${cfg.runtime.stateDir} 0750 d2bd d2b - -"
    ];

    # When the Waybar helper is enabled, ensure the binary is on the system
    # path so Waybar's `exec = "d2b-sk-waybar-helper"` works without an
    # absolute path.
    environment.systemPackages = lib.mkIf
      (cfg.integrations.waybar.enable && cfg.statusHelper.package != null)
      [ cfg.statusHelper.package ];
  };
}
