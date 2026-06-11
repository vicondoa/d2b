# Observability stack guest component for the auto-declared stack VM.
#
# TODO: `component-stack`.
# This module will install the observability backend and inbound vsock
# receiver inside the auto-declared `sys-obs` VM.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling.observability;
  quote = builtins.toJSON;

  prometheusPort = 9090;
  lokiPort = 3100;
  lokiGrpcPort = 9096;
  tempoHttpPort = 3200;
  tempoOtlpGrpcPort = 4317;
  alloyMetricsPort = 12345;

  grafanaCredentialDir = "/var/lib/nixling-observability";
  # Internal framework convention: when neither secret is overridden
  # by the consumer, `observability-host-secrets.nix` generates them
  # on the host and shares them read-only into this VM at the path
  # below. The string is duplicated between the two modules
  # deliberately — it is an internal contract, not a public option.
  hostSecretsGuestDir = "/run/nixling-obs-secrets";
  grafanaSecretKeyPath = "${grafanaCredentialDir}/grafana-secret-key";
  grafanaSecretKeySource =
    if cfg.grafana.secretKeyFile != null
    then toString cfg.grafana.secretKeyFile
    else "${hostSecretsGuestDir}/grafana-secret-key";
  grafanaAdminPasswordPath = "${grafanaCredentialDir}/grafana-admin-password";
  grafanaAdminPasswordSource =
    if cfg.grafana.adminPasswordFile != null
    then toString cfg.grafana.adminPasswordFile
    else "${hostSecretsGuestDir}/grafana-admin-password";
  alloyRuntimeDir = "/run/nixling/alloy";
  obsIngressSocket = "${alloyRuntimeDir}/obs-ingress.sock";
  stackHostName = config.networking.hostName;

  # Canonical tempo retention + sampling
  # policy. Per-tenant overrides are emitted as a separate YAML file
  # because Tempo's inline `overrides.defaults.*` form covers only
  # the default tenant; per-tenant rows (the critical tenant getting
  # `tracesCritical` retention) live under
  # `overrides.per_tenant_override_config`.
  tempoPerTenantOverrides = pkgs.writeText "nixling-tempo-overrides.yaml" (
    lib.generators.toYAML { } {
      overrides = {
        "${cfg.sampling.criticalTenant}" = {
          compaction = {
            block_retention = cfg.retention.tracesCritical;
          };
        };
      };
    }
  );

  samplingPercentageCritical =
    builtins.toString (cfg.sampling.criticalRatio * 100.0);
  samplingPercentageDefault =
    builtins.toString (cfg.sampling.defaultRatio * 100.0);

  dashboardDir = pkgs.runCommand "nixling-grafana-dashboards" { } ''
    mkdir -p "$out"
    cp ${./dashboards/01-nixling-overview.json} "$out/01-nixling-overview.json"
    cp ${./dashboards/02-vm-resources.json} "$out/02-vm-resources.json"
    cp ${./dashboards/03-lifecycle-traces.json} "$out/03-lifecycle-traces.json"
    cp ${./dashboards/04-logs.json} "$out/04-logs.json"
    cp ${./dashboards/05-per-vm-store.json} "$out/05-per-vm-store.json"
    cp ${./dashboards/06-obs-vm-health.json} "$out/06-obs-vm-health.json"
    substituteInPlace "$out/06-obs-vm-health.json" \
      --replace-fail "__OBS_VM_NAME__" "${cfg.vmName}"
  '';

  allAlertRules = [
    {
      alert = "NixlingVMDown";
      expr = ''
        max by (vm, env, role) (nixling_vm_observability_enabled == 1)
        and on (vm, env, role)
        max by (vm, env, role) (nixling_vm_running == 0)
      '';
      for = "5m";
      labels = {
        severity = "warning";
        track = "observability";
      };
      annotations = {
        summary = "Nixling VM down";
        description = "VM {{ $labels.vm }} (env {{ $labels.env }}) has been unreachable for 5 minutes.";
      };
    }
    {
      alert = "NixlingNetVMDownWithRunningWorkloads";
      expr = ''
        max by (env) (nixling_vm_running{role="router"} == 0)
        and on (env)
        max by (env) (nixling_vm_running{role!="router"} == 1)
      '';
      for = "5m";
      labels = {
        severity = "critical";
        track = "observability";
      };
      annotations = {
        summary = "Nixling router VM down";
        description = "Router VM for env {{ $labels.env }} is down while workload VMs are still running.";
      };
    }
    {
      alert = "NixlingObsVMUnreachableFromHost";
      expr = ''
        nixling_vm_ch_api_up{role="obs"} == 0
      '';
      for = "10m";
      labels = {
        severity = "warning";
        track = "observability";
      };
      annotations = {
        summary = "Observability VM unreachable";
        description = "Observability VM {{ $labels.vm }} (env {{ $labels.env }}) has been unreachable from the host for 10 minutes; telemetry collection has halted.";
      };
    }
    {
      alert = "NixlingVsockRelayDown";
      expr = ''
        label_replace(
          node_systemd_unit_state{name=~"nixling-otel-relay@.+\\.service",state="active"} == 0,
          "vm",
          "$1",
          "name",
          "nixling-otel-relay@(.+)\\.service"
        )
        * on (vm) group_left (env)
        max by (vm, env) (nixling_vm_running == 1)
      '';
      for = "3m";
      labels = {
        severity = "warning";
        track = "observability";
      };
      annotations = {
        summary = "Nixling vsock relay down";
        description = "Vsock relay for VM {{ $labels.vm }} (env {{ $labels.env }}) has been inactive for 3 minutes.";
      };
    }
    {
      alert = "NixlingCHAPISocketMissing";
      expr = ''
        max by (vm, env) (nixling_vm_running == 1)
        and on (vm, env)
        max by (vm, env) (nixling_vm_ch_api_up == 0)
      '';
      for = "2m";
      labels = {
        severity = "warning";
        track = "observability";
      };
      annotations = {
        summary = "Nixling CH API unavailable";
        description = "Cloud Hypervisor API for VM {{ $labels.vm }} (env {{ $labels.env }}) is unreachable while the host still expects the VM to be running.";
      };
    }
    {
      alert = "NixlingStoreSyncFailure";
      expr = ''
        label_replace(
          label_replace(
            max_over_time(
              node_systemd_unit_state{
                name=~"nixling-.+-store-sync\\.service|nixling-store-sync@.+\\.service",
                state="failed"
              }[10m]
            ) > 0,
            "vm",
            "$1",
            "name",
            "nixling-(.+)-store-sync\\.service"
          ),
          "vm",
          "$1",
          "name",
          "nixling-store-sync@(.+)\\.service"
        )
        + on (vm) group_left (env)
        (0 * max by (vm, env) (nixling_vm_running))
      '';
      labels = {
        severity = "warning";
        track = "observability";
      };
      annotations = {
        summary = "Nixling store sync failed";
        description = "Store sync for VM {{ $labels.vm }} (env {{ $labels.env }}) has failed within the last 10 minutes.";
      };
    }
    {
      alert = "NixlingGuestTelemetryMissing";
      expr = ''
        max by (vm, env) (nixling_vm_observability_enabled == 1)
        * on (vm, env)
        max by (vm, env) (nixling_vm_running == 1)
        unless on (vm, env)
        max by (vm, env) (count_over_time(up{job="nixling-vm-telemetry",vm=~".+"}[10m]) > 0)
      '';
      labels = {
        severity = "info";
        track = "observability";
      };
      annotations = {
        summary = "Guest telemetry missing";
        description = "Guest VM {{ $labels.vm }} has not reported telemetry in 10 minutes.";
      };
    }
    {
      alert = "NixlingObsVMStackUnhealthy";
      expr = ''
        up{job=~"^(grafana|prometheus|loki|tempo|alloy)$"} == 0
      '';
      for = "5m";
      labels = {
        severity = "critical";
        track = "observability";
      };
      annotations = {
        summary = "Observability stack component down";
        description = "An observability stack component has been unreachable for 5 minutes.";
      };
    }
  ];

  enabledAlertRules = builtins.filter (
    rule: lib.attrByPath [ rule.alert "enable" ] true cfg.alerts
  ) allAlertRules;

  nixlingObservabilityRules = pkgs.writeText "nixling-observability.rules.yml" (
    lib.generators.toYAML { } {
      groups = lib.optional (enabledAlertRules != [ ]) {
        name = "nixling_observability";
        interval = "30s";
        rules = enabledAlertRules;
      };
    }
  );

  alloyConfig = pkgs.writeText "nixling-observability-stack.alloy" (
    lib.concatStringsSep "\n\n" [
      ''
        prometheus.remote_write "local" {
          endpoint {
            url = "http://127.0.0.1:${toString prometheusPort}/api/v1/write"
          }
        }
      ''

      ''
        otelcol.exporter.prometheus "metrics" {
          forward_to = [prometheus.remote_write.local.receiver]
        }
      ''

      ''
        loki.write "local" {
          endpoint {
            url = "http://127.0.0.1:${toString lokiPort}/loki/api/v1/push"
          }
        }
      ''

      ''
        otelcol.exporter.loki "logs" {
          forward_to = [loki.write.local.receiver]
        }
      ''

      ''
        // Tempo retention + sampling budget.
        // Pipeline shape:
        //   otelcol.receiver.otlp.ingress.traces
        //     -> otelcol.processor.tail_sampling.tempo_budget
        //          (critical=always, default=probabilistic ${samplingPercentageDefault}%)
        //     -> otelcol.connector.routing.tempo_tenant
        //          (by attribute ${cfg.sampling.criticalAttribute}=${cfg.sampling.criticalValue})
        //     -> otelcol.exporter.otlp.traces_{critical,default}
        //          (sets X-Scope-OrgID to ${cfg.sampling.criticalTenant}
        //           / ${cfg.sampling.defaultTenant} so Tempo's per-tenant
        //           retention overrides apply).
        otelcol.processor.tail_sampling "tempo_budget" {
          decision_wait               = "5s"
          num_traces                  = 50000
          expected_new_traces_per_sec = 100

          policy {
            name = "critical_keep_all"
            type = "and"
            and {
              and_sub_policy {
                name = "match_critical_attribute"
                type = "string_attribute"
                string_attribute {
                  key    = ${quote cfg.sampling.criticalAttribute}
                  values = [${quote cfg.sampling.criticalValue}]
                }
              }
              and_sub_policy {
                name = "always_sample"
                type = "always_sample"
              }
            }
          }

          policy {
            name = "default_probabilistic"
            type = "probabilistic"
            probabilistic {
              sampling_percentage = ${samplingPercentageDefault}
            }
          }

          output {
            traces = [otelcol.connector.routing.tempo_tenant.input]
          }
        }

        otelcol.connector.routing "tempo_tenant" {
          default_pipelines = [otelcol.exporter.otlp.traces_default.input]

          table {
            statement = "route() where attributes[${quote cfg.sampling.criticalAttribute}] == ${quote cfg.sampling.criticalValue}"
            pipelines = [otelcol.exporter.otlp.traces_critical.input]
          }

          output {
          }
        }

        otelcol.exporter.otlp "traces_default" {
          client {
            endpoint    = "127.0.0.1:${toString tempoOtlpGrpcPort}"
            compression = "none"
            headers     = {
              "X-Scope-OrgID" = ${quote cfg.sampling.defaultTenant},
            }
            tls {
              insecure = true
            }
          }
        }

        otelcol.exporter.otlp "traces_critical" {
          client {
            endpoint    = "127.0.0.1:${toString tempoOtlpGrpcPort}"
            compression = "none"
            headers     = {
              "X-Scope-OrgID" = ${quote cfg.sampling.criticalTenant},
            }
            tls {
              insecure = true
            }
          }
        }
      ''

      ''
        otelcol.receiver.otlp "ingress" {
          grpc {
            endpoint  = "${obsIngressSocket}"
            transport = "unix"
          }

          output {
            metrics = [otelcol.exporter.prometheus.metrics.input]
            logs    = [otelcol.exporter.loki.logs.input]
            traces  = [otelcol.processor.tail_sampling.tempo_budget.input]
          }
        }
      ''

      ''
        prometheus.exporter.unix "stack" {
        }

        discovery.relabel "stack_node_targets" {
          targets = prometheus.exporter.unix.stack.targets

          rule {
            target_label = "host"
            replacement  = ${quote stackHostName}
          }

          rule {
            target_label = "vm"
            replacement  = ${quote stackHostName}
          }

          rule {
            target_label = "env"
            replacement  = ${quote "obs"}
          }

          rule {
            target_label = "instance"
            replacement  = ${quote stackHostName}
          }
        }

        otelcol.receiver.prometheus "stack_node" {
          output {
            metrics = [otelcol.exporter.prometheus.metrics.input]
          }
        }

        prometheus.scrape "stack_node" {
          targets    = discovery.relabel.stack_node_targets.output
          forward_to = [otelcol.receiver.prometheus.stack_node.receiver]
        }
      ''
    ]
  );
in
{
  # `sys-obs-stack` only imports this guest module, not the host-side
  # options-observability.nix module, so re-declare the subset the VM
  # consumes here with the same defaults.
  options.nixling.observability = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Enable the observability stack inside the auto-declared stack
        VM. Defaults to `true` because this module is only imported by
        `sys-obs-stack`.
      '';
    };

    vmName = lib.mkOption {
      type = lib.types.str;
      default = "sys-obs";
      description = ''
        VM name of the auto-declared observability stack VM.
      '';
    };

    retention = {
      metrics = lib.mkOption {
        type = lib.types.str;
        default = "30d";
        description = "Retention window for metrics in the observability stack VM.";
      };

      logs = lib.mkOption {
        type = lib.types.str;
        default = "14d";
        description = "Retention window for logs in the observability stack VM.";
      };

      traces = lib.mkOption {
        type = lib.types.str;
        default = "7d";
        description = ''
          Default retention window for traces in the observability
          stack VM. Applies to the default Tempo tenant
          (`sampling.defaultTenant`). See
          [docs/reference/tempo-retention-sampling.md] for the
          canonical policy.
        '';
      };

      tracesCritical = lib.mkOption {
        type = lib.types.str;
        default = "30d";
        description = ''
          Retention window for the critical Tempo tenant
          (`sampling.criticalTenant`). Spans tagged with the
          configured `sampling.criticalAttribute` =
          `sampling.criticalValue` (default `kind = "critical"`)
          are routed to this tenant and kept for this longer
          window so post-incident forensics on framework-critical
          events (SpawnRunner failures, BundleTampered,
          broker authz denials, etc.) stay queryable beyond the
          default 7-day budget.
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
          Span attribute value (matched against
          `sampling.criticalAttribute`) that pins a span into the
          critical Tempo tenant — sampled at
          `sampling.criticalRatio` and retained for
          `retention.tracesCritical`.
        '';
      };

      criticalRatio = lib.mkOption {
        type = lib.types.float;
        default = 1.0;
        description = ''
          Sampling ratio for critical spans, 0.0–1.0. Pinned to
          1.0 by the canonical policy so every critical span is
          retained.
        '';
      };

      defaultRatio = lib.mkOption {
        type = lib.types.float;
        default = 0.1;
        description = ''
          Head-consistent (traceID-deterministic) sampling ratio
          for non-critical spans, 0.0–1.0. Pinned to 0.1 by the
          canonical policy to keep per-VM trace volume inside
          the disk budget documented in
          docs/reference/tempo-retention-sampling.md.
        '';
      };

      criticalTenant = lib.mkOption {
        type = lib.types.str;
        default = "nixling-critical";
        description = ''
          Tempo tenant id (set as `X-Scope-OrgID`) that critical
          spans are routed to.
        '';
      };

      defaultTenant = lib.mkOption {
        type = lib.types.str;
        default = "nixling-default";
        description = ''
          Tempo tenant id (set as `X-Scope-OrgID`) that
          non-critical spans are routed to.
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
        default = "0.0.0.0";
        description = "Address Grafana binds inside the observability stack VM. The auto-declared obs VM injects the derived env IP via the host-level option; standalone use can override this.";
      };

      listenPort = lib.mkOption {
        type = lib.types.port;
        default = 3000;
        description = "TCP port Grafana listens on inside the observability stack VM.";
      };

      secretKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Optional path to a file containing the Grafana session/signing
          secret key. Operators can use this to source the credential
          from sops-nix, agenix, or another declarative secrets
          framework instead of the framework-generated default.
        '';
      };

      adminPasswordFile = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = ''
          Optional path to a file containing the Grafana admin password.
          Operators can use this to source the credential from sops-nix,
          agenix, or another declarative secrets framework instead of
          the framework-generated default.
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

    transport.relayPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.socat;
      description = ''
        Package providing the obs-VM-side vsock relay binary. Defaults
        to `pkgs.socat`.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    services.alloy = {
      enable = true;
      configPath = alloyConfig;
      extraFlags = [ "--server.http.listen-addr=127.0.0.1:${toString alloyMetricsPort}" ];
    };

    services.prometheus = {
      enable = true;
      listenAddress = "127.0.0.1";
      port = prometheusPort;
      retentionTime = cfg.retention.metrics;
      extraFlags = [ "--web.enable-remote-write-receiver" ];
      ruleFiles = [ nixlingObservabilityRules ];
      scrapeConfigs = [
        {
          job_name = "prometheus";
          static_configs = [
            {
              targets = [ "127.0.0.1:${toString prometheusPort}" ];
            }
          ];
        }
        {
          job_name = "grafana";
          metrics_path = "/metrics";
          static_configs = [
            {
              targets = [ "${cfg.grafana.listenAddress}:${toString cfg.grafana.listenPort}" ];
            }
          ];
        }
        {
          job_name = "loki";
          static_configs = [
            {
              targets = [ "127.0.0.1:${toString lokiPort}" ];
            }
          ];
        }
        {
          job_name = "tempo";
          static_configs = [
            {
              targets = [ "127.0.0.1:${toString tempoHttpPort}" ];
            }
          ];
        }
        {
          job_name = "alloy";
          static_configs = [
            {
              targets = [ "127.0.0.1:${toString alloyMetricsPort}" ];
            }
          ];
        }
      ];
    };

    services.loki = {
      enable = true;
      configuration = {
        auth_enabled = false;
        server = {
          http_listen_address = "127.0.0.1";
          http_listen_port = lokiPort;
          grpc_listen_address = "127.0.0.1";
          grpc_listen_port = lokiGrpcPort;
        };
        common = {
          instance_addr = "127.0.0.1";
          path_prefix = "/var/lib/loki";
          storage = {
            filesystem = {
              chunks_directory = "/var/lib/loki/chunks";
              rules_directory = "/var/lib/loki/rules";
            };
          };
          replication_factor = 1;
          ring = {
            kvstore = {
              store = "inmemory";
            };
          };
        };
        schema_config = {
          configs = [
            {
              from = "2024-01-01";
              store = "tsdb";
              object_store = "filesystem";
              schema = "v13";
              index = {
                prefix = "index_";
                period = "24h";
              };
            }
          ];
        };
        compactor = {
          working_directory = "/var/lib/loki/compactor";
          compaction_interval = "5m";
          retention_enabled = true;
          delete_request_store = "filesystem";
          retention_delete_delay = "1h";
        };
        limits_config = {
          retention_period = cfg.retention.logs;
        };
        frontend = {
          encoding = "protobuf";
        };
        analytics = {
          reporting_enabled = false;
        };
      };
    };

    services.tempo = {
      enable = true;
      settings = {
        # Enable multi-tenancy so per-tenant
        # retention overrides apply. Alloy attaches the X-Scope-OrgID
        # header per critical / default span via two OTLP exporters
        # downstream of the tail-sampling processor.
        multitenancy_enabled = true;
        server = {
          http_listen_address = "127.0.0.1";
          http_listen_port = tempoHttpPort;
        };
        distributor = {
          receivers = {
            otlp = {
              protocols = {
                grpc = {
                  endpoint = "127.0.0.1:${toString tempoOtlpGrpcPort}";
                };
              };
            };
          };
        };
        ingester = {
          max_block_duration = "5m";
        };
        compactor = {
          compaction = {
            # Global ceiling: blocks live at most `tracesCritical`
            # before the compactor evicts them. Per-tenant overrides
            # (below) shorten the default tenant to `traces`.
            block_retention = cfg.retention.tracesCritical;
          };
        };
        # Policy: default tenant -> `retention.traces` (7d);
        # critical tenant -> `retention.tracesCritical` (30d) via the
        # per_tenant_override_config file generated above.
        overrides = {
          defaults = {
            compaction = {
              block_retention = cfg.retention.traces;
            };
          };
          per_tenant_override_config = toString tempoPerTenantOverrides;
        };
        storage = {
          trace = {
            backend = "local";
            local = {
              path = "/var/lib/tempo/blocks";
            };
            wal = {
              path = "/var/lib/tempo/wal";
            };
          };
        };
      };
    };

    services.grafana = {
      enable = true;
      settings = {
        server = {
          protocol = "http";
          http_addr = cfg.grafana.listenAddress;
          http_port = cfg.grafana.listenPort;
          domain = cfg.grafana.listenAddress;
          root_url = "http://${cfg.grafana.listenAddress}:${toString cfg.grafana.listenPort}/";
        };
        users = {
          allow_sign_up = false;
          auto_assign_org = true;
          auto_assign_org_role = "Viewer";
        };
        auth = {
          # Keep the login form available even when anonymous Viewer
          # is enabled, so operators can still sign in as nixling-admin
          # for admin tasks (contact points, dashboard provisioning,
          # plugin install).
          disable_login_form = false;
        };
        # Anonymous dashboards stay opt-in for trusted single-host LAN
        # deployments; authenticated access is the default.
        "auth.anonymous" = {
          enabled = cfg.grafana.anonymousViewer.enable;
          org_name = "Main Org.";
          org_role = "Viewer";
        };
        analytics = {
          reporting_enabled = false;
        };
        metrics = {
          enabled = true;
        };
        security = {
          admin_user = "nixling-admin";
          admin_password = "$__file{/run/credentials/grafana.service/admin_password}";
          secret_key = "$__file{/run/credentials/grafana.service/secret_key}";
        };
      };
      provision.datasources.settings = {
        apiVersion = 1;
        prune = true;
        datasources = [
          {
            name = "Prometheus";
            type = "prometheus";
            uid = "prometheus";
            access = "proxy";
            url = "http://127.0.0.1:${toString prometheusPort}";
            isDefault = true;
            editable = false;
          }
          {
            name = "Loki";
            type = "loki";
            uid = "loki";
            access = "proxy";
            url = "http://127.0.0.1:${toString lokiPort}";
            editable = false;
            jsonData = {
              derivedFields = [
                {
                  datasourceUid = "tempo";
                  matcherRegex = "\"trace_id\":\"([0-9a-fA-F]{32})\"";
                  name = "TraceID";
                  url = "$\${__value.raw}";
                  urlDisplayLabel = "View Trace";
                }
              ];
            };
          }
          {
            name = "Tempo";
            type = "tempo";
            uid = "tempo";
            access = "proxy";
            url = "http://127.0.0.1:${toString tempoHttpPort}";
            editable = false;
            # Multitenancy is on; default
            # Tempo datasource queries the default tenant. A sibling
            # `Tempo (Critical)` datasource (uid = `tempo-critical`)
            # queries the critical tenant.
            jsonData = {
              httpHeaderName1 = "X-Scope-OrgID";
              serviceMap = {
                datasourceUid = "prometheus";
              };
              tracesToLogsV2 = {
                datasourceUid = "loki";
                spanStartTimeShift = "-2s";
                spanEndTimeShift = "2s";
                filterByTraceID = true;
                filterBySpanID = false;
                customQuery = true;
                tags = [
                  {
                    key = "vm.name";
                    value = "vm";
                  }
                  {
                    key = "vm.env";
                    value = "env";
                  }
                  {
                    key = "systemd.unit";
                    value = "unit";
                  }
                ];
                query = "{vm=\"$\${__span.tags[\\\"vm.name\\\"]}\", env=\"$\${__span.tags[\\\"vm.env\\\"]}\"} | json | trace_id=\"$\${__trace.traceId}\"";
              };
              search = {
                hide = false;
              };
              traceQuery = {
                timeShiftEnabled = true;
                spanStartTimeShift = "-2s";
                spanEndTimeShift = "2s";
              };
              streamingEnabled = {
                search = true;
                metrics = true;
              };
            };
            secureJsonData = {
              httpHeaderValue1 = "$__env{NIXLING_TEMPO_DEFAULT_TENANT}";
            };
          }
          {
            # Sibling Tempo datasource that
            # queries the critical tenant (`X-Scope-OrgID =
            # ${cfg.sampling.criticalTenant}`). Critical spans are
            # retained for `retention.tracesCritical` (default 30d)
            # so post-incident forensics on SpawnRunner failures,
            # BundleTampered, broker authz denials, etc. stay
            # queryable beyond the default 7d budget.
            name = "Tempo (Critical)";
            type = "tempo";
            uid = "tempo-critical";
            access = "proxy";
            url = "http://127.0.0.1:${toString tempoHttpPort}";
            editable = false;
            jsonData = {
              httpHeaderName1 = "X-Scope-OrgID";
              serviceMap = {
                datasourceUid = "prometheus";
              };
            };
            secureJsonData = {
              httpHeaderValue1 = "$__env{NIXLING_TEMPO_CRITICAL_TENANT}";
            };
          }
        ];
      };
      provision.dashboards.settings = {
        apiVersion = 1;
        providers = [
          {
            name = "nixling";
            folder = "Nixling";
            type = "file";
            disableDeletion = false;
            allowUiUpdates = false;
            updateIntervalSeconds = 30;
            options = {
              path = "${dashboardDir}";
              foldersFromFilesStructure = false;
            };
          }
        ];
      };
    };

    systemd.services.grafana.serviceConfig.LoadCredential = [
      "secret_key:${grafanaSecretKeySource}"
      "admin_password:${grafanaAdminPasswordSource}"
    ];
    systemd.services.grafana.environment = {
      NIXLING_TEMPO_DEFAULT_TENANT = cfg.sampling.defaultTenant;
      NIXLING_TEMPO_CRITICAL_TENANT = cfg.sampling.criticalTenant;
    };

    # NOTE: as of v0.2.0, the framework generates the Grafana secret
    # key and admin password on the HOST (see
    # `nixos-modules/observability-host-secrets.nix`) and shares them
    # into this VM read-only at `/run/nixling-obs-secrets/`. The
    # in-VM activation scripts that used to live here are gone — they
    # generated the credentials in the wrong filesystem (the guest)
    # and forced consumers to add a sudoable operator account inside
    # `sys-obs-stack` just to read them back out to the host. If
    # either of `cfg.grafana.{secretKeyFile,adminPasswordFile}` is
    # overridden by a consumer, neither the host generator nor this
    # module touches that secret.

    # Keep Grafana reachable from the host on the dedicated obs LAN. The
    # default 10.40.0.10 bind address lives on a single-host host-LAN-only
    # segment and is not Internet-routed.
    networking.firewall.allowedTCPPorts = [ cfg.grafana.listenPort ];

    # Static alloy user/group inside the stack VM. The
    # nixling-otel-vsock-in sidecar runs as User=alloy and needs the
    # user to exist outside of alloy.service's lifecycle.
    users.users.alloy = {
      isSystemUser = true;
      group = "alloy";
      home = "/var/lib/alloy";
      createHome = false;
      description = "Grafana Alloy (nixling-managed static account)";
    };
    users.groups.alloy = { };

    systemd.services.alloy.serviceConfig = {
      DynamicUser = lib.mkForce false;
      User = lib.mkForce "alloy";
      Group = lib.mkForce "alloy";

      StateDirectory = lib.mkAfter [ "alloy" ];
      StateDirectoryMode = "0750";

      RuntimeDirectory = lib.mkAfter [ "nixling/alloy" ];
      RuntimeDirectoryMode = "0710";
      RuntimeDirectoryPreserve = "yes";
    };

    systemd.services.nixling-otel-vsock-in = {
      description = "Receive OTLP from host relay, forward to obs Alloy UDS.";
      wantedBy = [ "multi-user.target" ];
      after = [ "alloy.service" ];
      bindsTo = [ "alloy.service" ];
      restartIfChanged = false;
      startLimitBurst = 20;
      startLimitIntervalSec = 300;

      serviceConfig = {
        Type = "exec";
        ExecStart = "${cfg.transport.relayPackage}/bin/socat -d -d VSOCK-LISTEN:14317,fork,max-children=16,reuseaddr UNIX-CONNECT:/run/nixling/obs-ingress.sock";
        Restart = "on-failure";
        RestartSec = "3s";
        TasksMax = 32;
        MemoryMax = "64M";
        LimitNOFILE = 1024;
        User = "alloy";
        Group = "alloy";
        SupplementaryGroups = [ ];
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

    # Keep the documented /run/nixling/obs-ingress.sock path stable for
    # clients/tests while the real Alloy ingress socket lives privately under
    # /run/nixling/alloy/.
    systemd.tmpfiles.rules = lib.mkAfter [
      "d /run/nixling 0755 root root -"
      "d /run/nixling/alloy 0700 alloy alloy -"
      "L+ /run/nixling/obs-ingress.sock - - - - ${obsIngressSocket}"
    ];
  };
}
