# Host-side observability component.
#
# TODO: `component-host`.
# Owns the host Alloy forwarder and the host-local bridge into the
# observability VM's vsock backend.
{ config, lib, pkgs, ... }:

let
  cfg = config.nixling.observability;
  hostName = config.networking.hostName;
  obsOtlpPort = 14317;
  vsockSocketForPort = socketPath: port: "${socketPath}_${toString port}";
  obsVsockHostSocket = "${config.nixling.store.stateDir}/${cfg.vmName}/vsock.sock";
  obsOtlpVsockHostSocket = vsockSocketForPort obsVsockHostSocket obsOtlpPort;
  alloyRuntimeDir = "/run/nixling/alloy";
  hostOtlpSocket = "${alloyRuntimeDir}/host-otlp.sock";
  hostEgressSocket = "${alloyRuntimeDir}/host-egress.sock";

  chVsockConnect = import ../../nixling-ch-vsock-connect.nix { inherit pkgs; };

  enabledVms = lib.filterAttrs (_: vm: vm.enable) config.nixling.vms;
  envNames = lib.attrNames config.nixling.envs;

  quote = builtins.toJSON;
  sanitizeLabel = value:
    builtins.replaceStrings [ "@" "." "-" "/" ] [ "_" "_" "_" "_" ] value;

  # Loki label contract: see docs/reference/loki-label-contract.md.
  # Only {vm, env, role, severity, source} are emitted as labels.
  # The systemd unit name is preserved as a `matches` filter (so we
  # only consume the right journald stream) but NOT promoted to a
  # label — unit names are an unbounded path-like axis. The host
  # name is reflected by role="host" / vm="host" rather than a
  # dedicated `host` label.
  mkJournalSource =
    {
      label,
      unit,
      vm,
      env,
      role,
    }:
    ''
      loki.source.journal "${label}" {
        forward_to = [otelcol.receiver.loki.journal.receiver]
        matches    = "_SYSTEMD_UNIT=${unit}"
        labels = {
          vm     = ${quote vm},
          env    = ${quote env},
          role   = "${role}",
          source = "journal",
        }
      }
    '';

  perVmJournalSources =
    lib.concatMapStringsSep "\n\n"
      (name:
        let
          manifestVm = config.nixling.manifest.${name};
          envLabel = if manifestVm.env != null then manifestVm.env else "none";
          units = [
            "microvm@${name}.service"
            "microvm-virtiofsd@${name}.service"
            "nixling-${name}-swtpm.service"
            "swtpm@${name}.service"
            "nixling-${name}-store-sync.service"
            "nixling-store-sync@${name}.service"
            "nixling-${name}-snd.service"
            "nixling-snd@${name}.service"
            "nixling-otel-relay@${name}.service"
          ];
        in
        lib.concatMapStringsSep "\n\n"
          (unit:
            mkJournalSource {
              label = sanitizeLabel "journal_${name}_${unit}";
              inherit unit;
              vm = name;
              env = envLabel;
              role = "workload";
            })
          units)
      (lib.attrNames enabledVms);

  usbipdJournalSources =
    lib.concatMapStringsSep "\n\n"
      (env:
        lib.concatMapStringsSep "\n\n"
          (unit:
            mkJournalSource {
              label = sanitizeLabel "journal_${env}_${unit}";
              inherit unit env;
              vm = "host";
              role = "usbipd";
            })
          [
            "nixling-sys-${env}-usbipd-backend.service"
            "nixling-sys-${env}-usbipd-proxy.service"
            "nixling-sys-${env}-usbipd-proxy.socket"
          ])
      envNames;

  singletonJournalSources = lib.concatStringsSep "\n\n" (
    [
      (mkJournalSource {
        label = "journal_nixling_otel_host_bridge_service";
        unit = "nixling-otel-host-bridge.service";
        vm = "host";
        env = cfg.env;
        role = "host";
      })
      (mkJournalSource {
        label = "journal_usbipd_nixling_service";
        unit = "usbipd-nixling.service";
        vm = "host";
        env = "host";
        role = "host";
      })
    ]
  );

  journalSources = lib.concatStringsSep "\n\n" (
    lib.filter (section: section != "") [
      perVmJournalSources
      usbipdJournalSources
      singletonJournalSources
    ]
  );

  alloyConfig = pkgs.writeText "nixling-observability-host.alloy" (
    lib.concatStringsSep "\n\n"
      (
        [
          ''
            otelcol.exporter.otlp "egress" {
              client {
                endpoint = "unix://${hostEgressSocket}"
                compression = "none"

                tls {
                  insecure = true
                }
              }
            }

            otelcol.receiver.otlp "host_local" {
              grpc {
                endpoint  = "${hostOtlpSocket}"
                transport = "unix"
              }

              output {
                metrics = [otelcol.exporter.otlp.egress.input]
                logs    = [otelcol.exporter.otlp.egress.input]
                traces  = [otelcol.exporter.otlp.egress.input]
              }
            }
          ''

          ''
            prometheus.exporter.unix "host" {
            }

            discovery.relabel "host_node_targets" {
              targets = prometheus.exporter.unix.host.targets

              rule {
                target_label = "host"
                replacement  = ${quote hostName}
              }

              rule {
                target_label = "vm"
                replacement  = "host"
              }

              rule {
                target_label = "env"
                replacement  = "host"
              }

              rule {
                target_label = "instance"
                replacement  = ${quote hostName}
              }
            }

            otelcol.receiver.prometheus "host_node" {
              output {
                metrics = [otelcol.exporter.otlp.egress.input]
              }
            }

            prometheus.scrape "host_node" {
              targets    = discovery.relabel.host_node_targets.output
              forward_to = [otelcol.receiver.prometheus.host_node.receiver]
            }
          ''

          ''
            prometheus.exporter.unix "systemd_units" {
              set_collectors = ["systemd"]
            }

            discovery.relabel "systemd_unit_targets" {
              targets = prometheus.exporter.unix.systemd_units.targets

              rule {
                target_label = "host"
                replacement  = ${quote hostName}
              }

              rule {
                target_label = "instance"
                replacement  = ${quote hostName}
              }
            }

            otelcol.receiver.prometheus "systemd_units" {
              output {
                metrics = [otelcol.exporter.otlp.egress.input]
              }
            }

            prometheus.scrape "systemd_units" {
              job_name   = "systemd-units"
              targets    = discovery.relabel.systemd_unit_targets.output
              forward_to = [otelcol.receiver.prometheus.systemd_units.receiver]
            }
          ''

          ''
            otelcol.receiver.loki "journal" {
              output {
                logs = [otelcol.exporter.otlp.egress.input]
              }
            }
          ''
        ]
        ++ [ journalSources ]
      )
  );
in
lib.mkIf cfg.enable {
  # Declare a static alloy user/group at host level. The companion
  # services nixling-otel-host-bridge (and per-VM relay sidecars)
  # need `SupplementaryGroups = [ "alloy" ]` to read sockets alloy
  # owns under /run/nixling/alloy. With DynamicUser=true, NixOS's
  # services.alloy would allocate a fresh UID/GID at each start and
  # the supplementary-group reference would be unresolvable at
  # service-stop time. Static account dodges that lifecycle issue.
  users.users.alloy = {
    isSystemUser = true;
    group = "alloy";
    home = "/var/lib/alloy";
    createHome = false;  # systemd StateDirectory= creates it
    description = "Grafana Alloy (nixling-managed static account)";
  };
  users.groups.alloy = { };

  services.alloy = {
    enable = true;
    configPath = alloyConfig;
  };

  systemd.services.alloy.serviceConfig = {
    DynamicUser = lib.mkForce false;
    User = lib.mkForce "alloy";
    Group = lib.mkForce "alloy";
    SupplementaryGroups = lib.mkAfter [ "adm" ];

    # alloy needs a writable working dir for its on-disk WAL +
    # remotecfg cache. NixOS sets WorkingDirectory=/var/lib/alloy
    # via StateDirectory; that still works with DynamicUser=false
    # as long as we declare the static account above (systemd
    # creates the dir + chowns to the User on first start).
    StateDirectory = lib.mkAfter [ "alloy" ];
    StateDirectoryMode = "0750";

    # alloy creates /run/nixling/alloy itself at service-start time,
    # owned by the static alloy UID/GID. The
    # nixling-otel-host-bridge sidecar runs After=alloy.service so
    # its ExecStartPre setfacl on this dir runs only once the dir
    # exists.
    RuntimeDirectory = lib.mkAfter [ "nixling/alloy" ];
    RuntimeDirectoryMode = "0710";
    RuntimeDirectoryPreserve = "yes";
  };

  # `nixling-otel-host-bridge.service` host singleton was deleted.
  # The OTel host bridge is now broker-spawned via
  # SpawnRunner{role: OtelHostBridge} with readiness gated by its
  # readiness predicate. The argv generator lives at
  # packages/nixling-host/src/otel_host_bridge_argv.rs;
  # the broker dispatcher at packages/nixling-priv-broker/src/runtime.rs
  # refuses bundle intent for non-obs VMs.
  # The systemd.tmpfiles.rules block below stays — those are the
  # documented stable socket name aliases consumed by Alloy + Grafana.

  # Keep the documented socket names stable for clients/docs while the
  # real Alloy-owned sockets live under /run/nixling/alloy/.
  systemd.tmpfiles.rules = lib.mkAfter [
    "L+ /run/nixling/host-otlp.sock - - - - ${hostOtlpSocket}"
    "L+ /run/nixling/host-egress.sock - - - - ${hostEgressSocket}"
  ];
}
