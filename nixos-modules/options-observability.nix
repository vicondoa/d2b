# d2b.observability.* — host-wide observability surface. Split into
# its own file for the v0.2.0 observability track so future PRs can
# extend the feature without reopening the baseline option schema.
{ config, lib, pkgs, ... }:

let
  d2bLib = import ./lib.nix { inherit lib; };
  inherit (d2bLib) subnetIp;
  defaultGrafanaListenAddress =
    subnetIp config.d2b.observability.lanSubnet config.d2b.observability.index;
  defaultRetention = {
    metrics = "30d";
    logs = "14d";
    traces = "7d";
    tracesCritical = "30d";
  };
  defaultSampling = {
    criticalAttribute = "kind";
    criticalValue = "critical";
    criticalRatio = 1.0;
    defaultRatio = 0.1;
    criticalTenant = "d2b-critical";
    defaultTenant = "d2b-default";
  };
  cfg = config.d2b.observability;
in
{
  options.d2b.observability = {
    enable = lib.mkEnableOption ''
      auto-declared observability VM, host forwarders/exporters, and
      per-VM guest telemetry sidecars
    '';

    env = lib.mkOption {
      type = lib.types.str;
      default = "obs";
      description = ''
        Name of the auto-declared observability env. When
        `d2b.observability.enable = true`, the future
        `observability-vm.nix` module materialises
        `d2b.envs.<env>` from this value.
      '';
    };

    vmName = lib.mkOption {
      type = lib.types.str;
      default = "sys-obs";
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
        default = defaultRetention.metrics;
        description = "Compatibility option from the retired stack; native SigNoz retention is configured in SigNoz/ClickHouse and this option currently emits a warning when changed.";
      };

      logs = lib.mkOption {
        type = lib.types.str;
        default = defaultRetention.logs;
        description = "Compatibility option from the retired stack; native SigNoz retention is configured in SigNoz/ClickHouse and this option currently emits a warning when changed.";
      };

      traces = lib.mkOption {
        type = lib.types.str;
        default = defaultRetention.traces;
        description = ''
          Compatibility option from the retired Tempo stack. Native
          SigNoz retention is configured in SigNoz/ClickHouse and this
          option currently emits a warning when changed.
        '';
      };

      tracesCritical = lib.mkOption {
        type = lib.types.str;
        default = defaultRetention.tracesCritical;
        description = ''
          Compatibility option from the retired Tempo critical-tenant
          path. Native SigNoz retention is configured in
          SigNoz/ClickHouse and this option currently emits a warning
          when changed.
        '';
      };
    };

    sampling = {
      criticalAttribute = lib.mkOption {
        type = lib.types.str;
        default = defaultSampling.criticalAttribute;
        description = ''
          Compatibility option from the retired Tempo sampling path.
          Native SigNoz sampling is not configured from this value.
        '';
      };

      criticalValue = lib.mkOption {
        type = lib.types.str;
        default = defaultSampling.criticalValue;
        description = ''
          Compatibility option from the retired Tempo sampling path.
          Native SigNoz sampling is not configured from this value.
        '';
      };

      criticalRatio = lib.mkOption {
        type = lib.types.float;
        default = defaultSampling.criticalRatio;
        description = ''
          Compatibility option from the retired Tempo sampling path.
          Native SigNoz sampling is not configured from this value.
        '';
      };

      defaultRatio = lib.mkOption {
        type = lib.types.float;
        default = defaultSampling.defaultRatio;
        description = ''
          Compatibility option from the retired Tempo sampling path.
          Native SigNoz sampling is not configured from this value.
        '';
      };

      criticalTenant = lib.mkOption {
        type = lib.types.str;
        default = defaultSampling.criticalTenant;
        description = ''
          Compatibility option from the retired Tempo tenant path.
          Native SigNoz sampling is not configured from this value.
        '';
      };

      defaultTenant = lib.mkOption {
        type = lib.types.str;
        default = defaultSampling.defaultTenant;
        description = ''
          Compatibility option from the retired Tempo tenant path.
          Native SigNoz sampling is not configured from this value.
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
          Retired Grafana stack compatibility option. It no longer
          affects native SigNoz authentication. Use
          `d2b.observability.signoz.jwtSecretFile` and
          `d2b.observability.signoz.rootPasswordFile` for native
          SigNoz credentials.
        '';
      };

      adminPasswordFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Retired Grafana stack compatibility option. It no longer
          affects native SigNoz authentication. Use
          `d2b.observability.signoz.rootPasswordFile` for the
          native SigNoz root password.
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

    signoz = {
      listenAddress = lib.mkOption {
        type = lib.types.str;
        default = defaultGrafanaListenAddress;
        description = ''
          Address SigNoz binds inside the observability env. Default
          tracks the observability VM's derived IP (`lanSubnet` +
          `index`).
        '';
      };

      listenPort = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = ''
          TCP port SigNoz listens on inside the observability env.
        '';
      };

      otlpGrpcPort = lib.mkOption {
        type = lib.types.port;
        default = 4317;
        description = ''
          Loopback port used by the SigNoz OTel Collector for local OTLP
          gRPC ingress inside the observability VM.
        '';
      };

      otlpHttpPort = lib.mkOption {
        type = lib.types.port;
        default = 4318;
        description = ''
          Loopback port used by the SigNoz OTel Collector for local OTLP
          HTTP ingress inside the observability VM.
        '';
      };

      adminEmail = lib.mkOption {
        type = lib.types.str;
        default = "admin@d2b.local";
        description = ''
          Root SigNoz admin email used for first-run bootstrap.
        '';
      };

      jwtSecretFile = lib.mkOption {
        type = lib.types.nullOr (lib.types.either lib.types.path lib.types.str);
        default = null;
        description = ''
          Optional host path containing SigNoz's JWT/tokenizer secret.
          When null, d2b generates
          `${"$"}{d2b.site.stateDir}/observability/signoz-jwt-secret`
          at activation. When set, activation copies this file into that
          host-secret path with `0444 root:root` under a `0700`
          root-owned directory before sharing it read-only into `sys-obs`.
        '';
      };

      rootPasswordFile = lib.mkOption {
        type = lib.types.nullOr (lib.types.either lib.types.path lib.types.str);
        default = null;
        description = ''
          Optional host path containing the SigNoz root user's password.
          When null, d2b generates
          `${"$"}{d2b.site.stateDir}/observability/signoz-root-password`
          at activation. When set, activation copies this file into that
          host-secret path with `0444 root:root` under a `0700`
          root-owned directory before sharing it read-only into `sys-obs`.
        '';
      };

      clickhousePasswordFile = lib.mkOption {
        type = lib.types.nullOr (lib.types.either lib.types.path lib.types.str);
        default = null;
        description = ''
          Optional host path containing the ClickHouse password used by
          SigNoz services. When null, d2b generates
          `${"$"}{d2b.site.stateDir}/observability/clickhouse-password`
          at activation. When set, activation copies this file into that
          host-secret path with `0444 root:root` under a `0700`
          root-owned directory before sharing it read-only into `sys-obs`.
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
        Include OpenTelemetry trace helpers in the `d2b` CLI.
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
        dedicated `d2b-otel-relay` implementation.

        Current contract: this package MUST provide a
        `bin/socat`-compatible CLI.
      '';
    };

    # Host edge-collector parity surface (ADR 0033). The host OTel
    # collector already ships hostmetrics + the StoreSync audit log; these
    # options bring it to parity with the per-VM guest collector and give
    # host-origin telemetry the real hostname identity.
    host = {
      identityName = lib.mkOption {
        type = lib.types.str;
        default = config.networking.hostName;
        defaultText = lib.literalExpression "config.networking.hostName";
        description = ''
          Resource-attribute identity stamped on host-origin telemetry
          (`vm.name` and `host.name`) at the trusted per-source ingress
          boundary. Defaults to the host's `networking.hostName`. Set to
          `"host"` to keep the pre-0.2.x literal label. `vm.role` stays
          `"host"` regardless, so host-origin telemetry is still
          selectable as a class independent of the machine name.
        '';
      };

      scrapeJournal = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Tail the host's systemd journal via the contrib `journald`
          receiver and forward it to SigNoz as logs over the existing
          host -> `sys-obs` vsock bridge (never a LAN). Default off: the
          host journal can contain auth failures, sudo command lines, and
          service-logged secrets, and it is forwarded non-redacted (only a
          PRIORITY->severity parser runs), at parity with the guest
          journal. Enable only when the `sys-obs` VM is a trusted operator
          sink. Retention of these logs is governed by SigNoz/ClickHouse
          TTL inside `sys-obs`, not `d2b.observability.retention.*`.
        '';
      };

      otlpIngest = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = false;
          description = ''
            Expose a host-local OpenTelemetry ingest endpoint so host-side
            instrumentation can push traces/logs/metrics through the same
            host -> `sys-obs` bridge as host metrics. The endpoint is a
            Unix-domain socket only (no TCP listener), isolated in its own
            directory at `/run/d2b/otel/ingest/host-otlp.sock` so the
            collector cannot reach `host-egress.sock`.
          '';
        };

        clientGroup = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = ''
            Optional group granted write access to the host OTLP ingest
            socket. When null (default) the socket is `0600`
            (collector + root only). When set, the socket is `0660`
            group-owned by this group (with `--x` traversal on the parent
            dirs) so members can emit telemetry. Only the dedicated
            `ingest/` subdirectory and socket are affected;
            `host-egress.sock` is never widened.
          '';
        };
      };
    };

    # Internal: the host OTel collector's pre-serialization config attrset,
    # surfaced so eval-based tests can assert receiver/pipeline/extension
    # shape without parsing the generated YAML. Not a stable API.
    _internal.hostCollectorConfig = lib.mkOption {
      type = lib.types.attrs;
      internal = true;
      default = { };
      description = "Internal host OTel collector config attrset (test surface).";
    };
  };

  config.assertions = [
    {
      assertion = (!cfg.host.scrapeJournal && !cfg.host.otlpIngest.enable) || cfg.enable;
      message = ''
        d2b.observability.host.scrapeJournal and
        d2b.observability.host.otlpIngest.enable require
        d2b.observability.enable = true: the host OTel collector only
        exists when the observability stack is enabled. Enable the stack or
        unset the host flags.
      '';
    }
  ];

  config.warnings = lib.mkIf cfg.enable (
    lib.optional (cfg.retention != defaultRetention) ''
      d2b.observability.retention.* is a compatibility surface for
      the retired Tempo/Loki stack. Native SigNoz/ClickHouse retention is
      not configured from these options yet; use SigNoz/ClickHouse
      retention controls and size `sys-obs` volumes explicitly.
    ''
    ++ lib.optional (cfg.sampling != defaultSampling) ''
      d2b.observability.sampling.* is a compatibility surface for
      the retired Tempo stack. Native SigNoz sampling is not configured
      from these options yet.
    ''
    ++ lib.optional (cfg.grafana.secretKeyFile != null || cfg.grafana.adminPasswordFile != null) ''
      d2b.observability.grafana.{secretKeyFile,adminPasswordFile}
      are retired Grafana-stack compatibility options and do not affect
      native SigNoz authentication. Use
      d2b.observability.signoz.{jwtSecretFile,rootPasswordFile}
      instead.
    ''
  );
}
