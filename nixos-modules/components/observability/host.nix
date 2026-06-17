# Host-side observability edge collector.
#
# Collects host telemetry with the upstream OpenTelemetry Collector
# Contrib binary and exports one OTLP stream into the broker-spawned
# host-to-obs bridge.
#
# Parity surface (ADR 0033): in addition to hostmetrics + the StoreSync
# audit log, the collector can (opt-in) tail the host journal and accept
# OTLP from host-local instrumentation, mirroring the per-VM guest
# collector. Host-origin identity (`vm.name`/`host.name`) is the host's
# `nixling.observability.host.identityName`; it is stamped here advisorily
# and re-stamped authoritatively at the trusted ingress boundary.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling.observability;
  hostCfg = cfg.host;
  identityName = hostCfg.identityName;
  scrapeJournal = hostCfg.scrapeJournal;
  otlpIngest = hostCfg.otlpIngest.enable;
  clientGroup = hostCfg.otlpIngest.clientGroup;

  otelRuntimeDir = "/run/nixling/otel";
  hostEgressSocket = "${otelRuntimeDir}/host-egress.sock";
  # OTLP ingest lives in its own subdirectory so the collector's write
  # authority for bind(2) cannot reach host-egress.sock — unlink/rename
  # authority is parent-directory scoped (ADR 0033).
  otelIngestDir = "${otelRuntimeDir}/ingest";
  hostOtlpSocket = "${otelIngestDir}/host-otlp.sock";

  hostCollectorMetricsPort = 12345;
  journaldStorageDir = "/var/lib/nixling-host-otel-collector/journald";

  storeSyncExportDir = "${config.nixling.site.stateDir}/observability/store-sync";
  storeSyncExportGlob = "${storeSyncExportDir}/store-sync-*.jsonl";

  ingestGroup = if clientGroup == null then "nixling-host-otel-collector" else clientGroup;
  ingestDirMode = if clientGroup == null then "0700" else "2750";
  ingestUmask = if clientGroup == null then "0177" else "0117";

  # Identity assigned at the edge is advisory: the central collector
  # re-stamps it at the trusted ingress boundary (ADR 0026/0033). vm.role
  # stays "host" so host-origin telemetry is selectable as a class.
  identityAttrs = [
    { key = "vm.name"; value = identityName; action = "upsert"; }
    { key = "vm.env"; value = "host"; action = "upsert"; }
    { key = "vm.role"; value = "host"; action = "upsert"; }
    { key = "host.name"; value = identityName; action = "upsert"; }
  ];

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
    if [ -d "$state_dir" ]; then
      [ -d "$obs_dir" ] || ${pkgs.coreutils}/bin/install -d -m 0700 -o root -g root "$obs_dir"
      [ -d "$export_dir" ] || ${pkgs.coreutils}/bin/install -d -m 0750 -o root -g root "$export_dir"
      ${pkgs.acl}/bin/setfacl -m "u:nixling-host-otel-collector:--x" "$state_dir" 2>/dev/null || true
      ${pkgs.acl}/bin/setfacl -m "u:nixling-host-otel-collector:--x" "$obs_dir" 2>/dev/null || true
      ${pkgs.acl}/bin/setfacl -m "u:nixling-host-otel-collector:r-x" "$export_dir" 2>/dev/null || true
      ${pkgs.acl}/bin/setfacl -d -m "u:nixling-host-otel-collector:r--" "$export_dir" 2>/dev/null || true
    fi
    ${lib.optionalString scrapeJournal ''

      # Pre-create the journald file_storage cursor dir with explicit perms.
      # UMask (set process-wide for the OTLP ingest socket) would otherwise
      # apply to the collector's own create_directory mkdir, stripping the
      # owner execute bit (0750 & ~0177 = 0640) and making the dir unusable.
      # Provisioning it here (privileged ExecStartPre, under StateDirectory)
      # lets the extension run with create_directory = false.
      ${pkgs.coreutils}/bin/install -d -m 0700 -o nixling-host-otel-collector -g nixling-host-otel-collector ${journaldStorageDir}
    ''}
    ${lib.optionalString otlpIngest ''

      # Dedicated, isolated OTLP ingest directory. The shared
      # ${otelRuntimeDir} stays read-only in the collector namespace
      # (no ReadWritePaths there), so host-egress.sock cannot be
      # unlinked/replaced. setfacl -b then chmod give deterministic perms,
      # not whatever the parent default ACL would mask in.
      ${pkgs.coreutils}/bin/install -d -o nixling-host-otel-collector -g ${ingestGroup} ${otelIngestDir}
      ${pkgs.acl}/bin/setfacl -b ${otelIngestDir}
      ${pkgs.coreutils}/bin/chmod ${ingestDirMode} ${otelIngestDir}
      ${lib.optionalString (clientGroup != null) ''
        ${pkgs.acl}/bin/setfacl -m g:${clientGroup}:--x /run/nixling ${otelRuntimeDir}
      ''}
      # Remove a stale pathname socket so AF_UNIX bind(2) succeeds after an
      # unclean exit (Restart=on-failure).
      ${pkgs.coreutils}/bin/rm -f ${hostOtlpSocket}
    ''}
  '';

  collectorAttrs = {
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
    } // lib.optionalAttrs otlpIngest {
      # Host-local OTLP ingest over a Unix socket only (no TCP listener),
      # isolated in ${otelIngestDir}.
      otlp.protocols.grpc = {
        endpoint = hostOtlpSocket;
        transport = "unix";
      };
    } // lib.optionalAttrs scrapeJournal {
      # Tail the host's systemd journal via the contrib journald receiver
      # (execs `journalctl`). start_at=end keeps boot backlog out; the
      # file_storage cursor resumes after a restart; severity_parser maps
      # the journal PRIORITY field onto readable OTel severities. Mirrors
      # the guest collector; forwarded only over the host->sys-obs vsock
      # bridge, non-redacted (ADR 0033 sensitivity note).
      journald = {
        start_at = "end";
        storage = "file_storage/journald";
        operators = [
          {
            type = "severity_parser";
            parse_from = "body.PRIORITY";
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
        limit_mib = 256;
        spike_limit_mib = 64;
      };
      # Identity-only: NO service.name, so ingested app/journal telemetry
      # keeps its own service.name instead of being relabelled as the
      # collector (ADR 0033).
      resource.attributes = identityAttrs;
      # The collector's own self-metrics carry the collector service.name.
      "resource/self".attributes = identityAttrs ++ [
        { key = "service.name"; value = "nixling-host-otel-collector"; action = "upsert"; }
      ];
      "resource/store_sync_audit".attributes = identityAttrs ++ [
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
      telemetry.metrics.readers = [
        {
          pull.exporter.prometheus = {
            host = "127.0.0.1";
            port = hostCollectorMetricsPort;
          };
        }
      ];
      pipelines = {
        metrics = {
          receivers = [ "hostmetrics" ] ++ lib.optional otlpIngest "otlp";
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
        "metrics/self" = {
          receivers = [ "prometheus" ];
          processors = [ "memory_limiter" "resource/self" "batch" ];
          exporters = [ "otlp" ];
        };
        "logs/store_sync_audit" = {
          receivers = [ "filelog/store_sync_audit" ];
          processors = [ "memory_limiter" "resource/store_sync_audit" "batch" ];
          exporters = [ "otlp" ];
        };
      } // lib.optionalAttrs otlpIngest {
        traces = {
          receivers = [ "otlp" ];
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
      } // lib.optionalAttrs (otlpIngest || scrapeJournal) {
        logs = {
          receivers = lib.optional otlpIngest "otlp" ++ lib.optional scrapeJournal "journald";
          processors = [ "memory_limiter" "resource" "batch" ];
          exporters = [ "otlp" ];
        };
      };
    } // lib.optionalAttrs scrapeJournal {
      extensions = [ "file_storage/journald" ];
    };
  } // lib.optionalAttrs scrapeJournal {
    extensions."file_storage/journald" = {
      directory = journaldStorageDir;
      # Provisioned by runtimePrep (ExecStartPre) with explicit 0700 perms;
      # not created here so the process-wide UMask (set for the OTLP ingest
      # socket) cannot strip the directory's owner execute bit.
      create_directory = false;
    };
  };

  collectorConfig = pkgs.writeText "nixling-host-otel-collector.yaml" (
    lib.generators.toYAML { } collectorAttrs
  );
in
lib.mkIf cfg.enable {
  users.users."nixling-host-otel-collector" = {
    isSystemUser = true;
    group = "nixling-host-otel-collector";
    home = "/var/lib/nixling-host-otel-collector";
    createHome = false;
    description = "nixling host OpenTelemetry collector";
  };
  users.groups."nixling-host-otel-collector" = { };

  # Internal test surface: the pre-serialization collector attrset
  # (ADR 0033). Lets eval-cases assert receiver/pipeline/extension shape
  # without parsing the generated YAML.
  nixling.observability._internal.hostCollectorConfig = collectorAttrs;

  systemd.tmpfiles.rules = [
    "d ${otelRuntimeDir} 0750 nixlingd nixling -"
    "L+ /run/nixling/host-egress.sock - - - - ${hostEgressSocket}"
  ] ++ lib.optional otlpIngest
    # The OTLP ingest subdir MUST exist before the unit's mount namespace
    # is constructed: systemd builds the ReadWritePaths bind mount for it
    # at start, and a missing path fails the unit at the NAMESPACE step
    # (226/NAMESPACE) before any ExecStartPre runs. tmpfiles creates it
    # ahead of the unit; runtimePrep then refines perms/ACLs.
    "d ${otelIngestDir} ${ingestDirMode} nixling-host-otel-collector ${ingestGroup} -";

  systemd.services.nixling-host-otel-collector = {
    description = "nixling host OpenTelemetry collector";
    wantedBy = [ "multi-user.target" ];
    after = [ "nixlingd.service" ];
    # journald receiver shells out to `journalctl`.
    path = lib.optional scrapeJournal pkgs.systemd;
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
    }
    // lib.optionalAttrs scrapeJournal {
      SupplementaryGroups = [ "systemd-journal" ];
    }
    // lib.optionalAttrs otlpIngest {
      # Only the dedicated ingest subdir is writable for bind(2); the
      # shared ${otelRuntimeDir} (with host-egress.sock) stays read-only.
      ReadWritePaths = [ otelIngestDir ];
      # Deterministic socket mode: 0600 (collector+root) by default, or
      # 0660 group-clientGroup via the setgid ingest dir when clientGroup
      # is set.
      UMask = ingestUmask;
    };
  };
}
