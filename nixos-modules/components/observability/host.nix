# Host-side observability edge collector.
#
# Collects host telemetry with the upstream OpenTelemetry Collector
# Contrib binary and exports one OTLP stream into the broker-spawned
# host-to-obs bridge.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling.observability;
  hostName = config.networking.hostName;
  otelRuntimeDir = "/run/nixling/otel";
  hostEgressSocket = "${otelRuntimeDir}/host-egress.sock";
  hostCollectorMetricsPort = 12345;

  collectorConfig = pkgs.writeText "nixling-host-otel-collector.yaml" (
    lib.generators.toYAML { } {
      receivers = {
        hostmetrics = {
          collection_interval = "30s";
          scrapers = {
            cpu = { };
            disk = { };
            filesystem = { };
            load = { };
            memory = { };
            network = { };
            paging = { };
            processes = { };
          };
        };
      };
      processors = {
        memory_limiter = {
          check_interval = "1s";
          limit_mib = 256;
          spike_limit_mib = 64;
        };
        resource.attributes = [
          { key = "host.name"; value = hostName; action = "upsert"; }
          { key = "vm.name"; value = "host"; action = "upsert"; }
          { key = "vm.env"; value = "host"; action = "upsert"; }
          { key = "vm.role"; value = "host"; action = "upsert"; }
          { key = "service.name"; value = "nixling-host-otel-collector"; action = "upsert"; }
        ];
        batch = {
          send_batch_size = 4096;
          timeout = "1s";
        };
      };
      exporters.otlp = {
        endpoint = "unix://${hostEgressSocket}";
        compression = "none";
        tls.insecure = true;
        sending_queue.enabled = true;
        retry_on_failure.enabled = true;
      };
      service = {
        telemetry.metrics.address = "127.0.0.1:${toString hostCollectorMetricsPort}";
        pipelines.metrics = {
          receivers = [ "hostmetrics" ];
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
      };
    }
  );
in
lib.mkIf cfg.enable {
  users.users.nixling-host-otel-collector = {
    isSystemUser = true;
    group = "nixling-host-otel-collector";
    home = "/var/lib/nixling-host-otel-collector";
    createHome = false;
    description = "nixling host OpenTelemetry collector";
  };
  users.groups.nixling-host-otel-collector = { };

  systemd.tmpfiles.rules = [
    "d ${otelRuntimeDir} 0750 nixlingd nixling -"
    "L+ /run/nixling/host-egress.sock - - - - ${hostEgressSocket}"
  ];

  systemd.services.nixling-host-otel-collector = {
    description = "nixling host OpenTelemetry collector";
    wantedBy = [ "multi-user.target" ];
    after = [ "nixlingd.service" ];
    serviceConfig = {
      Type = "exec";
      User = "nixling-host-otel-collector";
      Group = "nixling-host-otel-collector";
      ExecStart = "${pkgs.opentelemetry-collector-contrib}/bin/otelcol-contrib --config=file:${collectorConfig}";
      Restart = "on-failure";
      RestartSec = "3s";
      StateDirectory = "nixling-host-otel-collector";
      RuntimeDirectory = "nixling/otel";
      RuntimeDirectoryMode = "0750";
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      ProtectKernelTunables = true;
      ProtectKernelModules = true;
      ProtectControlGroups = true;
      PrivateTmp = true;
      PrivateDevices = true;
      RestrictAddressFamilies = [ "AF_UNIX" "AF_INET" "AF_INET6" ];
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
    };
  };
}
