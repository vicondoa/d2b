# d2b.envs.<env>.* — isolated per-env networks. Each env is
# materialised by network.nix into two host bridges (`br-<env>-up`
# point-to-point host↔net-VM, `br-<env>-lan` net-VM↔workload-VMs),
# an auto-generated headless net VM (`sys-<env>-net`), NAT/firewall,
# and a per-env broker-spawned USBIP proxy. Workload
# VMs join an env by setting `d2b.vms.<name>.env = "<env>"` and
# `index = <N>`. Extracted from options.nix for reviewability.
{ lib, ... }:

{
  options.d2b.envs = lib.mkOption {
    description = ''
      Isolated per-env networks. Each env owns two bridges
      (`br-<name>-up` and `br-<name>-lan`), an auto-declared headless
      net VM (`sys-<name>-net`) that NATs and firewalls the LAN, a
      dnsmasq DHCP/DNS server on the LAN, and a
      broker-spawned USBIP proxy on the host bound to
      the uplink IP.

      Workload VMs reference an env via `d2b.vms.<vm>.env`.
    '';
    default = { };
    type = lib.types.attrsOf (lib.types.submodule ({ name, ... }: {
      options = {
        enable = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Whether to materialise this env's bridges + net VM.";
        };

        lanSubnet = lib.mkOption {
          type = lib.types.str;
          example = "10.20.0.0/24";
          description = ''
            CIDR for the env's workload LAN bridge. The net VM
            takes `.1`; workload VMs get `.<index>` via dnsmasq
            host-reservations. Host has NO interface on this bridge.
            Must be a /24 with the network address ending in `.0`.
          '';
        };

        uplinkSubnet = lib.mkOption {
          type = lib.types.str;
          example = "192.0.2.252/30";
          description = ''
            Point-to-point CIDR between the host and the net VM.
            Host takes `.1`, net VM takes `.2`. The per-env usbipd
            proxy (`sys-<env>-usbipd`/`proxy` runner) binds to the
            host's `.1` here. Must be a /30. RFC 5737 reserves
            192.0.2.0/24, 198.51.100.0/24 and 203.0.113.0/24 as
            documentation ranges; pick a /30 inside one of those if
            you want addresses that visibly belong to d2b.
          '';
        };

        mtu = lib.mkOption {
          type = lib.types.nullOr lib.types.int;
          default = null;
          description = "Override MTU for the env's bridges, taps, and guest NICs. Leave null for the default (1500).";
        };

        mssClamp = lib.mkEnableOption "TCP MSS clamping on the net VM's nftables forward chain (recommended when running over a tunneled uplink)";

        lan.allowEastWest = lib.mkEnableOption "east-west traffic between workload VMs in this env (default: isolated; also requires d2b.site.allowUnsafeEastWest = true)";

        ui.accentColor = lib.mkOption {
          type = lib.types.nullOr (lib.types.strMatching "^#[0-9a-fA-F]{6}$");
          default = null;
          example = "#ffa500";
          description = ''
            Optional compositor-agnostic accent color for this d2b env,
            as a six-digit CSS hex color (`#rrggbb`). When null, d2b
            derives a deterministic color from the env name. Resolved UI
            color artifacts normalize the value to lowercase.
          '';
        };

        netName = lib.mkOption {
          type = lib.types.str;
          default = "sys-${name}-net";
          description = ''
            VM name under `d2b.vms.<netName>` for the
            auto-declared net VM. Defaults to `sys-<env>-net`.
          '';
        };

        hostBlocklist = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [
            "10.0.0.0/8"
            "172.16.0.0/12"
            "192.168.0.0/16"
            "169.254.0.0/16"
          ];
          description = ''
            Destination CIDRs the net VM DROPs on forward from
            the LAN. Intent: workload VMs must not reach the host's
            primary-LAN IP, peer envs, or other RFC1918 services.
            The framework automatically augments this with
            `d2b.hostLanCidrs` and every other env's LAN/uplink
            CIDR. Carve-outs (intra-env LAN, USBIP-to-host-uplink-IP)
            are evaluated before this list.
          '';
        };

        homeLan = {
          enable = lib.mkEnableOption "a home-LAN-facing interface on this env's generated net VM";

          bridge = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "br-home";
            description = "Host bridge or macvtap parent presented to the net VM as its home-LAN-facing NIC.";
          };

          address = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "192.168.1.2/24";
            description = "Static IPv4 address assigned to the net VM's deterministic home-LAN interface.";
          };

          egress.allowCidrs = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            example = [ "192.168.1.53/32" ];
            description = "Home-LAN destinations workload VMs may reach before the normal host blocklist drops apply.";
          };

          portForwards = lib.mkOption {
            type = lib.types.listOf (lib.types.submodule {
              options = {
                vm = lib.mkOption {
                  type = lib.types.str;
                  description = "Workload VM receiving the forwarded home-LAN traffic.";
                };
                port = lib.mkOption {
                  type = lib.types.port;
                  description = "TCP/UDP port to forward.";
                };
                protocol = lib.mkOption {
                  type = lib.types.enum [ "tcp" "udp" ];
                  default = "tcp";
                  description = "Transport protocol for this forward.";
                };
              };
            });
            default = [ ];
            description = "Explicit home-LAN DNAT rules into workload VMs.";
          };

          mdns = {
            enable = lib.mkEnableOption "mDNS reflection between the env LAN and the home LAN";

            dnsmasqLocal.enable = lib.mkEnableOption "dnsmasq forwarding for .local lookups to mDNS";
          };
        };

        extraNetConfig = lib.mkOption {
          type = lib.types.unspecified;
          default = { };
          example = lib.literalExpression ''
            { networking.hostName = "example-gw"; }
          '';
          description = ''
            Extra NixOS module merged into the auto-declared net
            VM's configuration. Use for per-env overrides
            (hostname, ssh keys, extra dnsmasq options, etc).
          '';
        };
      };
    }));
  };
}
