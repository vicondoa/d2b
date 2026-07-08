# d2b.realms.<realm>.network — realm-native network declaration as the
# replacement public surface for d2b.envs.<env>.
#
# This file is imported as a fragment inside the d2b.realms.<realm>
# submodule (see options-realms.nix).  It extends the stub `network`
# block that was defined in options-realms.nix with the full
# bridge / subnet / uplink / externalNetwork / mDNS / port-forward
# shape that mirrors the runtime contract carried by d2b.envs.<env>.
#
# Runtime materialisation behaviour follows network.mode:
#   "none"        — no bridges, no net VM, no host network resources
#                   are claimed.  Safe default for metadata-only realm
#                   declarations.
#   "inherit-env" — the realm delegates network to an existing
#                   d2b.envs.<env> entry named by network.envs[0].
#                   Bridge lifecycle remains controlled by the env.
#   "declared"    — the realm OWNS the network declaration.  This is
#                   the v2-native path: the realm's network.* options
#                   supply the subnet / bridge / externalNetwork
#                   parameters and d2b materialises the bridges + net
#                   VM under a realm-derived name.
#   "external"    — the realm uses an externally-managed network.
#                   No d2b-managed bridges are created; only
#                   policy-metadata fields are meaningful.
#
# IMPORTANT: setting network.mode = "declared" and simultaneously
# retaining a d2b.envs entry with overlapping CIDRs is an error caught
# by assertions.nix.  Operators who already have a d2b.envs entry
# should keep mode = "none" (default) or "inherit-env" during the
# transition and only switch to "declared" when they are ready to let
# d2b remove the legacy env.
{ lib, config, name, ... }:

let
  realmId = config.id;

  externalNetworkPortForwardType = lib.types.submodule {
    freeformType = null;
    options = {
      protocol = lib.mkOption {
        type = lib.types.enum [ "tcp" "udp" ];
        default = "tcp";
        description = "Layer-4 protocol to forward from the net VM's external interface.";
      };

      listenPort = lib.mkOption {
        type = lib.types.port;
        example = 2222;
        description = "Port on the net VM's external interface.";
      };

      workload = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "laptop";
        description = ''
          Workload name in this realm that receives the forward.
          Use either this field or `targetIp`, not both.
        '';
      };

      targetIp = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = "Explicit workload-LAN target IP; use instead of workload for advanced routing.";
      };

      targetPort = lib.mkOption {
        type = lib.types.nullOr lib.types.port;
        default = null;
        example = 22;
        description = "Target port on the workload. Defaults to listenPort when null.";
      };

      sourceCidrs = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "192.168.1.0/24" ];
        description = "Optional source CIDR allowlist for this forward.";
      };
    };
  };
in
{
  options.network = {
    # -- Subnet / bridge shape -------------------------------------------

    lanSubnet = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "10.20.0.0/24";
      description = ''
        CIDR for the realm's workload LAN bridge.  The net VM
        takes the `.1` address; workload VMs receive their address
        from `.index`.  Must be a /24 ending in `.0`.

        Required when `network.mode = "declared"`.
      '';
    };

    uplinkSubnet = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "192.0.2.252/30";
      description = ''
        Point-to-point CIDR between the host and the realm's net VM.
        Host takes `.1`; net VM takes `.2`.  Must be a /30.

        Required when `network.mode = "declared"`.
      '';
    };

    mtu = lib.mkOption {
      type = lib.types.nullOr lib.types.int;
      default = null;
      description = "Override MTU for the realm's bridges, taps, and guest NICs. Null uses the OS default (1500).";
    };

    mssClamp = lib.mkEnableOption "TCP MSS clamping on the net VM's nftables forward chain (recommended over tunneled uplinks)";

    lan = {
      allowEastWest = lib.mkEnableOption "east-west traffic between workloads inside this realm (also requires d2b.site.allowUnsafeEastWest = true)";
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
        Destination CIDRs dropped by the net VM on forward from the
        workload LAN.  The framework augments this with
        `d2b.hostLanCidrs` and peer realm/env CIDR sets.
      '';
    };

    # -- External network ------------------------------------------------

    externalNetwork = {
      enable = lib.mkEnableOption "external network policy metadata for this realm";

      attachment = {
        enable = lib.mkEnableOption "a separate net-VM NIC on the upstream host LAN";

        interface = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "eno1";
          description = "Physical host interface attached to the net VM's external NIC.";
        };

        mode = lib.mkOption {
          type = lib.types.enum [ "macvtap" ];
          default = "macvtap";
          description = "Host attachment mode for the external NIC.";
        };

        macvtapMode = lib.mkOption {
          type = lib.types.enum [ "bridge" "private" "vepa" "passthru" ];
          default = "bridge";
          description = "macvtap/macvlan mode when attachment.mode = macvtap.";
        };

        macAddress = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "Optional fixed MAC for the net VM external NIC. Null derives a deterministic locally-administered address from the realm id.";
        };

        ipv4 = {
          method = lib.mkOption {
            type = lib.types.enum [ "dhcp" "static" ];
            default = "dhcp";
            description = "How the net VM configures external0 inside the guest.";
          };

          address = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "192.168.1.50/24";
            description = "Static IPv4 address with prefix when method = static.";
          };

          gateway = lib.mkOption {
            type = lib.types.nullOr lib.types.str;
            default = null;
            example = "192.168.1.1";
            description = "Optional static default gateway for external0.";
          };

          dns = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = [ ];
            example = [ "192.168.1.1" ];
            description = "Optional static DNS resolvers for external0.";
          };
        };
      };

      egress = {
        enable = lib.mkEnableOption "workload-initiated external network access NATed behind the net VM's external0 address";

        allowedCidrs = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ ];
          example = [ "192.168.1.0/24" ];
          description = "External network CIDRs reachable through the realm's net VM.";
        };

        masquerade = lib.mkOption {
          type = lib.types.bool;
          default = true;
          description = "Whether the net VM should masquerade external egress.";
        };
      };

      portForwards = lib.mkOption {
        type = lib.types.listOf externalNetworkPortForwardType;
        default = [ ];
        description = "Explicit DNAT rules from the net VM's external0 to workloads in this realm.";
      };

      mdns = {
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
          description = "Whether the net VM should publish workstation presence on the external network.";
        };

        dnsmasqLocal.port = lib.mkOption {
          type = lib.types.port;
          default = 53530;
          description = "Loopback UDP/TCP port for the net-VM-local `.local` DNS bridge when dnsmasqLocal.enable is true.";
        };
      };
    };

    # -- Net VM name / extra config --------------------------------------

    netVmName = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      defaultText = lib.literalExpression "\"sys-<realm-id>-net\"";
      description = ''
        Optional override for the auto-declared net VM name.  Defaults
        to `sys-<realm-id>-net` when mode = "declared".
      '';
    };

    extraNetConfig = lib.mkOption {
      type = lib.types.unspecified;
      default = { };
      example = lib.literalExpression ''
        { networking.hostName = "realm-gw"; }
      '';
      description = "Extra NixOS module merged into the realm's auto-declared net VM configuration.";
    };

    # -- UI color --------------------------------------------------------

    ui.accentColor = lib.mkOption {
      type = lib.types.nullOr (lib.types.strMatching "^#[0-9a-fA-F]{6}$");
      default = null;
      example = "#ffa500";
      description = "Optional compositor-agnostic accent color for this realm's network context.";
    };
  };
}
