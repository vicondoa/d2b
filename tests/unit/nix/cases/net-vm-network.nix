# Fixed Network Provider surface.
#
# Host networking is no longer materialised from env or manifest data. The
# Network resource is the only Nix authority; host effects are admitted and
# executed by the daemon and broker.
{ mkEval, lib, pkgs, flakeRoot, ... }:

let
  catalogShape = import ../../../../nixos-modules/generated/provider-catalog-shape.nix;

  catalogEntry = name:
    let
      digestFields = lib.listToAttrs (map
        (field: lib.nameValuePair field
          "sha256:${builtins.hashString "sha256" "${name}/${field}"}")
        catalogShape.digestFields);
      plainFields = lib.listToAttrs (map
        (field: lib.nameValuePair field "${name}/${field}")
        (lib.filter (field: !(builtins.elem field catalogShape.digestFields))
          catalogShape.fields));
    in digestFields // plainFields;

  artifact = name: type: {
    package = pkgs.writeText name name;
    inherit type;
    catalog = catalogEntry name;
  };

  provider = {
    network-local = {
      type = "Provider";
      spec.artifactId = "provider-network-local";
    };
  };

  network = name: lan: uplink: {
    type = "Network";
    spec = {
      providerRef = "Provider/network-local";
      lanCidr = lan;
      uplinkCidr = uplink;
      netVmSystemArtifactId = "net-vm-base";
    };
  };

  v3Fixture = { lib, ... }: {
    options = {
      d2b._index = lib.mkOption {
        type = lib.types.attrs;
        default = { };
        internal = true;
      };
      d2b._bundle = lib.mkOption {
        type = lib.types.attrs;
        default = { };
        internal = true;
      };
      d2b._providerCatalog = lib.mkOption {
        type = lib.types.attrs;
        default = { };
        internal = true;
      };
      d2b._artifactCatalogV3 = lib.mkOption {
        type = lib.types.attrs;
        default = { };
        internal = true;
      };
    };
    config = {
      d2b.artifacts = {
        provider-network-local = artifact "provider-network-local" "provider";
        net-vm-base = artifact "net-vm-base" "nixos-system";
      };
      d2b.zones.local-root.resources =
        provider
        // {
          work-net = network "work-net" "10.20.0.0/24" "192.0.2.0/30";
          gateway-net = (network "gateway-net" "10.30.0.0/24" "198.51.100.0/30") // {
            spec.externalAttachment = {
              parentInterface = "eno1";
              ipv4 = {
                method = "static";
                address = "203.0.113.2/24";
                gateway = "203.0.113.1";
                dns = [ "203.0.113.53" ];
              };
            };
          };
        };
      d2b.zones.personal = {
        parentZone = "local-root";
        resources = provider // {
          work-net = network "work-net" "10.40.0.0/24" "198.51.100.4/30";
        };
      };
    };
  };

  cfg = (mkEval [ v3Fixture ]).config;
  localNetworks = cfg.d2b._index.networks.byZone.local-root;
  personalNetworks = cfg.d2b._index.networks.byZone.personal;
  workNetwork = localNetworks.work-net;
  gatewayNetwork = localNetworks.gateway-net;
  networkSource = builtins.readFile
    (flakeRoot + "/packages/d2b-provider-network-local/nix/network.nix");
  netSource = builtins.readFile
    (flakeRoot + "/packages/d2b-provider-network-local/nix/net.nix");

  failureMessages = module:
    map (assertion: assertion.message)
      (lib.filter (assertion: !assertion.assertion)
        (mkEval [ v3Fixture module ]).config.assertions);

  rejects = needle: module:
    lib.any (message: lib.hasInfix needle message) (failureMessages module);
in
{
  "net-vm-network/v3-resource-canonical-spec" = {
    expr = localNetworks.work-net.spec;
    expected = {
      providerRef = "Provider/network-local";
      lanCidr = "10.20.0.0/24";
      uplinkCidr = "192.0.2.0/30";
      mtu = null;
      mssClamp = false;
      isolation.allowEastWest = false;
      routing.hostBlocklist = [
        "10.0.0.0/8"
        "169.254.0.0/16"
        "172.16.0.0/12"
        "192.168.0.0/16"
      ];
      dhcp = {
        domain = null;
        ignoreClientNames = true;
      };
      dns = {
        forwarders = [ ];
        cacheSize = 1000;
      };
      externalAttachment = null;
      mdns = {
        enable = false;
        reflector = true;
        dnsmasqLocal = false;
        dnsmasqLocalPort = 53530;
        publishWorkstation = false;
      };
      netVmNameOverride = null;
      netVmSystemArtifactId = "net-vm-base";
      attachments = [ ];
    };
  };

  "net-vm-network/v3-resource-bundle-omits-runtime-metadata" = {
    expr = {
      keys = lib.attrNames workNetwork;
      hasStorePath = lib.hasInfix "/nix/store/" (builtins.toJSON workNetwork);
    };
    expected = {
      keys = [ "name" "ref" "spec" "type" "zoneName" ];
      hasStorePath = false;
    };
  };

  "net-vm-network/v3-resource-host-admission-row-is-explicit" = {
    expr = builtins.head (lib.filter
      (row: row.zone == "local-root" && row.network == "work-net")
      cfg.d2b._resourceCompiler.networks.admission);
    expected = {
      zone = "local-root";
      network = "work-net";
      ref = "Network/work-net";
      lanCidr = "10.20.0.0/24";
      uplinkCidr = "192.0.2.0/30";
      attachmentRefs = [ ];
    };
  };

  "net-vm-network/v3-resource-same-name-zones-stay-distinct" = {
    expr = {
      local = localNetworks.work-net.ref;
      personal = personalNetworks.work-net.ref;
      zones = builtins.attrNames cfg.d2b._index.networks.byZone;
      zoneRows = [ workNetwork.zoneName personalNetworks.work-net.zoneName ];
    };
    expected = {
      local = "Network/work-net";
      personal = "Network/work-net";
      zones = [ "local-root" "personal" ];
      zoneRows = [ "local-root" "personal" ];
    };
  };

  "net-vm-network/v3-resource-cross-zone-cidr-overlap-rejected" = {
    expr = rejects "overlaps" {
      d2b.zones.personal.resources.work-net.spec.lanCidr =
        lib.mkForce "10.20.0.0/24";
    };
    expected = true;
  };

  "net-vm-network/v3-resource-gateway-preserved-and-dhcp-null" = {
    expr = {
      gateway = gatewayNetwork.spec.externalAttachment.ipv4.gateway;
      method = gatewayNetwork.spec.externalAttachment.ipv4.method;
      address = gatewayNetwork.spec.externalAttachment.ipv4.address;
      dhcp = gatewayNetwork.spec.externalAttachment.ipv4.dhcp or null;
    };
    expected = {
      gateway = "203.0.113.1";
      method = "static";
      address = "203.0.113.2/24";
      dhcp = null;
    };
  };

  "net-vm-network/v3-resource-isolation-default-remains-closed" = {
    expr = {
      allowEastWest = localNetworks.work-net.spec.isolation.allowEastWest;
      routing = localNetworks.work-net.spec.routing.hostBlocklist;
    };
    expected = {
      allowEastWest = false;
      routing = [
        "10.0.0.0/8"
        "169.254.0.0/16"
        "172.16.0.0/12"
        "192.168.0.0/16"
      ];
    };
  };

  "net-vm-network/host-module-has-no-retired-authority" = {
    expr = lib.all
      (needle: !(lib.hasInfix needle networkSource))
      [ "cfg.envs" "host.environments" "manifest" "route:env:" "netVmName" ];
    expected = true;
  };

  "net-vm-network/net-guest-keeps-dhcp-neutralizer" = {
    expr = lib.hasInfix "\"10-eth-dhcp\" = lib.mkForce" netSource;
    expected = true;
  };

  "net-vm-network/net-guest-has-no-network-desired-data" = {
    expr = lib.all
      (needle: !(lib.hasInfix needle netSource))
      [ "services.dnsmasq" "hostBlocklist" "attachments.json" "route:env:" ];
    expected = true;
  };
}
