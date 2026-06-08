# Per-environment network materialisation for nixling.
#
# For each `nixling.envs.<env>`, this module produces:
#
#   • Two host-side bridges:
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
#   • Per-env `nixling-sys-<env>-usbipd-proxy.{service,socket}`
#     pairs that bind the host's uplink IP and proxy to a per-env
#     internal backend (`nixling-sys-<env>-usbipd-backend.service`,
#     bound to 127.0.0.1:<port>). The proxy sockets are
#     socket-activated and always bound.
#   • `networking.nat.internalInterfaces` growth so the net VM's
#     SNATted egress gets re-NATted to the upstream.
#   • Static route on the host's uplink interface so the host can
#     `ssh user@<lan>.<index>` and packets head out via the net VM.
#
# H1 (bridge port isolation): workload taps land on the LAN bridge
# with `Isolated = true` so they can only exchange frames with the
# net VM's tap, not with each other.  The net VM's LAN tap
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
  nl = import ./lib.nix { inherit lib; };
  inherit (nl) subnetIp subnetMask mkMac;

  # -------- Per-env materialisation ------------------------------------------
  envs = lib.filterAttrs (_: n: n.enable) cfg.envs;

  # All enabled VMs (workload + net) — used by the route-preflight
  # ordering/dependency wiring below (W3b H1).
  enabledVms = lib.filterAttrs (_: vm: vm.enable) cfg.vms;

  # Workload VMs in an env (excludes the net VM and any VM with env=null).
  workloadsInEnv = envName:
    lib.filterAttrs
      (_: vm: vm.enable && vm.env == envName)
      cfg.vms;

  # Per-env metadata used by host.nix, cli.nix, and net.nix.
  # `hostBlocklist` is augmented with the host's own primary-LAN CIDRs
  # (cfg.hostLanCidrs) so the default DROP rule explicitly excludes
  # everything on the wire the host itself sits on, not just the
  # broad RFC1918 catch-alls.
  netMeta = envName: net: rec {
    name = envName;
    inherit (net) lanSubnet uplinkSubnet netName;
    hostBlocklist = lib.unique (net.hostBlocklist ++ cfg.hostLanCidrs);
    lanBridge = "br-${envName}-lan";
    uplinkBridge = "br-${envName}-up";
    hostUplinkIp = subnetIp uplinkSubnet 1;
    netUplinkIp = subnetIp uplinkSubnet 2;
    netLanIp = subnetIp lanSubnet 1;
    uplinkMask = subnetMask uplinkSubnet;
    lanMask = subnetMask lanSubnet;
    # DHCP pool: avoid the net VM (.1), reserved low (.2–.9), and
    # the static-reservation block (.10–.250).
    dhcpRangeStart = subnetIp lanSubnet 251;
    dhcpRangeEnd = subnetIp lanSubnet 254;
    netUplinkMac = mkMac envName "up" 2;
    netLanMac = mkMac envName "lan" 1;
    workloads = lib.mapAttrs
      (vmName: vm: {
        ip = subnetIp lanSubnet vm.index;
        mac = mkMac envName "lan" vm.index;
        hostName = vmName;
      })
      (workloadsInEnv envName);
  };

  allMeta = lib.mapAttrs netMeta envs;

  # Per-env backend port: 3241 + alphabetical index of env name.
  # lib.attrNames returns names sorted, so the assignment is deterministic.
  # Lifted here so both systemd.services and networking.firewall.extraCommands
  # can reference it without duplicating the computation.
  envNames = lib.attrNames envs;
  envPortMap = lib.listToAttrs (
    lib.imap0 (i: name: { inherit name; value = 3241 + i; }) envNames
  );
  backendPort = envName: envPortMap.${envName};
in
{
  # Expose the per-env metadata to host.nix and cli.nix.
  nixling._envMeta = allMeta;

  # ---------------------------------------------------------------------------
  # /etc/hosts entries so the host can `ssh <vm>` without remembering
  # IPs. We don't trust dnsmasq for the host's own name resolution
  # (the host doesn't use the net VMs as DNS), so we write the
  # mapping directly via networking.hosts. Covers every workload VM
  # (via its LAN IP) plus each env's net VM (uplink IP, since the
  # host has no LAN-side route to .1).
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
        assertion = vm.env == null || lib.hasAttr vm.env cfg.envs;
        message = "nixling.vms.${vmName}.env = \"${toString vm.env}\" "
          + "but nixling.envs has no such env (have: "
          + lib.concatStringsSep ", " (lib.attrNames cfg.envs) + ").";
      })
      cfg.vms)
    # `staticIp` and `env` are mutually exclusive.
    ++ (lib.mapAttrsToList
      (vmName: vm: {
        assertion = !(vm.staticIp != null && vm.env != null);
        message = "nixling.vms.${vmName}: set EITHER `env`/`index` "
          + "OR the deprecated `staticIp`, not both.";
      })
      cfg.vms)
    # Unique indices within an env.
    ++ (lib.mapAttrsToList
      (envName: _:
        let
          indices = lib.mapAttrsToList (_: vm: vm.index) (workloadsInEnv envName);
          dups = lib.subtractLists (lib.unique indices) indices;
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
    # Phase 2b networking hardening: per-env CIDR validation.
    # - lanSubnet MUST be exactly /24 with the network address
    #   ending in `.0` (the framework's static-IP scheme assumes
    #   .0-.254 host range).
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
    # Inter-env CIDR overlap (W3b H3): exact-string equality was the
    # phase-2b check; it missed real overlaps like
    # `10.0.0.0/16` ⊃ `10.0.1.0/24`. cidrOverlaps does pure-Nix
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
    # the per-link .network file is honoured for that interface but
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
          # H1: disable STP and multicast snooping on the LAN bridge.
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
          # H1: same rationale for the /30 uplink bridge (only ever has
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
        # Uplink bridge: host has the /30 .1 here, plus a static
        # route to the LAN via the net VM's .2.
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
          };
          linkConfig.RequiredForOnline = "no";
        };

        # LAN bridge: host has NO IP. The bridge interface still
        # needs a networkd entry so it comes up administratively
        # (otherwise the kernel won't forward).
        "20-${m.lanBridge}" = {
          matchConfig.Name = m.lanBridge;
          networkConfig = {
            ConfigureWithoutCarrier = true;
          };
          linkConfig = {
            RequiredForOnline = "no";
          };
        };

        # Net-VM uplink tap → uplink bridge. Only the net VM uses
        # this bridge so isolation is not applicable here.
        "30-up-${envName}" = {
          matchConfig.Name = "${envName}-u*";
          networkConfig.Bridge = m.uplinkBridge;
          linkConfig.RequiredForOnline = "no";
        };

        # H1: Net-VM LAN tap (${envName}-l1) → LAN bridge, NOT isolated.
        # The net VM must be able to send to and receive from every
        # workload tap; the exact-name match (priority 25) wins over
        # the wildcard workload rule (priority 30) so this tap stays
        # fully connected.
        "25-net-lan-${envName}" = {
          matchConfig.Name = "${envName}-l1";
          networkConfig.Bridge = m.lanBridge;
          linkConfig.RequiredForOnline = "no";
        };

        # H1: Workload LAN taps (${envName}-l<index>, index >= 2) →
        # LAN bridge, ISOLATED. Each workload tap can only exchange
        # frames with the net VM's tap, not with peer workload taps.
        # The priority-25 rule above claims ${envName}-l1 first, so
        # this rule only applies to workload taps.
        "30-lan-${envName}" = {
          matchConfig.Name = "${envName}-l*";
          networkConfig.Bridge = m.lanBridge;
          linkConfig.RequiredForOnline = "no";
          bridgeConfig.Isolated = true;
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
      # P2r3 nixos-1/networking-1: 3240 is now handled by explicit iptables
      # rules in extraCommands (with -I nixos-fw 1) so NixOS-generated accept
      # rules are not used for this port.  uplinkBridge entry intentionally absent.
      # No allows on the lan bridge — host has no IP there anyway.
      "${m.lanBridge}" = {
        allowedTCPPorts = [ ];
        allowedUDPPorts = [ ];
      };
    })
    allMeta);

  # NAT: re-NAT the net-VM's SNATted egress to the host's upstream.
  networking.nat = {
    enable = true;
    internalInterfaces = lib.mapAttrsToList (_: m: m.uplinkBridge) allMeta;
  };

  # ---------------------------------------------------------------------------
  # M6 / P2r2 nixos-1+security-1 — usbipd exclusive single-env export.
  #
  # ARCHITECTURE NOTE (h2-usbip-arch-doc, post-W2 review):
  # ----------------------------------------------------------------
  # The W2 panel-review networking reviewer flagged that the original
  # plan called for a host-wide `nixling-sys-usbipd.service` binding
  # `127.0.0.1:3241`, whereas this module declares PER-ENV backends
  # (`nixling-sys-<env>-usbipd-backend.service`). The per-env design
  # is the **intended** and **safer** shape:
  #
  #   * Each env gets its own loopback-bound backend (different
  #     port). usbipd has no `--host` flag, so the backend binds
  #     0.0.0.0 by default — we restrict it to loopback via
  #     source-based iptables in extraCommands.
  #   * The user-facing proxy is socket-activated on the env's
  #     uplink IP (`<hostUplinkIp>:3240`) and forwards to its own
  #     backend's loopback port via systemd-socket-proxyd.
  #   * A host-wide singleton would centralize the trust boundary
  #     and create a single point where a misconfigured firewall
  #     could leak the service to other interfaces. Per-env
  #     backends fail closed.
  #
  # The cli.nix exclusive-export layer (see below) coordinates
  # which backend is active at any moment, since `usbip bind`
  # exports a device to the host-wide kernel usbip-host namespace.
  # ----------------------------------------------------------------
  #
  # Problem (addressed by this commit): a global `usbip bind -b $BUSID` exports
  # the device to the host-wide usbip-host kernel namespace, so any usbipd
  # instance—regardless of port—can serve the same device to any env.
  #
  # Fix (two layers):
  #
  #   Layer 1 — cli.nix exclusive export (primary):
  #     do_usb and do_up stop all non-target env sockets/backends before
  #     binding the device, and restore them after detach. Only one env's
  #     backend can be active while a device is bound.
  #
  #   Layer 2 — iptables defense-in-depth (below, in extraCommands):
  #     Even if a backend somehow starts, iptables rules restrict each
  #     backend port to 127.0.0.1 on lo, and each proxy port (3240) on
  #     each env's uplinkBridge to that env's own uplinkSubnet.
  #
  # Service layout: one backend per env on a distinct loopback port:
  #   nixling-sys-<env>-usbipd-backend.service — usbipd on 127.0.0.1:<port>
  #   nixling-sys-<env>-usbipd-proxy.socket    — ListenStream=<hostUplinkIp>:3240
  #   nixling-sys-<env>-usbipd-proxy.service   — socket-proxyd to its own backend port
  #
  # Ports: 3241 + alphabetical index of env name (sorted, deterministic).
  # Each proxy forwards only to its own backend; backend ports are lifted to
  # the module-level let so extraCommands can share the same computation.
  #
  # Confinement: NoNewPrivileges + CapabilityBoundingSet restrict the backend
  # to net capabilities only. ProtectSystem is omitted because usbipd requires
  # read access to /sys/bus/usb for device enumeration.
  # ---------------------------------------------------------------------------

  systemd.services =
    lib.mkMerge (
    # Per-env backend services — each on a distinct loopback port.
    lib.mapAttrsToList (_: m: {
      "nixling-sys-${m.name}-usbipd-backend" = {
        description = "USB/IP backend for nixling ${m.name} env "
          + "(127.0.0.1:${toString (backendPort m.name)})";
        serviceConfig = {
          Type = "exec";
          ExecStartPre = "${pkgs.kmod}/bin/modprobe usbip-host";
          ExecStart = "${pkgs.linuxPackages.usbip}/bin/usbipd -4 "
            + "--tcp-port ${toString (backendPort m.name)}";
          Restart = "on-failure";
          RestartSec = 2;
          # Confinement: restrict to network capabilities only.
          NoNewPrivileges = true;
          CapabilityBoundingSet = "CAP_NET_BIND_SERVICE CAP_NET_RAW";
          AmbientCapabilities  = "CAP_NET_BIND_SERVICE CAP_NET_RAW";
          RestrictAddressFamilies = "AF_INET AF_INET6 AF_UNIX AF_NETLINK";
          LockPersonality = true;
        };
      };
    }) allMeta
    # Per-env socket-proxy services — each forwards to its own backend port.
    ++ lib.mapAttrsToList (_: m: {
      "nixling-sys-${m.name}-usbipd-proxy" = {
        description = "USB/IP socket proxy for nixling ${m.name} env "
          + "(${m.hostUplinkIp}:3240 → 127.0.0.1:${toString (backendPort m.name)})";
        requires = [ "nixling-sys-${m.name}-usbipd-backend.service" ];
        after    = [ "nixling-sys-${m.name}-usbipd-backend.service" ];
        serviceConfig = {
          ExecStart =
            "${pkgs.systemd}/lib/systemd/systemd-socket-proxyd "
            + "127.0.0.1:${toString (backendPort m.name)}";
          Restart    = "on-failure";
          RestartSec = 2;
          # Basic confinement for the long-lived proxy process.
          NoNewPrivileges = true;
          CapabilityBoundingSet = "";
          LockPersonality = true;
        };
      };
    }) allMeta
    # Phase 2b networking hardening: route preflight oneshot.
    # W3b H1 followup: this unit is fail-closed. If the host has a
    # static route for any env's LAN that resolves to something other
    # than the env's `uplinkBridge`, we exit 1 — and because each
    # nixling-managed VM unit declares `Requires=` on this unit
    # (set via `requiredBy` below), the VMs refuse to start until the
    # operator clears the route conflict.
    #
    # Contract:
    #   - wantedBy = multi-user.target so it ALWAYS runs at boot.
    #   - RemainAfterExit = true so the unit stays "active" after a
    #     successful preflight and dependent units see the "active"
    #     state instead of "inactive (dead)".
    #   - For every enabled VM, ordered Before= the wrapper and the
    #     underlying microvm@ template instance, AND RequiredBy= the
    #     wrapper, so VM-start refuses if preflight fails.
    ++ lib.optional (allMeta != { }) {
      nixling-net-route-preflight = {
        description = "nixling: env LAN-route preflight (fail-closed) — blocks all nixling VMs from starting if any env's lanSubnet does not resolve via its uplink bridge";
        wantedBy = [ "multi-user.target" ];
        wants = [ "network-online.target" ];
        after = [ "systemd-networkd.service" "network-online.target" ];
        # Order + dependency: every enabled nixling-managed VM (workload
        # and net) waits on this preflight. systemd does not accept a
        # wildcard `nixling@*.service` for templates, so we enumerate.
        before =
          lib.concatMap
            (n: [ "nixling@${n}.service" "microvm@${n}.service" ])
            (lib.attrNames enabledVms);
        requiredBy = map (n: "nixling@${n}.service") (lib.attrNames enabledVms);
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
          ExecStart = "${pkgs.writeShellScript "nixling-net-route-preflight" ''
            set -u
            # v0.1.0 H6 — fail-closed invariant.
            # If `ip route get <env-lan-ip>` returns nothing (lookup
            # failed, bridge missing, no route), the preflight MUST
            # treat that as failure. The previous shape wrapped the
            # wrong-dev check in `if [ -n "$_probe_ip" ]; then`,
            # which silently passed when the lookup returned empty —
            # i.e. the very case the preflight exists to catch
            # (workloads about to start with no route at all) would
            # be reported as OK. Explicit empty-check below.
            _rc=0
            ${lib.concatStringsSep "\n" (lib.mapAttrsToList
              (envName: m: ''
                _probe_ip="$(${pkgs.iproute2}/bin/ip route get ${subnetIp m.lanSubnet 10} 2>/dev/null \
                  | head -n1)" || _probe_ip=""
                if [ -z "$_probe_ip" ]; then
                  echo "nixling-net-route-preflight: ERROR env '${envName}': 'ip route get ${subnetIp m.lanSubnet 10}' returned no result" >&2
                  echo "  This likely means the env's LAN bridge doesn't exist or has no IP." >&2
                  echo "  Check 'ip addr show ${m.uplinkBridge}' and 'systemctl status nixling-${envName}-net.service'." >&2
                  _rc=1
                else
                  case "$_probe_ip" in
                    *"dev ${m.uplinkBridge}"*)
                      : # OK — landed on this env's uplink bridge.
                      ;;
                    *)
                      echo "nixling-net-route-preflight: ERROR env '${envName}' workload IP ${subnetIp m.lanSubnet 10} resolves via:" >&2
                      echo "  $_probe_ip" >&2
                      echo "  expected dev ${m.uplinkBridge}; check for stale routes / CIDR overlaps." >&2
                      _rc=1
                      ;;
                  esac
                fi
              '')
              allMeta)}
            exit "$_rc"
          ''}";
        };
      };
    }
  );

  # ---------------------------------------------------------------------------
  # P2r3 nixos-1/networking-1/security-1 — iptables rule ordering fix.
  #
  # Problem fixed: the previous code used iptables -A (append) which placed
  # DROP rules AFTER the NixOS-generated accept rules from allowedTCPPorts.
  # iptables first-match wins, so the accept fired before the drop.
  #
  # Fix: use iptables -I nixos-fw 1 (insert at position 1) so our rules
  # are evaluated BEFORE any NixOS-generated accepts. 3240 is removed from
  # allowedTCPPorts above; we emit an explicit accept only for the correct
  # envs uplinkSubnet. Backend ports use ! -i lo (not -i lo ! -s) which
  # matches all non-loopback interfaces correctly.
  #
  # Insertion order (last inserted = position 1):
  #   1st insert: backend DROP (! -i lo) -> pushed to pos N by later inserts
  #   2nd insert: cross-env DROP for 3240 -> pushed above backend DROP
  #   3rd insert: correct-env ACCEPT for 3240 -> lands at pos 1
  # Final order: ACCEPT correct-env -> DROP cross-env -> DROP backend non-lo
  # ---------------------------------------------------------------------------
  networking.firewall.extraCommands = lib.concatMapStringsSep "\n" (m:
    let
      bPort = toString (backendPort m.name);
    in
    ''
      # P2r3 nixos-1/networking-1/security-1 [${m.name}]: insert at pos 1 (before NixOS accepts).
      # Step 1: drop non-loopback-source access to backend port ${bPort}.
      # Note: usbipd has no --host flag; it binds to 0.0.0.0. Use source-based
      # rule (! -s 127.0.0.1) instead of interface-based (! -i lo) because
      # connections from the host to its own IP route via lo, bypassing ! -i lo.
      iptables -I nixos-fw 1 -p tcp --dport ${bPort} ! -s 127.0.0.1 -j DROP
      # Step 2: drop cross-env access to usbip proxy port 3240 on this bridge (pushed to pos 2).
      iptables -I nixos-fw 1 -i ${m.uplinkBridge} -p tcp --dport 3240 ! -s ${m.uplinkSubnet} -j DROP
      # Step 3: accept correct-env access to 3240 (inserted last -> lands at pos 1, DROP at pos 2).
      iptables -I nixos-fw 1 -i ${m.uplinkBridge} -p tcp --dport 3240 -s ${m.uplinkSubnet} -j nixos-fw-accept
    ''
  ) (lib.attrValues allMeta);

  networking.firewall.extraStopCommands = lib.concatMapStringsSep "\n" (m:
    let
      bPort = toString (backendPort m.name);
    in
    ''
      iptables -D nixos-fw -p tcp --dport ${bPort} ! -s 127.0.0.1 -j DROP 2>/dev/null || true
      iptables -D nixos-fw -i ${m.uplinkBridge} -p tcp --dport 3240 ! -s ${m.uplinkSubnet} -j DROP 2>/dev/null || true
      iptables -D nixos-fw -i ${m.uplinkBridge} -p tcp --dport 3240 -s ${m.uplinkSubnet} -j nixos-fw-accept 2>/dev/null || true
    ''
  ) (lib.attrValues allMeta);

  # Per-env socket units: each binds exactly the host's uplink IP so
  # usbipd is unreachable on the WAN interface or any other address.
  systemd.sockets = lib.mkMerge (lib.mapAttrsToList
    (_: m: {
      "nixling-sys-${m.name}-usbipd-proxy" = {
        description =
          "USB/IP socket for nixling ${m.name} env (${m.hostUplinkIp}:3240)";
        socketConfig = {
          ListenStream = "${m.hostUplinkIp}:3240";
          Accept       = false;
        };
        wantedBy = [ "sockets.target" ];
      };
    })
    allMeta);

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
