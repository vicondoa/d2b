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

  runtimePrep = pkgs.writeShellScript "nixling-host-otel-runtime-prep" ''
    set -eu
    ${pkgs.coreutils}/bin/install -d -m 0750 -o nixlingd -g nixling ${otelRuntimeDir}
    ${pkgs.acl}/bin/setfacl -m u:nixling-host-otel-collector:--x /run/nixling
    ${pkgs.acl}/bin/setfacl \
      -m u:nixling-host-otel-collector:rwx \
      -m d:u:nixling-host-otel-collector:rwx \
      ${otelRuntimeDir}
    if [ -S ${hostEgressSocket} ]; then
      ${pkgs.acl}/bin/setfacl -m u:nixling-host-otel-collector:rw ${hostEgressSocket}
    fi
  '';

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
        prometheus = {
          config.scrape_configs = [
            {
              job_name = "nixling-host-otel-collector";
              scrape_interval = "30s";
              static_configs = [
                { targets = [ "127.0.0.1:${toString hostCollectorMetricsPort}" ]; }
              ];
            }
          ];
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
          receivers = [ "hostmetrics" "prometheus" ];
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
      ExecStartPre = "+${runtimePrep}";
      ExecStart = "${pkgs.opentelemetry-collector-contrib}/bin/otelcol-contrib --config=file:${collectorConfig}";
      Restart = "on-failure";
      RestartSec = "3s";
      StateDirectory = "nixling-host-otel-collector";
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
