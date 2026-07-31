# Network resource validation and compilation.
{ config, lib, ... }:

let
  cfg = config.d2b;
  d2bLib = import ./lib.nix { inherit lib; };

  resourceRefPattern =
    "^(Host|Guest)/[a-z][a-z0-9-]{0,62}$";
  tokenPattern = "^[a-z][a-z0-9-]{0,62}$";
  ifNamePattern = "^[A-Za-z0-9_-]{1,15}$";
  macPattern = "^([0-9a-f]{2}:){5}[0-9a-f]{2}$";

  executionPolicyDefaultFields = [
    "defaultDomain"
    "allowedDomains"
    "defaultUserRef"
    "budget"
    "networkAttachments"
    "deviceAttachments"
    "volumeAttachmentDefaults"
  ];

  networkSpecFields = [
    "providerRef"
    "updatePolicy"
    "provider"
    "lanCidr"
    "uplinkCidr"
    "mtu"
    "mssClamp"
    "isolation"
    "routing"
    "dhcp"
    "dns"
    "externalAttachment"
    "mdns"
    "netVmNameOverride"
    "netVmSystemArtifactId"
    "attachments"
  ];

  defaultHostBlocklist = [
    "10.0.0.0/8"
    "169.254.0.0/16"
    "172.16.0.0/12"
    "192.168.0.0/16"
  ];

  networkDefaults = {
    mtu = null;
    mssClamp = false;
    isolation.allowEastWest = false;
    routing.hostBlocklist = defaultHostBlocklist;
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
    attachments = [ ];
  };

  attrOr = attrs: name: fallback:
    if builtins.isAttrs attrs && builtins.hasAttr name attrs
    then attrs.${name}
    else fallback;

  exactKeys = allowed: value:
    builtins.isAttrs value
    && lib.all (key: builtins.elem key allowed) (lib.attrNames value);

  validOctet = value:
    (value == "0" || !lib.hasPrefix "0" value)
    && lib.toInt value <= 255;

  ipv4Octets = value:
    let matched =
      if builtins.isString value
      then builtins.match "^([0-9]{1,3})\\.([0-9]{1,3})\\.([0-9]{1,3})\\.([0-9]{1,3})$" value
      else null;
    in
    if matched != null && lib.all validOctet matched then matched else null;

  validIpv4 = value: ipv4Octets value != null;

  validCidr = requiredPrefix: value:
    let
      parts = if builtins.isString value then lib.splitString "/" value else [ ];
      address = if lib.length parts == 2 then builtins.head parts else null;
      prefix = if lib.length parts == 2 then lib.last parts else null;
    in
    prefix == toString requiredPrefix && validIpv4 address;

  validAnyCidr = value:
    let
      parts = if builtins.isString value then lib.splitString "/" value else [ ];
      address = if lib.length parts == 2 then builtins.head parts else null;
      prefix = if lib.length parts == 2 then lib.last parts else null;
    in
    address != null
    && builtins.match "^[0-9]{1,2}$" prefix != null
    && lib.toInt prefix <= 32
    && validCidr (lib.toInt prefix) value;

  validLanCidr = value:
    validCidr 24 value
    && lib.last (ipv4Octets (builtins.head (lib.splitString "/" value))) == "0";

  validUplinkCidr = value:
    let
      address = if validCidr 30 value then builtins.head (lib.splitString "/" value) else null;
      octets = if address != null then ipv4Octets address else null;
    in
    octets != null && lib.mod (lib.toInt (lib.last octets)) 4 == 0;

  validMac = value:
    value == null
    || (builtins.isString value
      && builtins.match macPattern value != null
      && lib.mod (lib.fromHexString (builtins.substring 0 2 value)) 2 == 0);

  parseRef = ref:
    let parts = if builtins.isString ref then lib.splitString "/" ref else [ ];
    in if lib.length parts == 2 then {
      type = builtins.elemAt parts 0;
      name = builtins.elemAt parts 1;
    } else null;

  resolvesAs = resources: acceptedTypes: ref:
    let parsed = parseRef ref;
    in parsed != null
      && builtins.elem parsed.type acceptedTypes
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == parsed.type;

  providerFor = resources: providerRef:
    let parsed = parseRef providerRef;
    in if parsed != null
      && parsed.type == "Provider"
      && builtins.hasAttr parsed.name resources
      && resources.${parsed.name}.type == "Provider"
    then resources.${parsed.name}
    else null;

  artifactFor = artifactId:
    if builtins.isString artifactId && builtins.hasAttr artifactId cfg.artifacts
    then cfg.artifacts.${artifactId}
    else null;

  networks = lib.concatMap
    (zoneName:
      let zone = cfg.zones.${zoneName};
      in lib.mapAttrsToList
        (name: resource: {
          inherit zoneName zone name resource;
          spec = resource.spec;
          path = "d2b.zones.${zoneName}.resources.${name}";
        })
        (lib.filterAttrs (_: resource: resource.type == "Network") zone.resources))
    (lib.sort lib.lessThan (lib.attrNames cfg.zones));

  networkCidrs = lib.concatMap
    (network:
      lib.optionals (validLanCidr (attrOr network.spec "lanCidr" null)) [ {
        inherit (network) zoneName name path;
        field = "lanCidr";
        cidr = network.spec.lanCidr;
      } ]
      ++ lib.optionals (validUplinkCidr (attrOr network.spec "uplinkCidr" null)) [ {
        inherit (network) zoneName name path;
        field = "uplinkCidr";
        cidr = network.spec.uplinkCidr;
      } ])
    networks;

  unorderedPairs = values:
    let count = lib.length values;
    in lib.concatMap
      (index: lib.genList
        (offset: {
          left = lib.elemAt values index;
          right = lib.elemAt values (index + offset + 1);
        })
        (count - index - 1))
      (lib.genList (index: index) count);

  cidrOverlaps = lib.filter
    (pair:
      pair.left.zoneName == pair.right.zoneName
      && d2bLib.cidrOverlaps pair.left.cidr pair.right.cidr)
    (unorderedPairs networkCidrs);

  crossZoneExternalClaims = lib.filter
    (pair:
      pair.left.zoneName != pair.right.zoneName
      && pair.left.parentInterface == pair.right.parentInterface
      && pair.left.macvtapMode == "bridge"
      && pair.right.macvtapMode == "bridge")
    (unorderedPairs (lib.concatMap
      (network:
        let external = attrOr network.spec "externalAttachment" null;
        in lib.optional (builtins.isAttrs external && external ? parentInterface) {
          inherit (network) zoneName name path;
          inherit (external) parentInterface;
          macvtapMode = attrOr external "macvtapMode" "bridge";
        })
      networks));

  canonicalSpec = spec:
    lib.recursiveUpdate networkDefaults
      (builtins.removeAttrs spec executionPolicyDefaultFields);

  compiledNetworks = map
    (network: {
      inherit (network) zoneName name;
      type = "Network";
      ref = "Network/${network.name}";
      spec = canonicalSpec network.spec;
    })
    networks;

  compiledByZone = lib.foldl'
    (result: network:
      result // {
        ${network.zoneName} = (result.${network.zoneName} or { }) // {
          ${network.name} = network;
        };
      })
    { }
    compiledNetworks;

  portForwardAssertions = network: external:
    lib.flatten (lib.imap0
      (index: forward:
        let
          where = "${network.path}.spec.externalAttachment.portForwards.${toString index}";
          targetRef = attrOr forward "targetRef" null;
          targetIp = attrOr forward "targetIp" null;
          sourceCidrs = attrOr forward "sourceCidrs" [ ];
        in [
          {
            assertion = exactKeys
              [ "protocol" "listenPort" "targetRef" "targetIp" "targetPort" "sourceCidrs" ]
              forward;
            message = "${where} contains an unsupported field. Remove every field except protocol, listenPort, targetRef, targetIp, targetPort, and sourceCidrs.";
          }
          {
            assertion = builtins.elem (attrOr forward "protocol" null) [ "tcp" "udp" ]
              && builtins.isInt (attrOr forward "listenPort" null)
              && attrOr forward "listenPort" 0 >= 1
              && attrOr forward "listenPort" 0 <= 65535
              && builtins.isInt (attrOr forward "targetPort" null)
              && attrOr forward "targetPort" 0 >= 1
              && attrOr forward "targetPort" 0 <= 65535;
            message = "${where} must declare tcp or udp and nonzero 16-bit listen and target ports.";
          }
          {
            assertion = (targetRef == null) != (targetIp == null)
              && (targetRef == null || resolvesAs network.zone.resources [ "Host" "Guest" ] targetRef)
              && (targetIp == null || validIpv4 targetIp);
            message = "${where} must select exactly one same-Zone targetRef or IPv4 targetIp.";
          }
          {
            assertion = builtins.isList sourceCidrs
              && lib.length sourceCidrs <= 64
              && lib.all validAnyCidr sourceCidrs
              && lib.all
                (source: lib.all
                  (row: row.zoneName != network.zoneName || row.field != "lanCidr"
                    || !d2bLib.cidrOverlaps source row.cidr)
                  networkCidrs)
                sourceCidrs;
            message = "${where}.sourceCidrs must be bounded IPv4 CIDRs disjoint from Zone LAN CIDRs.";
          }
        ])
      (attrOr external "portForwards" [ ]));

  networkAssertions = network:
    let
      spec = canonicalSpec network.spec;
      resources = network.zone.resources;
      attachments = attrOr spec "attachments" [ ];
      indexes = map (attachment: attrOr attachment "index" null) attachments;
      isolation = attrOr spec "isolation" { };
      routing = attrOr spec "routing" { };
      dhcp = attrOr spec "dhcp" { };
      dns = attrOr spec "dns" { };
      mdns = attrOr spec "mdns" { };
      external = attrOr spec "externalAttachment" null;
      provider = providerFor resources (attrOr spec "providerRef" null);
      providerArtifact = artifactFor
        (if provider != null then attrOr provider.spec "artifactId" null else null);
      systemArtifact = artifactFor (attrOr spec "netVmSystemArtifactId" null);
      allowedCidrs =
        if builtins.isAttrs external
        then attrOr (attrOr external "egress" { }) "allowedCidrs" [ ]
        else [ ];
      netVmName = attrOr spec "netVmNameOverride" null;
    in
    [
      {
        assertion = exactKeys (networkSpecFields ++ executionPolicyDefaultFields) network.spec;
        message = "${network.path}.spec contains an unsupported field. Remove fields not declared by the Network ResourceSpec schema.";
      }
      {
        assertion = attrOr spec "providerRef" null != null
          && provider != null
          && providerArtifact != null
          && providerArtifact.type == "provider";
        message = "${network.path}.spec.providerRef must resolve to a same-Zone Provider backed by a provider artifact.";
      }
      {
        assertion = builtins.isString (attrOr spec "netVmSystemArtifactId" null)
          && builtins.match tokenPattern spec.netVmSystemArtifactId != null;
        message = "${network.path}.spec.netVmSystemArtifactId is required and must be a bounded artifact ID.";
      }
      {
        assertion = systemArtifact != null && systemArtifact.type == "nixos-system";
        message = "${network.path}.spec.netVmSystemArtifactId must resolve to a nixos-system artifact.";
      }
      {
        assertion = validLanCidr (attrOr spec "lanCidr" null);
        message = "${network.path}.spec.lanCidr must be a canonical IPv4 /24 network ending in .0.";
      }
      {
        assertion = validUplinkCidr (attrOr spec "uplinkCidr" null);
        message = "${network.path}.spec.uplinkCidr must be a canonical IPv4 /30 network.";
      }
      {
        assertion = !(validLanCidr (attrOr spec "lanCidr" null)
          && validUplinkCidr (attrOr spec "uplinkCidr" null))
          || !d2bLib.cidrOverlaps spec.lanCidr spec.uplinkCidr;
        message = "${network.path}.spec lanCidr and uplinkCidr must not overlap.";
      }
      {
        assertion = attrOr spec "mtu" null == null
          || (builtins.isInt spec.mtu && spec.mtu >= 576 && spec.mtu <= 9216);
        message = "${network.path}.spec.mtu must be null or an integer from 576 through 9216.";
      }
      {
        assertion = builtins.isBool spec.mssClamp;
        message = "${network.path}.spec.mssClamp must be a boolean.";
      }
      {
        assertion = exactKeys [ "allowEastWest" ] isolation
          && builtins.isBool (attrOr isolation "allowEastWest" false);
        message = "${network.path}.spec.isolation must contain only the allowEastWest boolean.";
      }
      {
        assertion = exactKeys [ "hostBlocklist" ] routing
          && builtins.isList (attrOr routing "hostBlocklist" [ ])
          && lib.length (attrOr routing "hostBlocklist" [ ]) <= 64
          && lib.all validAnyCidr (attrOr routing "hostBlocklist" [ ])
          && lib.all (required: builtins.elem required (attrOr routing "hostBlocklist" [ ]))
            defaultHostBlocklist;
        message = "${network.path}.spec.routing.hostBlocklist must retain every mandatory host range.";
      }
      {
        assertion = exactKeys [ "domain" "ignoreClientNames" ] dhcp
          && (attrOr dhcp "domain" null == null
            || builtins.match tokenPattern dhcp.domain != null)
          && builtins.isBool (attrOr dhcp "ignoreClientNames" true);
        message = "${network.path}.spec.dhcp is invalid. Keep only domain and ignoreClientNames, use a bounded domain token or null, and set ignoreClientNames to a boolean.";
      }
      {
        assertion = exactKeys [ "forwarders" "cacheSize" ] dns
          && builtins.isList (attrOr dns "forwarders" [ ])
          && lib.length (attrOr dns "forwarders" [ ]) <= 8
          && lib.all validIpv4 (attrOr dns "forwarders" [ ])
          && builtins.isInt (attrOr dns "cacheSize" 1000);
        message = "${network.path}.spec.dns is invalid. Keep only forwarders and cacheSize, use at most eight IPv4 forwarders, and set cacheSize to an integer.";
      }
      {
        assertion = exactKeys
          [ "enable" "reflector" "dnsmasqLocal" "dnsmasqLocalPort" "publishWorkstation" ]
          mdns
          && builtins.isBool (attrOr mdns "enable" false)
          && builtins.isBool (attrOr mdns "reflector" true)
          && builtins.isBool (attrOr mdns "dnsmasqLocal" false)
          && builtins.isInt (attrOr mdns "dnsmasqLocalPort" 53530)
          && attrOr mdns "dnsmasqLocalPort" 0 >= 1
          && attrOr mdns "dnsmasqLocalPort" 0 <= 65535
          && builtins.isBool (attrOr mdns "publishWorkstation" false);
        message = "${network.path}.spec.mdns is invalid. Keep only enable, reflector, dnsmasqLocal, dnsmasqLocalPort, and publishWorkstation; use booleans and a port from 1 through 65535.";
      }
      {
        assertion = netVmName == null
          || (builtins.match tokenPattern netVmName != null
            && netVmName != "launcher"
            && !lib.hasPrefix "sys-" netVmName);
        message = "${network.path}.spec.netVmNameOverride is invalid or reserved. Set it to null or to a bounded lowercase name other than launcher and names beginning with sys-.";
      }
      {
        assertion = builtins.isList attachments
          && lib.length attachments <= 64
          && lib.length indexes == lib.length (lib.unique indexes);
        message = "${network.path}.spec.attachments must be bounded and use unique indices.";
      }
    ]
    ++ lib.flatten (lib.imap0
      (index: attachment:
        let where = "${network.path}.spec.attachments.${toString index}";
        in [
          {
            assertion = exactKeys [ "executionRef" "index" "mac" ] attachment;
            message = "${where} contains an unsupported field. Remove every field except executionRef, index, and mac.";
          }
          {
            assertion = builtins.match resourceRefPattern (attrOr attachment "executionRef" "") != null
              && resolvesAs resources [ "Host" "Guest" ] attachment.executionRef;
            message = "${where}.executionRef must resolve to a same-Zone Host or Guest.";
          }
          {
            assertion = builtins.isInt (attrOr attachment "index" null)
              && attachment.index >= 2
              && attachment.index <= 250;
            message = "${where}.index must be an integer from 2 through 250.";
          }
          {
            assertion = validMac (attrOr attachment "mac" null);
            message = "${where}.mac must be null or a lowercase unicast MAC address.";
          }
        ])
      attachments)
    ++ lib.optionals (builtins.isAttrs external) ([
      {
        assertion = exactKeys
          [ "mode" "parentInterface" "macvtapMode" "sharingPolicy" "mac" "ipv4" "egress" "portForwards" ]
          external;
        message = "${network.path}.spec.externalAttachment contains an unsupported field. Remove every field except mode, parentInterface, macvtapMode, sharingPolicy, mac, ipv4, egress, and portForwards.";
      }
      {
        assertion = attrOr external "mode" "macvtap" == "macvtap"
          && builtins.match ifNamePattern (attrOr external "parentInterface" "") != null
          && builtins.elem (attrOr external "macvtapMode" "bridge")
            [ "bridge" "private" "vepa" "passthru" ]
          && builtins.elem (attrOr external "sharingPolicy" "exclusive")
            [ "exclusive" "multiplexed" ]
          && (attrOr external "sharingPolicy" "exclusive" != "multiplexed"
            || attrOr external "macvtapMode" "bridge" == "bridge")
          && validMac (attrOr external "mac" null);
        message = "${network.path}.spec.externalAttachment is invalid. Set mode to macvtap, use a Linux interface name, select bridge, private, vepa, or passthru mode, use exclusive or bridge-mode multiplexed sharing, and use null or a lowercase unicast MAC.";
      }
      {
        assertion =
          let ipv4 = attrOr external "ipv4" { };
          in exactKeys [ "method" "address" "gateway" "dns" ] ipv4
            && builtins.elem (attrOr ipv4 "method" "dhcp") [ "dhcp" "static" ]
            && (attrOr ipv4 "method" "dhcp" == "dhcp"
              -> (attrOr ipv4 "address" null == null
                && attrOr ipv4 "gateway" null == null
                && attrOr ipv4 "dns" [ ] == [ ]))
            && (attrOr ipv4 "method" "dhcp" == "static"
              -> (validAnyCidr (attrOr ipv4 "address" "")
                && validIpv4 (attrOr ipv4 "gateway" null)
                && lib.all validIpv4 (attrOr ipv4 "dns" [ ])));
        message = "${network.path}.spec.externalAttachment.ipv4 is inconsistent. For dhcp, remove address, gateway, and dns; for static, set a CIDR address, an IPv4 gateway, and only IPv4 dns entries.";
      }
      {
        assertion =
          let egress = attrOr external "egress" { };
          in exactKeys [ "enable" "allowedCidrs" "masquerade" ] egress
            && builtins.isBool (attrOr egress "enable" false)
            && builtins.isList allowedCidrs
            && lib.length allowedCidrs <= 64
            && lib.all
              (cidr:
                validAnyCidr cidr
                  && lib.all
                    (row: row.zoneName != network.zoneName || !d2bLib.cidrOverlaps cidr row.cidr)
                    networkCidrs)
              allowedCidrs
            && builtins.isBool (attrOr egress "masquerade" true);
        message = "${network.path}.spec.externalAttachment.egress CIDRs must be bounded and disjoint from Zone Networks.";
      }
    ] ++ portForwardAssertions network external);

  allAssertions = lib.concatMap networkAssertions networks
    ++ map
      (pair: {
        assertion = false;
        message = "${pair.left.path}.spec.${pair.left.field} overlaps ${pair.right.path}.spec.${pair.right.field}. Change one of these CIDR options so the two Network ranges are disjoint.";
      })
      cidrOverlaps
    ++ map
      (pair: {
        assertion = false;
        message = "${pair.left.path} and ${pair.right.path}: external-physical-nic-cross-zone-l2. d2b refuses cross-Zone macvtap bridge sharing by design and provides no override. Assign the Networks different physical NICs through externalAttachment.parentInterface, or set at least one externalAttachment.macvtapMode to private, vepa, or passthru.";
      })
      crossZoneExternalClaims;

  selectedProviderPackages = lib.unique (lib.filter (package: package != null) (map
    (network:
      let provider = providerFor network.zone.resources (attrOr network.spec "providerRef" null);
          artifact = artifactFor
            (if provider != null then attrOr provider.spec "artifactId" null else null);
      in if artifact != null && artifact.type == "provider" then artifact.package else null)
    networks));
in
{
  options.d2b.zones = lib.mkOption {
    type = lib.types.attrsOf (lib.types.submodule {
      options.resources = lib.mkOption {
        type = lib.types.attrsOf (lib.types.submodule ({ config, ... }: {
          config.spec = lib.mkIf (config.type == "Network")
            (lib.mapAttrs (_: value: lib.mkDefault value) networkDefaults);
        }));
      };
    });
  };

  config = lib.mkIf (networks != [ ]) {
    assertions = allAssertions;

    d2b._index.networks = {
      list = compiledNetworks;
      byZone = compiledByZone;
    };
    d2b._resourceCompiler.networks = compiledByZone;

    networking.networkmanager.unmanaged = lib.mkAfter [ "interface-name:d2b-*" ];
    environment.systemPackages = selectedProviderPackages;
  };
}
