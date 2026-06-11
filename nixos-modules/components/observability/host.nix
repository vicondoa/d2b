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
  storeSyncExportDir = "${config.nixling.site.stateDir}/observability/store-sync";
  storeSyncExportGlob = "${storeSyncExportDir}/store-sync-*.jsonl";

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

    state_dir=${lib.escapeShellArg config.nixling.site.stateDir}
    obs_dir="$state_dir/observability"
    export_dir=${lib.escapeShellArg storeSyncExportDir}
    [ -d "$state_dir" ] || exit 0
    [ -d "$obs_dir" ] || ${pkgs.coreutils}/bin/install -d -m 0700 -o root -g root "$obs_dir"
    [ -d "$export_dir" ] || ${pkgs.coreutils}/bin/install -d -m 0750 -o root -g root "$export_dir"
    ${pkgs.acl}/bin/setfacl -m "u:nixling-host-otel-collector:--x" "$state_dir" 2>/dev/null || true
    ${pkgs.acl}/bin/setfacl -m "u:nixling-host-otel-collector:--x" "$obs_dir" 2>/dev/null || true
    ${pkgs.acl}/bin/setfacl -m "u:nixling-host-otel-collector:r-x" "$export_dir" 2>/dev/null || true
    ${pkgs.acl}/bin/setfacl -d -m "u:nixling-host-otel-collector:r--" "$export_dir" 2>/dev/null || true
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
        "filelog/store_sync_audit" = {
          include = [ storeSyncExportGlob ];
          start_at = "end";
          include_file_name = false;
          include_file_path = false;
          operators = [
            {
              type = "json_parser";
              parse_from = "body";
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
        "resource/store_sync_audit".attributes = [
          { key = "host.name"; value = hostName; action = "upsert"; }
          { key = "vm.name"; value = "host"; action = "upsert"; }
          { key = "vm.env"; value = "host"; action = "upsert"; }
          { key = "vm.role"; value = "host"; action = "upsert"; }
          { key = "service.name"; value = "nixling-store-sync"; action = "upsert"; }
          { key = "source"; value = "store-sync-audit"; action = "upsert"; }
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
        pipelines."logs/store_sync_audit" = {
          receivers = [ "filelog/store_sync_audit" ];
          processors = [ "memory_limiter" "resource/store_sync_audit" "batch" ];
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
