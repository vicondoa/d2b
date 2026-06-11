# Per-workload observability guest component.
#
# Runs an OpenTelemetry Collector edge agent inside each opted-in guest
# and forwards OTLP over the existing guest→host vsock relay.
{ lib, pkgs, config, ... }:

let
  cfg = config.nixling.observability;
  collectorMetricsPort = 12345;
  otelRuntimeDir = "/run/nixling/otel";
  guestOtlpSocket = "${otelRuntimeDir}/otlp.sock";
  guestOtlpEgressSocket = "${otelRuntimeDir}/otlp-egress.sock";
  auditEnabled = config.nixling.audit.enable or false;

  collectorConfig = pkgs.writeText "nixling-guest-otel-collector.yaml" (
    lib.generators.toYAML { } {
      receivers = {
        otlp.protocols.grpc = {
          endpoint = guestOtlpSocket;
          transport = "unix";
        };
        prometheus = {
          config.scrape_configs = [
            {
              job_name = "nixling-guest-otel-collector";
              scrape_interval = "30s";
              static_configs = [
                { targets = [ "127.0.0.1:${toString collectorMetricsPort}" ]; }
              ];
            }
          ];
        };
      } // lib.optionalAttrs cfg.scrapeNodeMetrics {
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
          limit_mib = 128;
          spike_limit_mib = 32;
        };
        resource.attributes = [
          { key = "vm.name"; value = cfg.identity.vmName; action = "upsert"; }
          { key = "vm.env"; value = cfg.identity.envName; action = "upsert"; }
          { key = "vm.role"; value = "workload"; action = "upsert"; }
          { key = "service.name"; value = "nixling-guest-otel-collector"; action = "upsert"; }
        ];
        batch = {
          send_batch_size = 2048;
          timeout = "1s";
        };
      };
      exporters.otlp = {
        endpoint = "unix://${guestOtlpEgressSocket}";
        compression = "none";
        tls.insecure = true;
        sending_queue.enabled = true;
        retry_on_failure.enabled = true;
      };
      service = {
        telemetry.metrics.address = "127.0.0.1:${toString collectorMetricsPort}";
        pipelines.metrics = {
          receivers = [ "otlp" "prometheus" ] ++ lib.optional cfg.scrapeNodeMetrics "hostmetrics";
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
        pipelines.traces = {
          receivers = [ "otlp" ];
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
        pipelines.logs = {
          receivers = [ "otlp" ];
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
      };
    }
  );
in
{
  options.nixling.observability = {
    scrapeJournal = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Compatibility toggle reserved for future journald collection; native OTel guest logs are not yet scraped from journald.";
    };

    scrapeNodeMetrics = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Whether the guest OTel collector scrapes this VM's hostmetrics receiver.";
    };

    identity = {
      vmName = lib.mkOption {
        type = lib.types.str;
        default = config.networking.hostName;
        description = "Internal VM resource attribute injected into guest telemetry.";
      };

      envName = lib.mkOption {
        type = lib.types.str;
        default = "none";
        description = "Internal env resource attribute injected into guest telemetry.";
      };
    };

    transport.relayPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.socat;
      description = "Package providing the guest-side observability relay binary.";
    };
  };

  config = {
    warnings = lib.optional cfg.scrapeJournal ''
      nixling.vms.<vm>.observability.scrapeJournal is currently a
      compatibility no-op in the native SigNoz path; journald/audit log
      ingestion is not wired yet.
    '';

    microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

    users.users.otel = {
      isSystemUser = true;
      group = "otel";
      home = "/var/lib/otel";
      createHome = false;
      description = "Guest OpenTelemetry collector user";
    };
    users.groups.otel = { };

    systemd.tmpfiles.rules = [
      "d ${otelRuntimeDir} 0710 otel otel -"
      "L+ /run/nixling/otlp.sock - - - - ${guestOtlpSocket}"
      "L+ /run/nixling/otlp-egress.sock - - - - ${guestOtlpEgressSocket}"
    ];

    systemd.services.nixling-otel-collector = {
      description = "nixling guest OpenTelemetry collector";
      wantedBy = [ "multi-user.target" ];
      restartIfChanged = false;
      serviceConfig = {
        Type = "exec";
        User = "otel";
        Group = "otel";
        ExecStart = "${pkgs.opentelemetry-collector-contrib}/bin/otelcol-contrib --config=file:${collectorConfig}";
        Restart = "on-failure";
        RestartSec = "3s";
        StateDirectory = "otel";
        RuntimeDirectory = "nixling/otel";
        RuntimeDirectoryMode = "0710";
        RuntimeDirectoryPreserve = "yes";
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

    systemd.services.nixling-otel-vsock-out = {
      description = "Bridge guest OTLP UDS to host vsock port 14317.";
      wantedBy = [ "multi-user.target" ];
      after = [ "nixling-otel-collector.service" ];
      wants = [ "nixling-otel-collector.service" ];
      restartIfChanged = false;

      serviceConfig = {
        User = "otel";
        Group = "otel";
        ExecStartPre = [
          "+${pkgs.coreutils}/bin/rm -f ${guestOtlpEgressSocket}"
        ];
        ExecStart = "${cfg.transport.relayPackage}/bin/socat -d -d UNIX-LISTEN:${guestOtlpEgressSocket},fork,max-children=16,reuseaddr,mode=0660 VSOCK-CONNECT:2:14317";
        Restart = "on-failure";
        RestartSec = "3s";
        StartLimitIntervalSec = "300s";
        StartLimitBurst = 20;
        DynamicUser = false;
        TasksMax = 32;
        MemoryMax = "64M";
        LimitNOFILE = 1024;
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        PrivateTmp = true;
        PrivateDevices = false;
        DeviceAllow = [ "/dev/vsock rw" ];
        RestrictAddressFamilies = [ "AF_UNIX" "AF_VSOCK" ];
        SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
        UMask = "0077";
      };
    };
  };
}
