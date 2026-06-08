# nixling.observability.* — host-wide observability surface. Split into
# its own file for the v0.2.0 observability track so follow-up PRs can
# extend the feature without reopening the baseline option schema.
{ config, lib, pkgs, ... }:

let
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) subnetIp;
  defaultGrafanaListenAddress =
    subnetIp config.nixling.observability.lanSubnet config.nixling.observability.index;
in
{
  options.nixling.observability = {
    enable = lib.mkEnableOption ''
      auto-declared observability VM, host forwarders/exporters, and
      per-VM guest telemetry sidecars
    '';

    env = lib.mkOption {
      type = lib.types.str;
      default = "obs";
      description = ''
        Name of the auto-declared observability env. When
        `nixling.observability.enable = true`, the future
        `observability-vm.nix` module materialises
        `nixling.envs.<env>` from this value.
      '';
    };

    vmName = lib.mkOption {
      type = lib.types.str;
      default = "sys-obs-stack";
      description = ''
        VM name of the auto-declared observability stack VM.
      '';
    };

    index = lib.mkOption {
      type = lib.types.int;
      default = 10;
      description = ''
        Workload-style LAN index reserved for the observability stack
        VM inside `lanSubnet`.
      '';
    };

    lanSubnet = lib.mkOption {
      type = lib.types.str;
      default = "10.40.0.0/24";
      description = ''
        LAN CIDR for the auto-declared observability env.
      '';
    };

    uplinkSubnet = lib.mkOption {
      type = lib.types.str;
      default = "203.0.113.0/30";
      description = ''
        Host↔observability-stack point-to-point CIDR for the auto-
        declared observability env.
      '';
    };

    retention = {
      metrics = lib.mkOption {
        type = lib.types.str;
        default = "30d";
        description = "Retention window for metrics in the observability stack.";
      };

      logs = lib.mkOption {
        type = lib.types.str;
        default = "14d";
        description = "Retention window for logs in the observability stack.";
      };

      traces = lib.mkOption {
        type = lib.types.str;
        default = "7d";
        description = ''
          Default retention window for traces (default Tempo
          tenant). Mirror of `nixling.observability.retention.traces`
          on the stack VM. P5 `ph5-p5-tempo-budget`.
        '';
      };

      tracesCritical = lib.mkOption {
        type = lib.types.str;
        default = "30d";
        description = ''
          Retention window for the critical Tempo tenant (spans
          tagged `kind=critical`). Mirror of
          `nixling.observability.retention.tracesCritical` on the
          stack VM. P5 `ph5-p5-tempo-budget`.
        '';
      };
    };

    sampling = {
      criticalAttribute = lib.mkOption {
        type = lib.types.str;
        default = "kind";
        description = ''
          Span attribute key inspected to decide whether a span
          belongs to the critical Tempo tenant.
        '';
      };

      criticalValue = lib.mkOption {
        type = lib.types.str;
        default = "critical";
        description = ''
          Span attribute value that pins a span into the critical
          Tempo tenant.
        '';
      };

      criticalRatio = lib.mkOption {
        type = lib.types.float;
        default = 1.0;
        description = ''
          Sampling ratio for critical spans (0.0–1.0). Pinned to
          1.0 by the canonical policy.
        '';
      };

      defaultRatio = lib.mkOption {
        type = lib.types.float;
        default = 0.1;
        description = ''
          Head-consistent sampling ratio for non-critical spans
          (0.0–1.0). Pinned to 0.1 by the canonical policy.
        '';
      };

      criticalTenant = lib.mkOption {
        type = lib.types.str;
        default = "nixling-critical";
        description = ''
          Tempo tenant id (`X-Scope-OrgID`) that critical spans
          are routed to.
        '';
      };

      defaultTenant = lib.mkOption {
        type = lib.types.str;
        default = "nixling-default";
        description = ''
          Tempo tenant id (`X-Scope-OrgID`) that non-critical
          spans are routed to.
        '';
      };
    };

    alerts = lib.mkOption {
      type = lib.types.attrsOf (lib.types.submodule {
        options.enable = lib.mkEnableOption "this alert rule" // { default = true; };
      });
      default = { };
      description = ''
        Per-alert toggles. The eight default alerts can be individually
        disabled by setting `<name>.enable = false`.
      '';
    };

    grafana = {
      listenAddress = lib.mkOption {
        type = lib.types.str;
        default = defaultGrafanaListenAddress;
        description = ''
          Address Grafana binds inside the observability env. Default
          tracks the observability VM's derived IP (`lanSubnet` +
          `index`).
        '';
      };

      listenPort = lib.mkOption {
        type = lib.types.port;
        default = 3000;
        description = ''
          TCP port Grafana listens on inside the observability env.
        '';
      };

      secretKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Optional path to a file containing the Grafana session/signing
          secret key. When null (the default), the framework generates a
          per-install secret on the **host** at
          `${"$"}{nixling.site.stateDir}/observability/grafana-secret-key`
          (mode 0400 root:root) and shares it read-only into
          `sys-obs-stack` at
          `/run/nixling-obs-secrets/grafana-secret-key`. When set,
          this path is loaded via systemd LoadCredential instead, and
          the framework's host-side generator leaves the secret alone.
          Use this option to source the secret from sops-nix, agenix,
          or any other declarative secrets framework.
        '';
      };

      adminPasswordFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Optional path to a file containing the Grafana admin password.
          When null (the default), the framework generates a per-install
          password on the **host** at
          `${"$"}{nixling.site.stateDir}/observability/grafana-admin-password`
          (default `/var/lib/nixling/observability/grafana-admin-password`,
          mode 0400 root:root) and shares it read-only into
          `sys-obs-stack` at
          `/run/nixling-obs-secrets/grafana-admin-password`. Host
          operators read it directly via `sudo cat <path>` — no
          cross-VM SSH required. When set, this path is loaded via
          systemd LoadCredential instead, and the framework's
          host-side generator leaves the secret alone. Use this
          option to source the admin password from sops-nix, agenix,
          or any other declarative secrets framework.
        '';
      };

      anonymousViewer.enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Re-enable Grafana's anonymous Viewer mode for trusted
          single-host LAN deployments. Disabled by default so Grafana
          requires an authenticated login.
        '';
      };
    };

    ch.exporter = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = ''
          Enable the host-side Cloud Hypervisor metrics exporter.
        '';
      };

      listenPort = lib.mkOption {
        type = lib.types.port;
        default = 9101;
        description = ''
          Loopback port the host-side Cloud Hypervisor metrics exporter
          listens on.
        '';
      };

      includeTopologyLabels = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Re-enable the exporter labels that expose bridge/tap and
          TPM/graphics/audio/YubiKey topology details. Disabled by
          default so retained metrics do not keep those host topology
          labels unless an operator explicitly opts in for debugging.
        '';
      };
    };

    cli.traces.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Include OpenTelemetry trace helpers in the `nixling` CLI.
      '';
    };

    # v0.2.0 note: the implementation still shells out to
    # `${relayPackage}/bin/socat` with socat-specific arguments, so any
    # override MUST provide a `bin/socat`-compatible CLI.
    transport.relayPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.socat;
      description = ''
        Package providing the observability byte-relay binary. Defaults
        to `pkgs.socat` today and stays swappable for a future
        dedicated `nixling-otel-relay` implementation.

        Current contract: this package MUST provide a
        `bin/socat`-compatible CLI.
      '';
    };
  };
}
