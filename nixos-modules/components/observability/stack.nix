# Observability stack guest component for the auto-declared `sys-obs` VM.
#
# Native SigNoz backend: ClickHouse + ZooKeeper + SigNoz + SigNoz OTel
# Collector. No container runtime is used.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling.observability;

  signoz = import ../../../pkgs/signoz { inherit pkgs; };
  signozOtelCollector = import ../../../pkgs/signoz-otel-collector { inherit pkgs; };
  signozSchemaMigrator = import ../../../pkgs/signoz-schema-migrator { inherit pkgs; };

  hostSecretsGuestDir = "/run/nixling-obs-secrets";
  clickhousePort = 9000;
  clickhouseHttpPort = 8123;
  clickhouseInterserverPort = 9009;
  zookeeperPort = 2181;
  signozPort = cfg.signoz.listenPort;
  otlpGrpcPort = cfg.signoz.otlpGrpcPort;
  otlpHttpPort = cfg.signoz.otlpHttpPort;
  collectorHealthPort = 13133;

  clickhousePasswordCredential = "clickhouse-password";
  signozJwtCredential = "signoz-jwt-secret";
  signozRootPasswordCredential = "signoz-root-password";

  clickhouseDsn = "tcp://127.0.0.1:${toString clickhousePort}";
  clickhouseDsnEnv = "tcp://127.0.0.1:${toString clickhousePort}?username=signoz&password=$SIGNOZ_CLICKHOUSE_PASSWORD";

  signozConfig = pkgs.writeText "nixling-signoz.yaml" (
    lib.generators.toYAML { } {
      global = {
        external_url = "http://${cfg.signoz.listenAddress}:${toString signozPort}";
        ingestion_url = "http://127.0.0.1:${toString otlpHttpPort}";
      };
      analytics.enabled = false;
      instrumentation = {
        logs.level = "info";
        traces.enabled = false;
        metrics = {
          enabled = true;
          readers.pull.exporter.prometheus = {
            host = "127.0.0.1";
            port = 9090;
          };
        };
      };
      pprof = {
        enabled = false;
        address = "127.0.0.1:6060";
      };
      web = {
        enabled = true;
        index = "index.html";
        directory = "${signoz}/web";
        settings = {
          posthog.enabled = false;
          appcues.enabled = false;
          sentry.enabled = false;
          pylon.enabled = false;
        };
      };
      sqlstore = {
        provider = "sqlite";
        max_open_conns = 100;
        max_conn_lifetime = "0s";
        sqlite = {
          path = "/var/lib/signoz/signoz.db";
          mode = "wal";
          busy_timeout = "10s";
          transaction_mode = "deferred";
        };
      };
      telemetrystore = {
        provider = "clickhouse";
        max_open_conns = 100;
        max_idle_conns = 50;
        dial_timeout = "5s";
        clickhouse = {
          dsn = clickhouseDsn;
          cluster = "cluster";
        };
      };
      alertmanager.signoz.external_url = "http://${cfg.signoz.listenAddress}:${toString signozPort}";
      tokenizer = {
        provider = "jwt";
        jwt.secret = "";
      };
      user.root = {
        enabled = true;
        email = cfg.signoz.adminEmail;
        password = "";
        org.name = "default";
      };
    }
  );

  collectorConfig = pkgs.writeText "nixling-signoz-otel-collector.yaml" (
    lib.generators.toYAML { } {
      receivers = {
        otlp = {
          protocols = {
            grpc.endpoint = "127.0.0.1:${toString otlpGrpcPort}";
            http.endpoint = "127.0.0.1:${toString otlpHttpPort}";
          };
        };
      };
      processors = {
        memory_limiter = {
          check_interval = "1s";
          limit_mib = 512;
          spike_limit_mib = 128;
        };
        batch = {
          send_batch_size = 8192;
          timeout = "1s";
        };
        "signozspanmetrics/delta" = {
          metrics_exporter = "signozclickhousemetrics";
          latency_histogram_buckets = [
            "100us" "1ms" "2ms" "6ms" "10ms" "50ms" "100ms" "250ms"
            "500ms" "1000ms" "1400ms" "2000ms" "5s" "10s" "20s" "40s" "60s"
          ];
          dimensions_cache_size = 100000;
          dimensions = [
            { name = "service.namespace"; default = "default"; }
            { name = "deployment.environment"; default = "default"; }
            { name = "signoz.collector.id"; }
          ];
          aggregation_temporality = "AGGREGATION_TEMPORALITY_DELTA";
        };
      };
      extensions = {
        health_check.endpoint = "127.0.0.1:${toString collectorHealthPort}";
        zpages.endpoint = "127.0.0.1:55679";
        pprof.endpoint = "127.0.0.1:1777";
      };
      exporters = {
        clickhousetraces = {
          datasource = "\${env:SIGNOZ_CLICKHOUSE_TRACES_DSN}";
          use_new_schema = true;
        };
        signozclickhousemetrics = {
          dsn = "\${env:SIGNOZ_CLICKHOUSE_METRICS_DSN}";
          database = "signoz_metrics";
          timeout = "45s";
        };
        clickhouselogsexporter = {
          dsn = "\${env:SIGNOZ_CLICKHOUSE_LOGS_DSN}";
          timeout = "10s";
          use_new_schema = true;
        };
        metadataexporter = {
          dsn = "\${env:SIGNOZ_CLICKHOUSE_METADATA_DSN}";
          timeout = "10s";
          tenant_id = "default";
          cache.provider = "in_memory";
        };
      };
      service = {
        telemetry.logs.encoding = "json";
        extensions = [ "health_check" "zpages" "pprof" ];
        pipelines = {
          traces = {
            receivers = [ "otlp" ];
            processors = [ "signozspanmetrics/delta" "memory_limiter" "batch" ];
            exporters = [ "clickhousetraces" "metadataexporter" ];
          };
          metrics = {
            receivers = [ "otlp" ];
            processors = [ "memory_limiter" "batch" ];
            exporters = [ "metadataexporter" "signozclickhousemetrics" ];
          };
          logs = {
            receivers = [ "otlp" ];
            processors = [ "memory_limiter" "batch" ];
            exporters = [ "clickhouselogsexporter" "metadataexporter" ];
          };
        };
      };
    }
  );

  clickhouseStart = pkgs.writeShellScript "nixling-clickhouse-start" ''
    set -eu
    export SIGNOZ_CLICKHOUSE_PASSWORD="$(cat "$CREDENTIALS_DIRECTORY/${clickhousePasswordCredential}")"
    exec ${pkgs.clickhouse}/bin/clickhouse-server --config=/etc/clickhouse-server/config.xml
  '';

  signozStart = pkgs.writeShellScript "nixling-signoz-start" ''
    set -eu
    export SIGNOZ_ANALYTICS_ENABLED=false
    export TELEMETRY_ENABLED=false
    export SIGNOZ_TOKENIZER_JWT_SECRET="$(cat "$CREDENTIALS_DIRECTORY/${signozJwtCredential}")"
    export SIGNOZ_USER_ROOT_ENABLED=true
    export SIGNOZ_USER_ROOT_EMAIL="${cfg.signoz.adminEmail}"
    export SIGNOZ_USER_ROOT_PASSWORD="$(cat "$CREDENTIALS_DIRECTORY/${signozRootPasswordCredential}")"
    export SIGNOZ_TELEMETRYSTORE_CLICKHOUSE_DSN="${clickhouseDsnEnv}"
    export SIGNOZ_SQLSTORE_SQLITE_PATH=/var/lib/signoz/signoz.db
    export SIGNOZ_WEB_DIRECTORY=${signoz}/web
    exec ${signoz}/bin/signoz server --config ${signozConfig}
  '';

  collectorStart = pkgs.writeShellScript "nixling-signoz-otel-collector-start" ''
    set -eu
    pw="$(cat "$CREDENTIALS_DIRECTORY/${clickhousePasswordCredential}")"
    export SIGNOZ_CLICKHOUSE_TRACES_DSN="tcp://127.0.0.1:${toString clickhousePort}/signoz_traces?username=signoz&password=$pw"
    export SIGNOZ_CLICKHOUSE_METRICS_DSN="tcp://127.0.0.1:${toString clickhousePort}/signoz_metrics?username=signoz&password=$pw"
    export SIGNOZ_CLICKHOUSE_LOGS_DSN="tcp://127.0.0.1:${toString clickhousePort}/signoz_logs?username=signoz&password=$pw"
    export SIGNOZ_CLICKHOUSE_METADATA_DSN="tcp://127.0.0.1:${toString clickhousePort}/signoz_metadata?username=signoz&password=$pw"
    exec ${signozOtelCollector}/bin/signoz-otel-collector \
      --config ${collectorConfig} \
      --manager-config ${signozOtelCollector}/conf/opamp.yaml \
      --copy-path /var/lib/signoz-otel-collector/config.yaml
  '';

  migrateSync = pkgs.writeShellScript "nixling-signoz-migrate-sync" ''
    set -eu
    pw="$(cat "$CREDENTIALS_DIRECTORY/${clickhousePasswordCredential}")"
    dsn="tcp://127.0.0.1:${toString clickhousePort}?username=signoz&password=$pw"
    for _ in $(seq 1 120); do
      if ${pkgs.clickhouse}/bin/clickhouse-client --host 127.0.0.1 --port ${toString clickhousePort} --user signoz --password "$pw" --query 'SELECT 1' >/dev/null 2>&1; then
        break
      fi
      sleep 1
    done
    ${signozSchemaMigrator}/bin/signoz-schema-migrator sync --dsn "$dsn" --replication --cluster-name cluster
  '';

  migrateAsync = pkgs.writeShellScript "nixling-signoz-migrate-async" ''
    set -eu
    pw="$(cat "$CREDENTIALS_DIRECTORY/${clickhousePasswordCredential}")"
    dsn="tcp://127.0.0.1:${toString clickhousePort}?username=signoz&password=$pw"
    ${signozSchemaMigrator}/bin/signoz-schema-migrator async --dsn "$dsn" --replication --cluster-name cluster
  '';
in
{
  options.nixling.observability = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Enable the native SigNoz observability stack inside the
        auto-declared observability VM.
      '';
    };

    vmName = lib.mkOption {
      type = lib.types.str;
      default = "sys-obs";
      description = "VM name of the auto-declared observability VM.";
    };

    retention = lib.mkOption {
      type = lib.types.attrsOf lib.types.unspecified;
      default = { };
      description = "Compatibility surface for host-level retention options.";
    };

    grafana = lib.mkOption {
      type = lib.types.attrsOf lib.types.unspecified;
      default = { };
      description = "Compatibility surface for retired Grafana options.";
    };

    alerts = lib.mkOption {
      type = lib.types.attrsOf lib.types.unspecified;
      default = { };
      description = "Compatibility surface for alert toggle options.";
    };

    signoz = {
      listenAddress = lib.mkOption {
        type = lib.types.str;
        default = "0.0.0.0";
        description = "Address SigNoz binds inside the observability VM.";
      };

      listenPort = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = "TCP port SigNoz listens on inside the observability VM.";
      };

      otlpGrpcPort = lib.mkOption {
        type = lib.types.port;
        default = 4317;
        description = "Loopback OTLP gRPC port for the SigNoz collector.";
      };

      otlpHttpPort = lib.mkOption {
        type = lib.types.port;
        default = 4318;
        description = "Loopback OTLP HTTP port for the SigNoz collector.";
      };

      adminEmail = lib.mkOption {
        type = lib.types.str;
        default = "admin@nixling.local";
        description = "Root SigNoz admin email for first-run bootstrap.";
      };
    };

    transport.relayPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.socat;
      description = "Package providing the obs-VM-side vsock relay binary.";
    };
  };

  config = lib.mkIf cfg.enable {
    time.timeZone = lib.mkForce "America/Los_Angeles";

    services.zookeeper = {
      enable = true;
      dataDir = "/var/lib/zookeeper";
      port = zookeeperPort;
      extraConf = ''
        clientPortAddress=127.0.0.1
        admin.enableServer=false
      '';
      extraCmdLineOptions = [
        "-Xms256m"
        "-Xmx512m"
        "-Dcom.sun.management.jmxremote"
        "-Dcom.sun.management.jmxremote.local.only=true"
      ];
    };

    services.clickhouse = {
      enable = true;
      package = pkgs.clickhouse;
      serverConfig = {
        listen_host = "127.0.0.1";
        http_port = clickhouseHttpPort;
        tcp_port = clickhousePort;
        interserver_http_port = clickhouseInterserverPort;
        path = "/var/lib/clickhouse/";
        tmp_path = "/var/lib/clickhouse/tmp/";
        user_files_path = "/var/lib/clickhouse/user_files/";
        max_server_memory_usage = 4294967296;
        mark_cache_size = 536870912;
        uncompressed_cache_size = 268435456;
      };
      extraServerConfig = ''
        <clickhouse>
          <remote_servers>
            <cluster>
              <shard>
                <replica>
                  <host>127.0.0.1</host>
                  <port>${toString clickhousePort}</port>
                </replica>
              </shard>
            </cluster>
          </remote_servers>
          <zookeeper>
            <node>
              <host>127.0.0.1</host>
              <port>${toString zookeeperPort}</port>
            </node>
          </zookeeper>
          <macros>
            <shard>01</shard>
            <replica>01</replica>
          </macros>
        </clickhouse>
      '';
      extraUsersConfig = ''
        <clickhouse>
          <users>
            <default>
              <password remove="1"/>
            </default>
            <signoz>
              <password from_env="SIGNOZ_CLICKHOUSE_PASSWORD"/>
              <profile>default</profile>
              <networks>
                <ip>127.0.0.1</ip>
              </networks>
            </signoz>
          </users>
        </clickhouse>
      '';
    };

    systemd.services.clickhouse.serviceConfig = {
      ExecStart = lib.mkForce clickhouseStart;
      LoadCredential = [
        "${clickhousePasswordCredential}:${hostSecretsGuestDir}/clickhouse-password"
      ];
      LimitNOFILE = 1048576;
    };

    boot.kernel.sysctl."vm.max_map_count" = lib.mkForce 262144;
    boot.kernelParams = lib.mkAfter [ "transparent_hugepage=madvise" ];

    users.users.signoz = {
      isSystemUser = true;
      group = "signoz";
      home = "/var/lib/signoz";
      createHome = false;
      description = "SigNoz service user";
    };
    users.groups.signoz = { };

    systemd.services.signoz-schema-migrate-sync = {
      description = "Run SigNoz ClickHouse schema sync migrations";
      wantedBy = [ "multi-user.target" ];
      requires = [ "clickhouse.service" "zookeeper.service" ];
      after = [ "clickhouse.service" "zookeeper.service" ];
      before = [ "signoz.service" "signoz-otel-collector.service" ];
      serviceConfig = {
        Type = "oneshot";
        User = "signoz";
        Group = "signoz";
        ExecStart = migrateSync;
        LoadCredential = [
          "${clickhousePasswordCredential}:${hostSecretsGuestDir}/clickhouse-password"
        ];
      };
    };

    systemd.services.signoz-schema-migrate-async = {
      description = "Run SigNoz ClickHouse schema async migrations";
      wantedBy = [ "multi-user.target" ];
      requires = [ "clickhouse.service" ];
      after = [ "clickhouse.service" "signoz-schema-migrate-sync.service" ];
      serviceConfig = {
        Type = "oneshot";
        User = "signoz";
        Group = "signoz";
        ExecStart = migrateAsync;
        LoadCredential = [
          "${clickhousePasswordCredential}:${hostSecretsGuestDir}/clickhouse-password"
        ];
      };
    };

    systemd.services.signoz = {
      description = "SigNoz server and UI";
      wantedBy = [ "multi-user.target" ];
      requires = [ "signoz-schema-migrate-sync.service" ];
      after = [ "signoz-schema-migrate-sync.service" ];
      serviceConfig = {
        Type = "simple";
        User = "signoz";
        Group = "signoz";
        WorkingDirectory = "${signoz}";
        ExecStart = signozStart;
        Restart = "on-failure";
        StateDirectory = "signoz";
        LoadCredential = [
          "${clickhousePasswordCredential}:${hostSecretsGuestDir}/clickhouse-password"
          "${signozJwtCredential}:${hostSecretsGuestDir}/signoz-jwt-secret"
          "${signozRootPasswordCredential}:${hostSecretsGuestDir}/signoz-root-password"
        ];
        NoNewPrivileges = true;
      };
    };

    systemd.services.signoz-otel-collector = {
      description = "SigNoz OTel Collector";
      wantedBy = [ "multi-user.target" ];
      requires = [ "signoz-schema-migrate-sync.service" ];
      after = [ "signoz-schema-migrate-sync.service" ];
      serviceConfig = {
        Type = "simple";
        User = "signoz";
        Group = "signoz";
        WorkingDirectory = "${signozOtelCollector}";
        ExecStart = collectorStart;
        Restart = "on-failure";
        StateDirectory = "signoz-otel-collector";
        LoadCredential = [
          "${clickhousePasswordCredential}:${hostSecretsGuestDir}/clickhouse-password"
        ];
        NoNewPrivileges = true;
      };
    };

    systemd.services.nixling-otel-vsock-in = {
      description = "Receive OTLP from host relays and forward to the SigNoz collector";
      wantedBy = [ "multi-user.target" ];
      wants = [ "signoz-otel-collector.service" ];
      after = [ "signoz-otel-collector.service" ];
      serviceConfig = {
        Type = "exec";
        ExecStart = "${cfg.transport.relayPackage}/bin/socat -d -d VSOCK-LISTEN:14317,fork,max-children=64,reuseaddr TCP:127.0.0.1:${toString otlpGrpcPort}";
        Restart = "on-failure";
        RestartSec = "3s";
        TasksMax = 128;
        MemoryMax = "128M";
        LimitNOFILE = 2048;
        User = "signoz";
        Group = "signoz";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        PrivateTmp = true;
        PrivateDevices = false;
        DeviceAllow = [ "/dev/vsock rw" ];
        RestrictAddressFamilies = [ "AF_UNIX" "AF_VSOCK" "AF_INET" ];
        SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
        UMask = "0077";
      };
    };

    networking.firewall.allowedTCPPorts = [ cfg.signoz.listenPort ];

    environment.systemPackages = [
      signoz
      signozOtelCollector
      signozSchemaMigrator
      pkgs.clickhouse
    ];
  };
}
