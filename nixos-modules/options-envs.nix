# d2b.envs.<env>.* — isolated per-env networks. Each env is
# materialised by network.nix into two host bridges (`br-<env>-up`
# point-to-point host↔net-VM, `br-<env>-lan` net-VM↔workload-VMs),
# an auto-generated headless net VM (`sys-<env>-net`), NAT/firewall,
# and a per-env broker-spawned USBIP proxy. Workload
# VMs join an env by setting `d2b.vms.<name>.env = "<env>"` and
# `index = <N>`. Extracted from options.nix for reviewability.
{ lib, config, ... }:

let
  homeLanInterfaceNameType = lib.types.strMatching "^[A-Za-z0-9_.:-]{1,15}$";

  homeLanStaticAddressType = lib.types.submodule {
    freeformType = null;
    options = {
      address = lib.mkOption {
        type = lib.types.str;
        example = "192.168.1.50/24";
        description = ''
          Static IPv4 address, in CIDR notation, for the generated
          net VM's home-LAN interface when `address.mode = "static"`.
        '';
      };

      gateway = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "192.168.1.1";
        description = "Optional IPv4 default gateway for static home-LAN addressing.";
      };

      dns = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "192.168.1.1" ];
        description = "Optional DNS resolvers for static home-LAN addressing.";
      };
    };
  };

  homeLanAddressType = lib.types.submodule {
    freeformType = null;
    options = {
      mode = lib.mkOption {
        type = lib.types.enum [ "dhcp" "static" ];
        default = "dhcp";
        description = "How the generated net VM obtains its home-LAN address.";
      };

      static = lib.mkOption {
        type = lib.types.nullOr homeLanStaticAddressType;
        default = null;
        description = "Static address details used when `mode = \"static\"`.";
      };
    };
  };

  homeLanPortForwardType = lib.types.submodule {
    freeformType = null;
    options = {
      listenPort = lib.mkOption {
        type = lib.types.nullOr lib.types.port;
        default = null;
        example = 8443;
        description = "Home-LAN port opened on the generated net VM.";
      };

      targetVm = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "workstation";
        description = "Same-env workload VM that receives this port forward.";
      };

      targetPort = lib.mkOption {
        type = lib.types.nullOr lib.types.port;
        default = null;
        example = 443;
        description = "Port on `targetVm` that receives forwarded traffic.";
      };

      protocol = lib.mkOption {
        type = lib.types.nullOr (lib.types.enum [ "tcp" "udp" ]);
        default = null;
        example = "tcp";
        description = "Transport protocol for this port forward.";
      };

      sourceCidrs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "192.168.1.0/24" ];
        description = ''
          Optional home-LAN source CIDR allowlist for this forward. Entries
          must not overlap peer d2b env CIDRs.
        '';
      };
    };
  };
in

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

        homeLan = lib.mkOption {
          type = lib.types.submodule {
            freeformType = null;
            options = {
              enable = lib.mkEnableOption "home-LAN policy metadata for this env";

              attachment = lib.mkOption {
                type = lib.types.submodule {
                  freeformType = null;
                  options = {
                    enable = lib.mkEnableOption "a generated-net-VM home-LAN attachment";

                    hostInterface = lib.mkOption {
                      type = lib.types.nullOr homeLanInterfaceNameType;
                      default = null;
                      example = "eno1";
                      description = ''
                        Explicit host interface used for the generated net
                        VM's home-LAN attachment. Required when
                        `attachment.enable = true`.
                      '';
                    };

                    mode = lib.mkOption {
                      type = lib.types.enum [ "macvtap" ];
                      default = "macvtap";
                      description = "Attachment backend for net-VM home-LAN presence.";
                    };

                    address = lib.mkOption {
                      type = homeLanAddressType;
                      default = { };
                      description = "Addressing policy for the net VM's home-LAN interface.";
                    };
                  };
                };
                default = { };
                description = ''
                  Physical home-LAN attachment for the generated net VM. d2b
                  does not run a host Avahi daemon, host mDNS relay/proxy, or
                  open host UDP/5353 for this surface.
                '';
              };

              egress = lib.mkOption {
                type = lib.types.submodule {
                  freeformType = null;
                  options = {
                    enable = lib.mkEnableOption "home-LAN egress from this env's net VM";

                    allowedCidrs = lib.mkOption {
                      type = lib.types.listOf lib.types.str;
                      default = config.d2b.hostLanCidrs;
                      defaultText = lib.literalExpression "config.d2b.hostLanCidrs";
                      example = [ "192.168.1.0/24" ];
                      description = ''
                        Home-LAN CIDRs this env may reach through the
                        generated net VM. Entries must not overlap peer d2b
                        env CIDRs.
                      '';
                    };

                    masquerade = lib.mkOption {
                      type = lib.types.bool;
                      default = true;
                      description = "Whether the generated net VM should masquerade home-LAN egress.";
                    };
                  };
                };
                default = { };
                description = "Home-LAN egress policy for this env.";
              };

              portForwards = lib.mkOption {
                type = lib.types.listOf homeLanPortForwardType;
                default = [ ];
                example = lib.literalExpression ''
                  [
                    {
                      listenPort = 8443;
                      targetVm = "workstation";
                      targetPort = 443;
                      protocol = "tcp";
                      sourceCidrs = [ "192.168.1.0/24" ];
                    }
                  ]
                '';
                description = "Home-LAN port forwards terminated by the generated net VM.";
              };

              mdns = lib.mkOption {
                type = lib.types.submodule {
                  freeformType = null;
                  options = {
                    enable = lib.mkEnableOption "mDNS behaviour inside the generated net VM";

                    reflector.enable = lib.mkOption {
                      type = lib.types.bool;
                      default = true;
                      description = "Whether net-VM mDNS reflection is requested when mDNS is enabled.";
                    };

                    dnsmasqLocal.enable = lib.mkEnableOption "net-VM-local dnsmasq mDNS name handling";

                    publishWorkstation = lib.mkOption {
                      type = lib.types.bool;
                      default = false;
                      description = "Whether the generated net VM should publish workstation presence on the home LAN.";
                    };
                  };
                };
                default = { };
                description = ''
                  mDNS policy owned by the generated net VM. The host must not
                  run Avahi, relay/proxy mDNS, or open UDP/5353 for this option.
                '';
              };
            };
          };
          default = { };
          description = ''
            Home-LAN access policy for this env. The generated net VM owns
            any home-LAN presence and may receive its own LAN IP; the host does
            not become the mDNS or LAN-presence endpoint.
          '';
        };

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
