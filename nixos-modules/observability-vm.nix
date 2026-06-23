# Auto-declared observability stack VM.
#
# TODO: `auto-obs-vm`.
# When `nixling.observability.enable = true`, materialise the reserved
# observability env and the headless stack VM that will later host the
# single-host Grafana/Prometheus/Loki/Tempo sink.
{ config, lib, ... }:

let
  cfg = config.nixling.observability;
  obsIngressSources = config.nixling._index.observability.sources;
in
{
  config = lib.mkIf cfg.enable {
    nixling.envs.${cfg.env} = {
      lanSubnet = lib.mkDefault cfg.lanSubnet;
      uplinkSubnet = lib.mkDefault cfg.uplinkSubnet;
    };

    # `cfg.vmName` defaults to `sys-obs`, matching the framework's
    # `sys-<env>-<role>` namespace for auto-declared system VMs. The current
    # reserved-prefix exemption in assertions.nix only whitelists the per-env
    # net VMs; the matching carve-out for this observability VM belongs with
    # the transport-vsock assertions work, not in this module.
    nixling.vms.${cfg.vmName} = {
      env = lib.mkDefault cfg.env;
      index = lib.mkDefault cfg.index;
      autostart = lib.mkDefault true;

      graphics.enable = lib.mkDefault false;
      tpm.enable = lib.mkDefault false;
      usbip.yubikey = lib.mkDefault false;
      audio.enable = lib.mkDefault false;
      audit.enable = lib.mkDefault false;

      config = {
        imports = [
          ./components/observability/stack.nix
        ];
        nixling.observability = {
          retention = lib.mkDefault cfg.retention;
          grafana = lib.mkDefault cfg.grafana;
          signoz = lib.mkDefault cfg.signoz;
          transport.relayPackage = lib.mkDefault cfg.transport.relayPackage;
          ingress.sources = lib.mkDefault obsIngressSources;
          alerts = lib.mkDefault cfg.alerts;
          # Physical host the guests run on; stamped as
          # deployment.environment on ingested telemetry. Resolved from
          # the host's networking.hostName (this module evaluates in the
          # host config context).
          hostName = lib.mkDefault config.networking.hostName;
        };

        # SigNoz + ClickHouse is materially heavier than the retired
        # Grafana/Prometheus/Loki/Tempo stack. Keep these defaults
        # overrideable, but make the auto-declared VM viable out of the
        # box for a single-node telemetry store.
        microvm.vcpu = lib.mkDefault 4;
        microvm.mem = lib.mkDefault 8192;
        microvm.volumes = lib.mkDefault [
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
}
