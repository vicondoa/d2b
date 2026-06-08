# Per-workload observability guest component.
#
# Imported into the GUEST config by host.nix whenever a VM sets
# `nixling.vms.<name>.observability.enable = true`.
{ lib, pkgs, config, ... }:

let
  cfg = config.nixling.observability;
  alloyPort = 12345;
  alloyRuntimeDir = "/run/nixling/alloy";
  guestOtlpSocket = "${alloyRuntimeDir}/otlp.sock";
  guestOtlpEgressSocket = "${alloyRuntimeDir}/otlp-egress.sock";
  quote = builtins.toJSON;

  alloyConfig = lib.concatStringsSep "\n\n" (
    [
      ''
        otelcol.exporter.otlp "vsock" {
          client {
            endpoint = "unix://${guestOtlpEgressSocket}"
            compression = "none"

            tls {
              insecure = true
            }
          }
        }

        otelcol.receiver.otlp "local" {
          grpc {
            endpoint  = "${guestOtlpSocket}"
            transport = "unix"
          }

          output {
            metrics = [otelcol.exporter.otlp.vsock.input]
            logs    = [otelcol.exporter.otlp.vsock.input]
            traces  = [otelcol.exporter.otlp.vsock.input]
          }
        }
      ''

      ''
        discovery.relabel "self_targets" {
          targets = [{
            "__address__" = "127.0.0.1:${toString alloyPort}",
          }]

          rule {
            target_label = "vm"
            replacement  = ${quote cfg.identity.vmName}
          }

          rule {
            target_label = "env"
            replacement  = ${quote cfg.identity.envName}
          }

          rule {
            target_label = "role"
            replacement  = "workload"
          }

          rule {
            target_label = "instance"
            replacement  = ${quote cfg.identity.vmName}
          }
        }

        otelcol.receiver.prometheus "self" {
          output {
            metrics = [otelcol.exporter.otlp.vsock.input]
          }
        }

        prometheus.scrape "self" {
          job_name   = "nixling-vm-telemetry"
          targets    = discovery.relabel.self_targets.output
          forward_to = [otelcol.receiver.prometheus.self.receiver]
        }
      ''
    ]
    ++ lib.optional cfg.scrapeNodeMetrics ''
      prometheus.exporter.unix "node" {
      }

      discovery.relabel "node_targets" {
        targets = prometheus.exporter.unix.node.targets

        rule {
          target_label = "vm"
          replacement  = ${quote cfg.identity.vmName}
        }

        rule {
          target_label = "env"
          replacement  = ${quote cfg.identity.envName}
        }

        rule {
          target_label = "role"
          replacement  = "workload"
        }

        rule {
          target_label = "instance"
          replacement  = ${quote cfg.identity.vmName}
        }
      }

      otelcol.receiver.prometheus "node" {
        output {
          metrics = [otelcol.exporter.otlp.vsock.input]
        }
      }

      prometheus.scrape "node" {
        job_name   = "nixling-vm-node"
        targets    = discovery.relabel.node_targets.output
        forward_to = [otelcol.receiver.prometheus.node.receiver]
      }
    ''
    ++ lib.optional cfg.scrapeJournal ''
      otelcol.receiver.loki "journal" {
        output {
          logs = [otelcol.exporter.otlp.vsock.input]
        }
      }

      loki.source.journal "journal" {
        forward_to = [otelcol.receiver.loki.journal.receiver]
        labels = {
          job  = "guest-journal",
          vm   = ${quote cfg.identity.vmName},
          env  = ${quote cfg.identity.envName},
          role = "workload",
        }
      }
    ''
  );
in
{
  options.nixling.observability = {
    scrapeJournal = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether the guest Alloy agent scrapes this VM's journald
        stream. Intended to be populated by
        `nixling.vms.<name>.observability.scrapeJournal`.
      '';
    };

    scrapeNodeMetrics = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether the guest Alloy agent scrapes this VM's node/system
        metrics. Intended to be populated by
        `nixling.vms.<name>.observability.scrapeNodeMetrics`.
      '';
    };

    identity = {
      vmName = lib.mkOption {
        type = lib.types.str;
        default = config.networking.hostName;
        description = ''
          Internal VM label injected into guest telemetry targets.
        '';
      };

      envName = lib.mkOption {
        type = lib.types.str;
        default = "none";
        description = ''
          Internal env label injected into guest telemetry targets.
        '';
      };
    };

    transport.relayPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.socat;
      description = ''
        Package providing the guest-side observability relay binary.
        Defaults to `pkgs.socat`.
      '';
    };
  };

  config = {
    microvm.hypervisor = lib.mkDefault "cloud-hypervisor";

    # Static alloy user/group inside the workload VM. The
    # nixling-otel-vsock-out sidecar runs as User=alloy and needs
    # the user to exist outside of alloy.service's lifecycle.
    users.users.alloy = {
      isSystemUser = true;
      group = "alloy";
      home = "/var/lib/alloy";
      createHome = false;
      description = "Grafana Alloy (nixling-managed static account)";
    };
    users.groups.alloy = { };

    services.alloy = {
      enable = true;
      extraFlags = [ "--server.http.listen-addr=127.0.0.1:${toString alloyPort}" ];
    };
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
    environment.etc."alloy/config.alloy".text = alloyConfig;

    # Keep the documented /run/nixling/*.sock paths stable for clients,
    # but place the actual Alloy-owned sockets under a private subdirectory.
    # /run/nixling/alloy itself is created by alloy.service's
    # `RuntimeDirectory=nixling/alloy` directive (set above), not via
    # tmpfiles, because we want it to vanish on service stop.
    systemd.tmpfiles.rules = [
      "d /run/nixling 0755 root root -"
      "L+ /run/nixling/otlp.sock - - - - ${guestOtlpSocket}"
      "L+ /run/nixling/otlp-egress.sock - - - - ${guestOtlpEgressSocket}"
    ];

    systemd.services.nixling-otel-vsock-out = {
      description = "Bridge guest OTLP UDS to host vsock port 14317.";
      wantedBy = [ "multi-user.target" ];
      after = [ "alloy.service" ];
      bindsTo = [ "alloy.service" ];
      restartIfChanged = false;

      serviceConfig = {
        User = "alloy";
        Group = "alloy";
        ExecStartPre = [
          # Clean up any stale UNIX-LISTEN socket from a prior crashed
          # instance. socat's own unlink-early option segfaulted in
          # socat 1.8.1.1 under our service hardening; rm -f is
          # simpler and unambiguous. Runs with privilege (+) so it
          # is not subject to ProtectSystem=strict on the service.
          "+${pkgs.coreutils}/bin/rm -f ${guestOtlpEgressSocket}"
        ];
        ExecStart = "${cfg.transport.relayPackage}/bin/socat -d -d UNIX-LISTEN:${guestOtlpEgressSocket},fork,max-children=16,reuseaddr,mode=0660 VSOCK-CONNECT:2:14317";

        Restart = "on-failure";
        RestartSec = "3s";
        StartLimitIntervalSec = "300s";
        StartLimitBurst = 20;

        # alloy user is declared statically above; DynamicUser=false
        # so the User= directive is honored.
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
