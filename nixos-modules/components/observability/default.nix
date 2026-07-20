# Realm observability composition.
{ config, lib, ... }:

let
  cfg = config.d2b.observability;
  rows = import ../../realm-observability-rows.nix {
    inherit config lib;
  };
in
{
  imports = [
    ./host.nix
  ];

  options.d2b._realmObservability = lib.mkOption {
    type = lib.types.attrs;
    default = { };
    internal = true;
    visible = false;
    description = "Canonical realm/workload observability resource rows.";
  };

  config = lib.mkIf cfg.enable {
    d2b._realmObservability = rows;

    d2b.realms.local-root = {
      path = "local-root";
      placement = "host-local";
      providers.runtime-local = {
        type = "runtime";
        implementationId = "cloud-hypervisor";
      };
      providers.observability-local = {
        type = "observability";
        implementationId = "local";
      };
      workloads.${cfg.vmName} = {
        providerRefs = {
          runtime = "runtime-local";
          observability = "observability-local";
        };
        autostart = true;
        config = {
          imports = [ ./stack.nix ];
          d2b.observability = {
            enable = true;
            vmName = cfg.vmName;
            retention = cfg.retention;
            grafana = cfg.grafana;
            signoz = cfg.signoz;
            transport.relayPackage = cfg.transport.relayPackage;
            ingress.sources = rows.ingressSources;
            alerts = cfg.alerts;
            hostName = config.networking.hostName;
          };
          microvm = {
            vcpu = lib.mkDefault 4;
            mem = lib.mkDefault 8192;
            volumes = lib.mkDefault [
              {
                image = "clickhouse.img";
                mountPoint = "/var/lib/clickhouse";
                size = 32768;
                fsType = "ext4";
                serial = "obs-clickhouse";
                direct = true;
              }
              {
                image = "zookeeper.img";
                mountPoint = "/var/lib/zookeeper";
                size = 2048;
                fsType = "ext4";
                serial = "obs-zookeeper";
                direct = true;
              }
              {
                image = "signoz.img";
                mountPoint = "/var/lib/signoz";
                size = 4096;
                fsType = "ext4";
                serial = "obs-signoz";
                direct = true;
              }
              {
                image = "signoz-otel.img";
                mountPoint = "/var/lib/signoz-otel-collector";
                size = 2048;
                fsType = "ext4";
                serial = "obs-otel";
                direct = true;
              }
            ];
          };
        };
      };
    };
  };
}
