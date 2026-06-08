# Host-side observability component.
#
# Wave-1 todo: `component-host`.
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

  mkJournalSource =
    {
      label,
      unit,
      vm,
      env,
    }:
    ''
      loki.source.journal "${label}" {
        forward_to = [otelcol.receiver.loki.journal.receiver]
        matches    = "_SYSTEMD_UNIT=${unit}"
        labels = {
          host = ${quote hostName},
          unit = ${quote unit},
          vm   = ${quote vm},
          env  = ${quote env},
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
      })
      (mkJournalSource {
        label = "journal_usbipd_nixling_service";
        unit = "usbipd-nixling.service";
        vm = "host";
        env = "host";
      })
    ]
    ++ lib.optional cfg.ch.exporter.enable (mkJournalSource {
      label = "journal_nixling_ch_exporter_service";
      unit = "nixling-ch-exporter.service";
      vm = "host";
      env = cfg.env;
    })
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
        ++ lib.optional cfg.ch.exporter.enable ''
          otelcol.receiver.prometheus "host_ch_exporter" {
            output {
              metrics = [otelcol.exporter.otlp.egress.input]
            }
          }

          prometheus.scrape "host_ch_exporter" {
            job_name = "nixling-ch-exporter"
            targets = [{
              "__address__" = "127.0.0.1:${toString cfg.ch.exporter.listenPort}",
              "host"        = ${quote hostName},
              "instance"    = ${quote hostName},
            }]
            forward_to = [otelcol.receiver.prometheus.host_ch_exporter.receiver]
          }
        ''
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

  systemd.services.nixling-otel-host-bridge = {
    description = "Host OTLP bridge into the observability VM vsock backend";
    wantedBy = [ "multi-user.target" ];
    after = [ "microvm@${cfg.vmName}.service" "alloy.service" ];
    bindsTo = [ "microvm@${cfg.vmName}.service" "alloy.service" ];
    restartIfChanged = false;
    startLimitBurst = 20;
    startLimitIntervalSec = 300;

    serviceConfig = {
      Type = "exec";
      User = lib.mkForce "nixling-otel-bridge";
      # Run with primary group `alloy` so socat creates host-egress.sock
      # with the right group ownership at bind time. socat's
      # `group=alloy` ExecStart option would have done the same thing
      # via a post-bind chown(2), but with `CapabilityBoundingSet=""`
      # the bridge has no CAP_CHOWN and socat 1.8.1.1 segfaults
      # instead of returning EPERM. Setting the primary group sidesteps
      # the chown entirely.
      Group = lib.mkForce "alloy";
      SupplementaryGroups = lib.mkForce [ ];
      ExecStartPre = [
        "+${pkgs.acl}/bin/setfacl -m u:nixling-otel-bridge:rwx ${alloyRuntimeDir}"
        # Clean up any stale listen socket from a prior crashed
        # instance. socat does not unlink existing UNIX-LISTEN paths
        # before binding (the unlink-early option triggered a
        # segfault in socat 1.8.1.1 in our deployment), so we use a
        # plain rm -f as a privileged ExecStartPre. The `+` runs
        # without ProtectSystem=strict scoping so it can write into
        # the alloy RuntimeDirectory regardless of unit hardening.
        "+${pkgs.coreutils}/bin/rm -f ${hostEgressSocket}"
        # NOTE: do NOT `test -S ${obsOtlpVsockHostSocket}` here.
        # Cloud-Hypervisor creates the per-port host UDS lazily on
        # the first connect from the host side; the file does not
        # exist before. Pre-checking it would chicken-and-egg with
        # this bridge being the thing that first connects. The
        # ExecStart socat will get ENOENT once if CH itself isn't
        # ready yet, and systemd's Restart=on-failure backs us off.
        # We DO check the per-VM CH base vsock UDS which is always
        # present once microvm@<vm>.service is active.
        "${pkgs.coreutils}/bin/test -S ${obsVsockHostSocket}"
      ];
      # OTLP push path on the bridge:
      #   host alloy → UNIX-LISTEN host-egress.sock → socat (this) →
      #   EXEC nixling-ch-vsock-connect <stack-base> 14317 →
      #   CH textual protocol on stack VM's vsock base UDS →
      #   stack VM's vsock 14317 (LISTENed by stack VM's socat).
      #
      # Why EXEC instead of UNIX-CONNECT:<base>_14317? CH only
      # creates `<base>_<port>` host UDS files lazily for the
      # GUEST→HOST direction (when a guest does vsock connect).
      # For HOST→GUEST, you must use the textual protocol on the
      # base UDS — `CONNECT <port>\n` / `OK <buf>\n` / bytes. See
      # nixling-ch-vsock-connect.nix for the protocol implementation.
      ExecStart = ''
        ${cfg.transport.relayPackage}/bin/socat -d -d \
          UNIX-LISTEN:${hostEgressSocket},fork,reuseaddr,mode=0660 \
          EXEC:"${chVsockConnect}/bin/nixling-ch-vsock-connect ${obsVsockHostSocket} ${toString obsOtlpPort}"
      '';
      Restart = "on-failure";
      RestartSec = "3s";
      DynamicUser = false;
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
      PrivateDevices = true;
      RestrictAddressFamilies = [ "AF_UNIX" ];
      SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ];
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      # ProtectSystem=strict makes / read-only except for these:
      #   - /run/nixling/alloy/   socat UNIX-LISTEN binds host-egress.sock here
      #   - /var/lib/nixling/vms/<obsVm>/  CH lazily creates vsock.sock_<port>
      #     here on the first host UNIX-CONNECT, so socat needs write access
      ReadWritePaths = [
        alloyRuntimeDir
        (builtins.dirOf obsOtlpVsockHostSocket)
      ];
    };
  };

  # Keep the documented socket names stable for clients/docs while the
  # real Alloy-owned sockets live under /run/nixling/alloy/.
  systemd.tmpfiles.rules = lib.mkAfter [
    "L+ /run/nixling/host-otlp.sock - - - - ${hostOtlpSocket}"
    "L+ /run/nixling/host-egress.sock - - - - ${hostEgressSocket}"
  ];
}
