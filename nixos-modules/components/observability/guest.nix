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
      } // lib.optionalAttrs cfg.scrapeJournal {
        # Tail the guest's systemd journal via the contrib journald
        # receiver (which execs `journalctl`). `start_at = "end"` keeps
        # boot-time backlog out of the pipeline; the file_storage cursor
        # below means a collector restart resumes where it left off
        # instead of silently dropping entries written during downtime.
        # The severity_parser maps the journal PRIORITY field onto OTel
        # severity so logs are filterable by level in SigNoz.
        journald = {
          start_at = "end";
          storage = "file_storage/journald";
          operators = [
            {
              type = "severity_parser";
              parse_from = "body.PRIORITY";
              # overwrite_text replaces the raw PRIORITY digit with the
              # canonical OTel severity text (INFO/WARN/ERROR/…) so SigNoz
              # shows a readable level instead of "3"/"4"/"6".
              overwrite_text = true;
              mapping = {
                fatal = [ "0" "1" "2" ];
                error = "3";
                warn = "4";
                info = [ "5" "6" ];
                debug = "7";
              };
            }
          ];
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
        ];
        "resource/self".attributes = [
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
      extensions = lib.optionalAttrs cfg.scrapeJournal {
        # Persist the journald read cursor so a collector restart
        # (OOM/crash → Restart=on-failure) resumes from the last read
        # entry instead of jumping back to the journal tail and dropping
        # everything written during the downtime window.
        "file_storage/journald" = {
          directory = "/var/lib/otel/journald";
          create_directory = true;
        };
      };
      service = {
        extensions = lib.optional cfg.scrapeJournal "file_storage/journald";
        telemetry.metrics.readers = [
          {
            pull.exporter.prometheus = {
              host = "127.0.0.1";
              port = collectorMetricsPort;
            };
          }
        ];
        pipelines.metrics = {
          receivers = [ "otlp" ] ++ lib.optional cfg.scrapeNodeMetrics "hostmetrics";
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
        pipelines."metrics/self" = {
          receivers = [ "prometheus" ];
          processors = [ "memory_limiter" "resource/self" "batch" ];
          exporters = [ "otlp" ];
        };
        pipelines.traces = {
          receivers = [ "otlp" ];
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
        pipelines.logs = {
          receivers = [ "otlp" ] ++ lib.optional cfg.scrapeJournal "journald";
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
      default = true;
      description = "Whether the guest OTel collector follows this VM's systemd journal (journald receiver) and forwards it to SigNoz as logs.";
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
    warnings = [ ];

    microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

    users.users.otel = {
      isSystemUser = true;
      group = "otel";
      home = "/var/lib/otel";
      createHome = false;
      description = "Guest OpenTelemetry collector user";
      # The journald receiver execs `journalctl`, which needs read
      # access to the system journal. Membership in `systemd-journal`
      # grants that without any extra capabilities.
      extraGroups = lib.optional cfg.scrapeJournal "systemd-journal";
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
      # The journald receiver shells out to `journalctl`; expose it (and
      # its systemd runtime deps) on the unit PATH.
      path = lib.optional cfg.scrapeJournal pkgs.systemd;
      serviceConfig = {
        Type = "exec";
        User = "otel";
        Group = "otel";
        # Supplementary group is granted via users.users.otel.extraGroups;
        # SupplementaryGroups here keeps it explicit for the unit even if
        # the static user definition is overridden.
        SupplementaryGroups = lib.optional cfg.scrapeJournal "systemd-journal";
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
