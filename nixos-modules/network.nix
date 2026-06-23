# Per-environment network materialisation for nixling.
#
# For each `nixling.envs.<env>`, this module produces
#
#   • Two host-side bridges
#       - br-<env>-up   (/30, host has the .1, net VM the .2)
#       - br-<env>-lan  (/24, host has NO interface, net VM is .1)
#   • An auto-declared headless net VM named `sys-<env>-net`,
#     registered as `nixling.vms."sys-${env}-net"`. The net VM
#     imports ./net.nix and gets its per-env knobs via specialArgs.
#   • Two `microvm-tap` style networkd rules per env so tap names
#     `up-<env>-*` and `lan-<env>-*` land on the right bridges.
#   • Host firewall: deny on the LAN bridge (host has no IP there
#     anyway, defence-in-depth), allow only TCP/3240 on the uplink
#     bridge (for USBIP).
#   • Per-env broker-spawned USBIP backend/proxy runner intents.
#     The backend runs `usbipd -4 --tcp-port <port>`; the proxy binds
#     the host's uplink IP:3240 and forwards to 127.0.0.1:<port>.
#   • `networking.nat.internalInterfaces` growth so the net VM's
#     SNATted egress gets re-NATted to the upstream.
#   • Static route on the host's uplink interface so the host can
#     `ssh user@<lan>.<index>` and packets head out via the net VM.
#
# Bridge port isolation: workload taps land on the LAN bridge
# with `Isolated = true` by default so they can only exchange
# frames with the net VM's tap, not with each other. The env-level
# `lan.allowEastWest` opt-in clears that flag. The net VM's LAN tap
# (`<env>-l1`) uses the priority-25 rule which comes before the
# isolation rule (priority 30) and therefore stays un-isolated.
# STP and multicast snooping are disabled on all bridges (no
# benefit, potential side channel, pure overhead at this scale).
#
# The matching guest-side wiring (auto-derived MAC + DHCP-only
# networkd block on workload VMs) lives in host.nix, since host.nix
# already owns the `microvm.vms.<name>` translation.
# network.nix does not consume `inputs`. Only host.nix imports a flake
# input (`inputs.microvm.nixosModules.host`); its `inputs` arrival is
# handled by the partial-application wrapper in ./default.nix. Listing
# `inputs` here used to risk a lazy `_module.args.inputs` lookup whose
# failure modes are opaque; the arg is dropped now that the partial-
# application pattern only covers host.nix.
{ config, pkgs, lib, ... }:

let
  cfg = config.nixling;
  index = cfg._index;
  nl = import ./lib.nix { inherit lib; };

  # -------- Per-env materialisation ------------------------------------------
  envs = index.enabledEnvs;
  envNames = index.enabledEnvNames;

  # Per-env metadata is owned by the normalized index. Use index-derived
  # env names for iteration so merely discovering environments does not force
  # metadata values that include workload scans.
  envMetaFor = envName: index.envMeta.${envName};
  allMeta = lib.genAttrs envNames envMetaFor;

  # USBIP host-side plumbing is only needed for envs that actually
  # carry a YubiKey-enabled workload VM, and only when host-side
  # YubiKey support is enabled at all.
  usbipMeta = index.usbip.envMeta;
  enabledVms = index.enabledVms;
in
{
  # ---------------------------------------------------------------------------
  # /etc/hosts entries so the host can `ssh <vm>` without remembering
  # IPs. We don't trust dnsmasq for the host's own name resolution
  # (the host doesn't use the net VMs as DNS), so we write the
  # mapping directly via networking.hosts. Covers every workload VM
  # (via its LAN IP) plus each env's net VM (uplink IP, since the
  # host has no LAN-side route to.1).
  # ---------------------------------------------------------------------------
  networking.hosts = lib.mkMerge (
    (lib.concatMap
      (m:
        # Workload VMs: ip -> [ vmName ]
        (lib.mapAttrsToList
          (vmName: w: { "${w.ip}" = [ vmName ]; })
          m.workloads)
        # Net VM: uplink IP -> [ netName ]
        ++ [{ "${m.netUplinkIp}" = [ m.netName ]; }])
      (lib.attrValues allMeta)));

  # ---------------------------------------------------------------------------
  # Assertions: catch the common mistakes early.
  # ---------------------------------------------------------------------------
  assertions =
    # Every VM that names an env must point at one that exists.
    (lib.mapAttrsToList
      (vmName: vm: {
        assertion = vm.env == null || (lib.hasAttr vm.env cfg.envs && cfg.envs.${vm.env}.enable);
        message = "nixling.vms.${vmName}.env = \"${toString vm.env}\" "
          + "but nixling.envs has no such ENABLED env (have enabled: "
          + lib.concatStringsSep ", " (lib.attrNames envs) + ").";
      })
      enabledVms)
    # `staticIp` and `env` are mutually exclusive.
    ++ (lib.mapAttrsToList
      (vmName: vm: {
        assertion = !(vm.staticIp != null && vm.env != null);
        message = "nixling.vms.${vmName}: set EITHER `env`/`index` "
          + "OR the deprecated `staticIp`, not both.";
      })
      enabledVms)
    # Unique indices within an env.
    ++ (lib.mapAttrsToList
      (envName: _:
        let
          indices = lib.mapAttrsToList (_: vm: vm.index) index.workloadsByEnv.${envName};
          dups = lib.attrNames (lib.filterAttrs (_: members: builtins.length members > 1)
            (lib.groupBy toString indices));
        in
        {
          assertion = dups == [ ];
          message = "nixling.envs.${envName}: VMs share index "
            + "values ${builtins.toJSON dups}. Each workload VM in an "
            + "env needs a unique `index`.";
        })
      envs)
    # Env name length: bridges are `br-<env>-lan` (7 + len env).
    # Linux caps interface names at 15 chars (IFNAMSIZ=16 incl. NUL),
    # so env must be ≤ 8. Tap names are `<env>-l<index>` (env + 2
    # + 1–3 digits), bounded by the same 15.
    ++ (lib.mapAttrsToList
      (envName: _: {
        assertion = lib.stringLength envName <= 8;
        message = "nixling.envs.${envName}: env name must be at "
          + "most 8 characters (Linux IFNAMSIZ-1=15 limit: bridge "
          + "`br-<env>-lan` is 7 + len(env) chars).";
      })
      envs)
    ++ (lib.mapAttrsToList
      (envName: net: {
        assertion = !(net.lan.allowEastWest && !cfg.site.allowUnsafeEastWest);
        message = "nixling.envs.${envName}.lan.allowEastWest requires nixling.site.allowUnsafeEastWest = true because peer-guest traffic is outside nixling's default isolation threat model.";
      })
      envs)
    # per-env CIDR validation.
    # - lanSubnet MUST be exactly /24 with the network address
    #   ending in `.0` (the framework's static-IP scheme assumes
    # .0-.254 host range).
    # - uplinkSubnet MUST be exactly /30 (point-to-point host↔net-VM).
    # - No two envs may share a lanSubnet or uplinkSubnet, and none
    #   may overlap with `nixling.hostLanCidrs`.
    ++ (lib.concatLists (lib.mapAttrsToList
      (envName: net:
        let
          lanParts = lib.splitString "/" net.lanSubnet;
          lanMask = if lib.length lanParts == 2 then lib.last lanParts else "";
          lanBase = lib.head lanParts;
          lanOctets = lib.splitString "." lanBase;
          upParts = lib.splitString "/" net.uplinkSubnet;
          upMask = if lib.length upParts == 2 then lib.last upParts else "";
        in
        [
          {
            assertion = lanMask == "24";
            message = "nixling.envs.${envName}.lanSubnet "
              + "= \"${net.lanSubnet}\" must be a /24 (got /${lanMask}).";
          }
          {
            assertion = lib.length lanOctets == 4
              && lib.last lanOctets == "0";
            message = "nixling.envs.${envName}.lanSubnet "
              + "= \"${net.lanSubnet}\" must have a network address "
              + "ending in '.0' (got '${lanBase}').";
          }
          {
            assertion = upMask == "30";
            message = "nixling.envs.${envName}.uplinkSubnet "
              + "= \"${net.uplinkSubnet}\" must be a /30 (got /${upMask}).";
          }
        ])
      envs))
    # Inter-env CIDR overlap: exact-string equality missed real
    # overlaps like `10.0.0.0/16` ⊃ `10.0.1.0/24`. cidrOverlaps does pure-Nix
    # IPv4 prefix arithmetic (see lib.nix). We reject any pair where
    # two distinct envs' subnets overlap, an env's lan/uplink subnets
    # overlap each other, or any env subnet overlaps with one of the
    # consumer-declared `nixling.hostLanCidrs` entries.
    ++ (
      let
        inherit (nl) cidrOverlaps;
        # Flatten every env subnet (lan + uplink) into a list of
        # { env; kind; cidr; } records so we can do pairwise overlap
        # checking with clear error messages.
        envCidrs = lib.concatMap
          (envName: [
            { env = envName; kind = "lanSubnet";    cidr = envs.${envName}.lanSubnet; }
            { env = envName; kind = "uplinkSubnet"; cidr = envs.${envName}.uplinkSubnet; }
          ])
          (lib.attrNames envs);
        # Generate all unordered pairs (i, j) with i < j.
        pairs =
          let n = lib.length envCidrs;
          in lib.concatMap
            (i: lib.genList
              (k: { a = lib.elemAt envCidrs i; b = lib.elemAt envCidrs (i + 1 + k); })
              (n - i - 1))
            (lib.genList (i: i) n);
        overlapping = lib.filter
          (p:
            # Allow comparing the SAME env's lanSubnet vs its own
            # uplinkSubnet: they cannot overlap by construction
            # (different prefix lengths and separated address space)
            # but if a misconfigured env declares overlapping values
            # we want to catch it too.
            cidrOverlaps p.a.cidr p.b.cidr)
          pairs;
        # Env-vs-host overlaps.
        envVsHost = lib.concatMap
          (e:
            lib.concatMap
              (h: lib.optional (cidrOverlaps e.cidr h)
                { env = e.env; kind = e.kind; cidr = e.cidr; host = h; })
              cfg.hostLanCidrs)
          envCidrs;
      in
      (map (p: {
        assertion = false;
        message = "nixling.envs: CIDR overlap between "
          + "${p.a.env}.${p.a.kind} (${p.a.cidr}) and "
          + "${p.b.env}.${p.b.kind} (${p.b.cidr}). "
          + "Even containment counts as overlap — VMs would alias "
          + "the same host bits and the host routing table cannot "
          + "disambiguate which env's bridge to reach.";
      }) overlapping)
      ++ (map (o: {
        assertion = false;
        message = "nixling.envs.${o.env}.${o.kind} (${o.cidr}) "
          + "overlaps with `nixling.hostLanCidrs` entry "
          + "\"${o.host}\". Pick a non-overlapping range — the "
          + "framework's static-route + NAT scheme requires every "
          + "env subnet to be disjoint from the host's primary LAN.";
      }) envVsHost)
    );

  # ---------------------------------------------------------------------------
  # Host-side bridges, IPs, static routes, networkd tap dispatch.
  # ---------------------------------------------------------------------------
  systemd.network = {
    enable = true;

    # systemd-networkd-wait-online waits for every networkd-managed
    # interface to reach >= "degraded". Our LAN bridges (br-<env>-lan)
    # sit at "Online state: unknown" forever (no host IP, no carrier
    # until a workload VM attaches), and `RequiredForOnline=no` on
    # the per-link.network file is honoured for that interface but
    # `--any` mode empirically still times out (it only counts
    # interfaces with RequiredForOnline=yes toward "any is online").
    #
    # The host's real upstream is `eno1`, which NetworkManager owns
    # (Setup=unmanaged in networkctl). systemd-networkd-wait-online
    # ignores unmanaged interfaces, so it has no real notion of
    # "the host is online" anyway — disabling it cleanly removes a
    # 2-minute timeout on every switch with no functional downside.
    # (NetworkManager has its own wait-online if anyone needs it.)
    wait-online.enable = false;

    netdevs = lib.mkMerge (lib.mapAttrsToList
      (_: m: {
        "10-${m.lanBridge}" = {
          netdevConfig = {
            Kind = "bridge";
            Name = m.lanBridge;
          };
          # Disable STP and multicast snooping on the LAN bridge.
          # This env has at most 1 net-VM tap + N workload taps; STP is
          # pure overhead (no loops possible) and IGMP snooping is a
          # timing side-channel with no benefit in an isolated /24.
          bridgeConfig = {
            STP = false;
            MulticastSnooping = false;
          };
        };
        "10-${m.uplinkBridge}" = {
          netdevConfig = {
            Kind = "bridge";
            Name = m.uplinkBridge;
          };
          # Same rationale for the /30 uplink bridge (only ever has
          # the net-VM tap, but keep settings consistent).
          bridgeConfig = {
            STP = false;
            MulticastSnooping = false;
          };
        };
      })
      allMeta);

    networks = lib.mkMerge (lib.mapAttrsToList
      (envName: m: {
        # Uplink bridge: host has the /30.1 here, plus a static
        # route to the LAN via the net VM's.2.
        #
        # v0.1.2: ConfigureWithoutCarrier = true is REQUIRED here.
        # Without it, networkd refuses to apply Address + Route
        # before the bridge has carrier, but the bridge only gets
        # carrier when the net VM attaches its uplink tap. The
        # `nixling-net-route-preflight.service` checks the static
        # route exists; it runs BEFORE the net VM start; deadlock.
        # Caught during the first real consumer migration.
        "20-${m.uplinkBridge}" = {
          matchConfig.Name = m.uplinkBridge;
          addresses = [{ Address = "${m.hostUplinkIp}/${m.uplinkMask}"; }];
          routes = [{
            Destination = m.lanSubnet;
            Gateway = m.netUplinkIp;
          }];
          networkConfig = {
            ConfigureWithoutCarrier = true;
            LinkLocalAddressing = "no";
            IPv6AcceptRA = false;
          };
          linkConfig = {
            RequiredForOnline = "no";
          } // lib.optionalAttrs (m.mtu != null) {
            MTUBytes = toString m.mtu;
          };
        };

        # LAN bridge: host has NO IP. The bridge interface still
        # needs a networkd entry so it comes up administratively
        # (otherwise the kernel won't forward).
        "20-${m.lanBridge}" = {
          matchConfig.Name = m.lanBridge;
          networkConfig = {
            ConfigureWithoutCarrier = true;
            LinkLocalAddressing = "no";
            IPv6AcceptRA = false;
          };
          linkConfig = {
            RequiredForOnline = "no";
          } // lib.optionalAttrs (m.mtu != null) {
            MTUBytes = toString m.mtu;
          };
        };

        # Net-VM uplink tap → uplink bridge. Only the net VM uses
        # this bridge so isolation is not applicable here.
        "30-up-${envName}" = {
          matchConfig.Name = "${envName}-u*";
          networkConfig.Bridge = m.uplinkBridge;
          linkConfig = {
            RequiredForOnline = "no";
          } // lib.optionalAttrs (m.mtu != null) {
            MTUBytes = toString m.mtu;
          };
        };

        # Net-VM LAN tap (${envName}-l1) → LAN bridge, NOT isolated.
        # The net VM must be able to send to and receive from every
        # workload tap; the exact-name match (priority 25) wins over
        # the wildcard workload rule (priority 30) so this tap stays
        # fully connected.
        "25-net-lan-${envName}" = {
          matchConfig.Name = "${envName}-l1";
          networkConfig.Bridge = m.lanBridge;
          linkConfig = {
            RequiredForOnline = "no";
          } // lib.optionalAttrs (m.mtu != null) {
            MTUBytes = toString m.mtu;
          };
        };

        # Workload LAN taps (${envName}-l<index>, index >= 2) →
        # LAN bridge. Default: ISOLATED so each workload tap can only
        # exchange frames with the net VM's tap, not with peer
        # workload taps. `lan.allowEastWest = true` clears the bridge
        # isolation and restores same-env east-west traffic.
        # The priority-25 rule above claims ${envName}-l1 first, so
        # this rule only applies to workload taps.
        "30-lan-${envName}" = {
          matchConfig.Name = "${envName}-l*";
          networkConfig.Bridge = m.lanBridge;
          linkConfig = {
            RequiredForOnline = "no";
          } // lib.optionalAttrs (m.mtu != null) {
            MTUBytes = toString m.mtu;
          };
          bridgeConfig.Isolated = !m.allowEastWest;
        };
      })
      allMeta);
  };

  # Don't let NetworkManager grab any of the new bridge / tap nics.
  networking.networkmanager.unmanaged = lib.flatten (lib.mapAttrsToList
    (envName: m: [
      "interface-name:${m.lanBridge}"
      "interface-name:${m.uplinkBridge}"
      "interface-name:${envName}-u*"
      "interface-name:${envName}-l*"
    ])
    allMeta);

  # ---------------------------------------------------------------------------
  # Host firewall.
  # The LAN bridges: host has no IP there — deny everything (no host
  # service should ever bind there). Uplink bridges: allow only the
  # USBIP carve-out and conntrack returns.
  # ---------------------------------------------------------------------------
  networking.firewall.interfaces = lib.mkMerge (lib.mapAttrsToList
    (envName: m: {
      # No allows on the lan bridge — host has no IP there anyway.
      "${m.lanBridge}" = {
        allowedTCPPorts = [ ];
        allowedUDPPorts = [ ];
      };
    })
    allMeta
    ++ lib.mapAttrsToList
      (_: m: {
        # The broker-owned `inet nixling` table carries the per-busid
        # USBIP carve-out/audit, but NixOS also installs a later
        # `ip filter INPUT` chain. Accept TCP/3240 on opted-in uplink
        # bridges here so that later chain does not drop traffic that
        # the broker has already scoped to the env's proxy listener.
        "${m.uplinkBridge}" = {
          allowedTCPPorts = [ 3240 ];
          allowedUDPPorts = [ ];
        };
      })
      usbipMeta);

  # NAT: re-NAT the net-VM's SNATted egress to the host's upstream.
  networking.nat = {
    enable = true;
    internalInterfaces = lib.mapAttrsToList (_: m: m.uplinkBridge) allMeta;
  };

  # ---------------------------------------------------------------------------
  # Bridge IPv6 boot-time sysctl application.
  #
  # Problem: bridges receive `disable_ipv6=1` only via the per-VM
  # ApplySysctl broker path, which fires when the FIRST VM in the env
  # starts.  Between boot and that first VM start the bridge has IPv6
  # active.  Additionally, `systemctl restart systemd-networkd` silently
  # undoes `disable_ipv6=1` by re-processing the netdev.
  #
  # Fix: emit declarative `boot.kernel.sysctl` entries for every declared
  # bridge here, applied at NixOS activation BEFORE any nixlingd/broker
  # invocation.  The per-VM ApplySysctl path is retained as defense-in-depth
  # (no change to broker emission or host-json output).
  #
  # Covers the corrected problem statement and boot-time window.
  # ---------------------------------------------------------------------------
  boot.kernel.sysctl = lib.mkMerge (lib.concatLists (lib.mapAttrsToList
    (_: m: [
      { "net.ipv6.conf.${m.lanBridge}.disable_ipv6"  = 1; }
      { "net.ipv6.conf.${m.lanBridge}.accept_ra"     = 0; }
      { "net.ipv6.conf.${m.lanBridge}.autoconf"      = 0; }
      { "net.ipv6.conf.${m.uplinkBridge}.disable_ipv6" = 1; }
      { "net.ipv6.conf.${m.uplinkBridge}.accept_ra"    = 0; }
      { "net.ipv6.conf.${m.uplinkBridge}.autoconf"     = 0; }
    ])
    allMeta));

  # ---------------------------------------------------------------------------
  # The per-env usbipd systemd units
  # (`nixling-sys-<env>-usbipd-{backend,proxy}.{service,socket}`) and the
  # `nixling-net-route-preflight.service` singleton were deleted here.
  #
  # Replacements:
  #   - usbipd-backend / usbipd-proxy: broker `SpawnRunner{role: Usbip,
  #     vm_id: sys-<env>-usbipd}` per the per-busid state machine in
  #     `docs/reference/privileges.md`.
  #   - net route preflight: `nixlingd` startup self-check +
  #     `nixling host reconcile --network --apply` via broker ops
  #   - Firewall carve-outs for per-env usbip ports: broker
  #     `UsbipBindFirewallRule` op.
  # ---------------------------------------------------------------------------


  # ---------------------------------------------------------------------------
  # Auto-declare the net VM for each env.
  # The net VM runs as a regular nixling VM (so its lifecycle is
  # nixling@<name>.service like every other VM) but its config is
  # entirely generated here from the env's metadata.
  # ---------------------------------------------------------------------------
  nixling.vms = lib.mapAttrs'
    (envName: m: {
      name = m.netName;
      value = {
        # autostart = true so the net VM comes up at host boot before
        # any workload VM tries to use the LAN. Override in
        # extraNetConfig if you need to debug-recreate manually.
        autostart = true;
        config = {
          imports = [
            ./net.nix
            (cfg.envs.${envName}.extraNetConfig or { })
          ];
          _module.args = {
            envMeta = m;
          };
        };
      };
    })
    allMeta;
}
