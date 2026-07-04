# Guest-side baseline for d2b per-env net VMs.
#
# Auto-instantiated by network.nix for each `d2b.envs.<env>`.
# The env's metadata + extraNetConfig come through specialArgs
# as `envMeta` and `envExtraConfig`.
#
# What this file owns
#   - Hostname + minimal-system config.
#   - Two systemd-networkd interfaces (eth0 = uplink, eth1 = lan).
#   - sysctl ip_forward.
#   - nftables: stateful firewall + MASQUERADE on eth0 + the carve-
#     outs that make the per-env isolation policy real.
#   - dnsmasq on eth1 with DHCP host-reservations for every workload
#     VM declared in this env, plus public-resolver forwarding.
#   - microvm.* hypervisor block (cloud-hypervisor, two tap interfaces named
#     `<env>-u2` (uplink-side) / `<env>-l1` (LAN-side), small VM,
#     no graphics).
{ envMeta, config, pkgs, lib, ... }:

let
  m = envMeta;
  homeAttachment = m.homeLan.attachment;
  homeLanEnabled = homeAttachment.enable;
  homeIf = homeAttachment.guestIfName;
  homePortForwards = m.homeLan.portForwards;
  homeSourceMatch = pf:
    lib.optionalString (pf.sourceCidrs != [ ])
      "ip saddr { ${lib.concatStringsSep ", " pf.sourceCidrs} } ";
  homeEgressRules = lib.concatMapStringsSep "\n          "
    (cidr: ''iifname "eth1" oifname "${homeIf}" ip daddr ${cidr} ct state new accept'')
    m.homeLan.egress.allowedCidrs;
  homeEgressUplinkDropRules = lib.concatMapStringsSep "\n          "
    (cidr: ''iifname "eth1" oifname "eth0" ip daddr ${cidr} drop'')
    m.homeLan.egress.allowedCidrs;
  homeDnatRules = lib.concatMapStringsSep "\n          "
    (pf: ''iifname "${homeIf}" ${homeSourceMatch pf}${pf.protocol} dport ${toString pf.listenPort} dnat to ${pf.targetIp}:${toString pf.targetPort}'')
    homePortForwards;
  homeForwardRules = lib.concatMapStringsSep "\n          "
    (pf: ''iifname "${homeIf}" oifname "eth1" ${homeSourceMatch pf}ip daddr ${pf.targetIp} ${pf.protocol} dport ${toString pf.targetPort} ct state new accept'')
    homePortForwards;
  # The net VM has./base.nix layered in by host.nix (see
  # nixos-modules/host.nix's `microvm.vms = lib.mapAttrs` block, which
  # unconditionally imports ./base.nix). Everything here builds on top
  # of that. One consequence: base.nix's catch-all DHCP fallback
  # network `10-eth-dhcp` (matchConfig.Type = "ether") is materialized
  # in the net VM too, where it would sort lex-first against the
  # per-MAC `10-uplink`/`10-lan` definitions below and DHCP both NICs
  # — breaking the static addressing. The `lib.mkForce` override at
  # the bottom of this file neutralizes it by replacing its match
  # with a MAC that can never match.
in
{
  imports = [
    ./net-mdns.nix
  ];

  networking.hostName = lib.mkDefault m.netName;

  # Routing/firewalling needs forwarding enabled.
  boot.kernel.sysctl = {
    "net.ipv4.ip_forward" = 1;
    "net.ipv4.conf.all.forwarding" = 1;
  };

  # systemd-networkd: two interfaces. Match by MAC since predictable
  # interface naming may not give us eth0/eth1 (microvm.nix's qemu
  # runner exposes them as e.g. enp0sN). The MACs come from the same
  # mkMac derivation as the host-side bridge dispatch in network.nix.
  systemd.network.networks = {
    # Neutralize base.nix's `10-eth-dhcp` catch-all on net VMs.
    # base.nix defines `10-eth-dhcp` with `matchConfig.Type = "ether"`
    # so workload VMs get DHCP on their single NIC. Net VMs have TWO
    # NICs explicitly bound by MAC below, and `10-eth-dhcp` would sort
    # lex-first (10-eth-dhcp < 10-lan < 10-uplink) and DHCP both NICs
    # — preempting the static config. `mkForce` replaces the whole
    # attrset; the bogus MAC ensures no interface ever matches, so
    # systemd-networkd writes a harmless file and skips it.
    "10-eth-dhcp" = lib.mkForce {
      matchConfig.MACAddress = "00:00:00:00:00:00";
    };
    "10-uplink" = {
      matchConfig.MACAddress = m.netUplinkMac;
      addresses = [{ Address = "${m.netUplinkIp}/${m.uplinkMask}"; }];
      routes = [{ Gateway = m.hostUplinkIp; }];
      networkConfig.DNS = [ "1.1.1.1" "8.8.8.8" ];
      linkConfig = {
        RequiredForOnline = "no";
      } // lib.optionalAttrs (m.mtu != null) {
        MTUBytes = toString m.mtu;
      };
    };
    "10-lan" = {
      matchConfig.MACAddress = m.netLanMac;
      addresses = [{ Address = "${m.netLanIp}/${m.lanMask}"; }];
      networkConfig = {
        IPMasquerade = "no";
        DNS = [ ];
      };
      linkConfig = {
        RequiredForOnline = "no";
      } // lib.optionalAttrs (m.mtu != null) {
        MTUBytes = toString m.mtu;
      };
    };
  } // lib.optionalAttrs homeLanEnabled {
    "10-home" = {
      matchConfig.MACAddress = homeAttachment.macAddress;
      networkConfig = {
        LinkLocalAddressing = "no";
        IPv6AcceptRA = false;
      } // (
        if homeAttachment.ipv4.method == "dhcp"
        then {
          DHCP = "ipv4";
          DNSDefaultRoute = false;
        }
        else {
          DNS = homeAttachment.ipv4.dns;
        }
      );
      dhcpV4Config = lib.optionalAttrs (homeAttachment.ipv4.method == "dhcp") {
        UseDNS = false;
        UseRoutes = false;
      };
      addresses = lib.optionals (homeAttachment.ipv4.method == "static") [
        { Address = homeAttachment.ipv4.address; }
      ];
      routes = lib.optionals (homeAttachment.ipv4.method == "static" && homeAttachment.ipv4.gateway != null) (
        [
          {
            Gateway = homeAttachment.ipv4.gateway;
            Metric = 2048;
          }
        ] ++ lib.optionals m.homeLan.egress.enable (map (cidr: {
          Destination = cidr;
          Gateway = homeAttachment.ipv4.gateway;
          Metric = 256;
        }) m.homeLan.egress.allowedCidrs)
      );
      linkConfig = {
        RequiredForOnline = "no";
      } // lib.optionalAttrs (m.mtu != null) {
        MTUBytes = toString m.mtu;
      };
    };
  };

  # The MACs that the host-side bridges expect on each NIC. The
  # nftables ruleset below also references "eth0"/"eth1" by name —
  # since we can't rely on the kernel naming, use systemd.link
  # files to rename them deterministically.
  systemd.network.links = {
    "10-uplink" = {
      matchConfig.MACAddress = m.netUplinkMac;
      linkConfig.Name = "eth0";
    };
    "10-lan" = {
      matchConfig.MACAddress = m.netLanMac;
      linkConfig.Name = "eth1";
    };
  } // lib.optionalAttrs homeLanEnabled {
    "10-home" = {
      matchConfig.MACAddress = homeAttachment.macAddress;
      linkConfig.Name = homeIf;
    };
  };

  # nftables firewall. We disable the legacy nixos networking.firewall
  # (it's iptables-based and would fight with our nft ruleset) and
  # author the policy directly.
  networking.firewall.enable = false;
  networking.nftables = {
    enable = true;
    ruleset = ''
      # IPv6 default-drop. d2b is IPv4-only by construction
      # (bridges, dnsmasq, hostBlocklist CIDRs are all v4); explicit
      # drop on every chain in the `ip6 filter` table closes the door
      # on any v6 traffic that may have slipped past the guest's
      # disable_ipv6 sysctl (e.g. from a misconfigured tunnel or
      # consumer override).
      table ip6 filter {
        chain input   { type filter hook input   priority 0; policy drop; }
        chain forward { type filter hook forward priority 0; policy drop; }
        chain output  { type filter hook output  priority 0; policy drop; }
      }

      table inet filter {
        chain input {
          type filter hook input priority 0; policy drop;

          iifname "lo" accept
          ct state established,related accept
          ct state invalid drop

          # Uplink: SSH from host to net VM (debug only).
          iifname "eth0" tcp dport 22 ct state new accept

          # LAN: DHCP + DNS for workload VMs.
          iifname "eth1" udp dport { 53, 67 } accept
          iifname "eth1" tcp dport 53 accept
          ${lib.optionalString homeLanEnabled ''
          # Home LAN: DHCP client replies for home0 only.
          iifname "${homeIf}" udp sport 67 udp dport 68 accept
          ''}

          ${lib.optionalString (m.homeLan.mdns.enable || m.homeLan.mdns.dnsmasqLocal.enable) ''
          # mDNS is only opened inside opted-in net VMs; host firewall
          # state remains untouched.
          iifname "${homeIf}" udp dport 5353 accept
          iifname "eth1" udp dport 5353 accept
          ''}

          # ICMP echo — rate-limited (was unconditional, now 10/s burst).
          ip protocol icmp icmp type echo-request limit rate 10/second burst 20 packets accept
        }

        chain forward {
          type filter hook forward priority 0; policy drop;

          ct state established,related accept
          ct state invalid drop

          # Clamp TCP MSS to the routed path MTU when the env rides a
          # tunneled uplink (WireGuard, Tailscale, etc).
          ${lib.optionalString m.mssClamp ''
          tcp flags syn tcp option maxseg size set rt mtu
          ''}

          # Opt-in same-env east-west traffic. This complements the
          # host bridge's `Isolated = false` path when
          # `d2b.envs.<env>.lan.allowEastWest = true`.
          ${lib.optionalString m.allowEastWest ''
          iifname "eth1" oifname "eth1" ct state new accept
          ''}

          # LAN-to-LAN (eth1→eth1) forwarding is intentionally absent
          # by default. H1 bridge isolation prevents direct L2 between workloads,
          # and the net VM must also not relay same-LAN frames at L3 unless the
          # env explicitly opts in.

          # Host → workload VMs: ssh, scp, anything the operator initiates.
          iifname "eth0" oifname "eth1" ct state new accept

          # Workload → host's USBIP daemon, only.
          iifname "eth1" oifname "eth0" \
            ip daddr ${m.hostUplinkIp} tcp dport 3240 ct state new accept
          iifname "eth1" oifname "eth0" ip daddr ${m.hostUplinkIp} drop

          # Workload → blocklisted host destinations (RFC1918, link-
          # local, etc): drop BEFORE the broad "lan to internet" rule
          # below.
          ${lib.concatMapStringsSep "\n          "
            (cidr: ''iifname "eth1" oifname "eth0" ip daddr ${cidr} drop'')
            m.hostBlocklist}

          ${lib.optionalString m.homeLan.egress.enable ''
          # If an allowed home-LAN CIDR is off-link or missing a home0 route,
          # fail closed instead of leaking it through the internet uplink.
          ${homeEgressUplinkDropRules}
          ''}

          # Workload → internet: allowed (NAT'd by postrouting below).
          iifname "eth1" oifname "eth0" ct state new accept

          ${lib.optionalString m.homeLan.egress.enable ''
          # Workload → home LAN: explicit CIDR opt-in, NAT'd behind home0.
          ${homeEgressRules}
          ''}

          ${lib.optionalString (homePortForwards != [ ]) ''
          # Home LAN → workload VMs: explicit DNAT forwards only.
          ${homeForwardRules}
          ''}
        }

        chain output {
          type filter hook output priority 0; policy accept;
        }
      }

      table inet nat {
        chain prerouting {
          type nat hook prerouting priority -100; policy accept;
          ${lib.optionalString (homePortForwards != [ ]) homeDnatRules}
        }

        chain postrouting {
          type nat hook postrouting priority 100; policy accept;
          oifname "eth0" masquerade
          ${lib.optionalString (m.homeLan.egress.enable && m.homeLan.egress.masquerade) ''
          oifname "${homeIf}" masquerade
          ''}
        }
      }
    '';
  };

  # dnsmasq: DHCP server on the LAN with per-VM host-reservations
  # plus DNS forwarding to public resolvers. The reservation list is
  # computed from envMeta.workloads (which network.nix builds from
  # every workload VM that named this env).
  services.dnsmasq = {
    enable = true;
    settings = {
      interface = "eth1";
      # bind-interfaces: dnsmasq binds the listening socket directly to
      # eth1 instead of accepting on the wildcard (0.0.0.0) socket. The
      # trade-off: bind-interfaces requires eth1 to exist when dnsmasq
      # starts (race vs networkd) — we close that race with an explicit
      # systemd-networkd-wait-online@eth1 ordering below. Security-wise
      # bind-interfaces is the safer choice: with bind-dynamic the
      # wildcard socket can leak DHCP/DNS to other interfaces (eth0)
      # if a misconfiguration ever causes it to listen there.
      bind-interfaces = true;
      listen-address = m.netLanIp;

      # DNS forwarding.
      domain-needed = true;
      bogus-priv = true;
      no-resolv = true;
      server = [ "1.1.1.1" "8.8.8.8" ];
      cache-size = 1000;

      # DHCP — pool covers the "unreserved" tail end of the subnet.
      dhcp-authoritative = true;
      dhcp-range = "${m.dhcpRangeStart},${m.dhcpRangeEnd},12h";
      dhcp-option = [
        "option:router,${m.netLanIp}"
        "option:dns-server,${m.netLanIp}"
      ];

      # Static reservations: one per workload VM declared in this env.
      dhcp-host = lib.mapAttrsToList
        (vmName: w: "${w.mac},${w.ip},${vmName},12h")
        m.workloads;

      # Prevent workloads from spoofing another workload's hostname via
      # DHCP option 12. dhcp-ignore-names makes dnsmasq ignore the
      # client-supplied hostname in DHCP requests and rely only on the
      # static dhcp-host reservations above.
      dhcp-ignore-names = true;
    };
  };

  # dnsmasq systemd confinement.
  #
  # The NixOS dnsmasq module starts the daemon as root then relies on dnsmasq's
  # internal `--user=dnsmasq` flag to drop privileges.  We harden this by
  #   1. Setting User=dnsmasq so the process never runs as root at all.
  #   2. Removing `--user=dnsmasq` from ExecStart (systemd's User= makes it
  #      redundant and the internal setuid would need SETUID/SETGID ambient
  #      caps to succeed from a non-root starting point, which we avoid).
  #   3. Replacing the module's preStart (which ran chown/touch as the service
  #      user — broken with User=dnsmasq) with a single root-privileged (+)
  #      ExecStartPre that owns all the privileged setup.
  #   4. Adding comprehensive systemd sandboxing.
  #
  # Make sure dnsmasq starts after networkd has eth1 up. bind-interfaces
  # (above) requires the kernel interface to exist at bind time, so we
  # additionally wait for systemd-networkd-wait-online@eth1 — that unit
  # blocks until eth1 reaches at least "degraded" (link up, address
  # assigned), which is exactly what bind-interfaces needs.
  systemd.services.dnsmasq = {
    after = [
      "systemd-networkd.service"
      "systemd-networkd-wait-online@eth1.service"
    ];
    wants = [
      "systemd-networkd.service"
      "systemd-networkd-wait-online@eth1.service"
    ];

    # Empty preStart so the NixOS dnsmasq module does NOT generate an
    # ExecStartPre (which would run as dnsmasq and fail the root ops).
    # Our +ExecStartPre below replaces it.
    preStart = lib.mkForce "";

    serviceConfig = {
      # --- identity ---
      User  = "dnsmasq";
      Group = "dnsmasq";

      # Remove --user=dnsmasq; systemd's User= already starts us as dnsmasq.
      ExecStart = lib.mkForce (
        "${config.services.dnsmasq.package}/bin/dnsmasq"
        + " -k --enable-dbus"
        + " -C ${config.services.dnsmasq.configFile}"
      );

      # Root-privileged (+) pre-start: mkdir, chown, touch /etc stub files,
      # and config-file test.  The + bypasses both User= and ProtectSystem=
      # so all of these succeed even though the main process is unprivileged.
      ExecStartPre = lib.mkBefore [
        "+${pkgs.writeShellScript "dnsmasq-pre-h4" ''
          set -e
          mkdir -m 755 -p /var/lib/dnsmasq
          touch /var/lib/dnsmasq/dnsmasq.leases
          chown -R dnsmasq /var/lib/dnsmasq
          touch /etc/dnsmasq-conf.conf
          ${config.services.dnsmasq.package}/bin/dnsmasq \
            --test -C ${config.services.dnsmasq.configFile}
        ''}"
      ];

      # --- capabilities ---
      # Ambient: the non-root process needs these effective after exec.
      #   CAP_NET_BIND_SERVICE — bind ports 53 (DNS) and 67 (DHCP, <1024)
      #   CAP_NET_RAW          — raw sockets for ICMP duplicate-IP detection
      #   CAP_NET_ADMIN        — DHCP route management (required by dnsmasq)
      AmbientCapabilities   = "CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN";
      # Bounding set caps what can ever be raised. SETUID/SETGID included per
      # spec; with NoNewPrivileges they cannot be leveraged via execve.
      CapabilityBoundingSet = "CAP_NET_BIND_SERVICE CAP_NET_RAW CAP_NET_ADMIN CAP_SETUID CAP_SETGID";

      # --- process hardening ---
      NoNewPrivileges       = true;
      # strict → entire FS hierarchy read-only except /dev /proc /sys and
      # tmpfs mounts (/run, /tmp).  ReadWritePaths carves out the lease dir.
      ProtectSystem         = lib.mkForce "strict";
      ProtectHome           = true;
      PrivateTmp            = true;
      PrivateDevices        = true;
      PrivateNetwork        = false;    # dnsmasq must reach the network
      ProtectKernelTunables = true;
      ProtectKernelModules  = true;
      ProtectControlGroups  = true;

      # AF_NETLINK so dnsmasq can watch RTM_NEWADDR events (alwaysKeepRunning).
      RestrictAddressFamilies = "AF_UNIX AF_INET AF_INET6 AF_NETLINK";
      RestrictNamespaces      = true;
      LockPersonality         = true;
      MemoryDenyWriteExecute  = true;

      SystemCallFilter        = "@system-service ~@privileged ~@resources";
      SystemCallArchitectures = "native";

      # --- writable state ---
      StateDirectory     = "dnsmasq";     # creates /var/lib/dnsmasq owned by dnsmasq
      ReadWritePaths     = "/var/lib/dnsmasq";
    };
  };

  # Declare the dnsmasq user/group explicitly so mutableUsers=false
  # (below) doesn't fight with the upstream module's implicit declaration.
  users.users.dnsmasq = {
    isSystemUser = true;
    group        = "dnsmasq";
    description  = "Dnsmasq daemon user";
  };
  users.groups.dnsmasq = { };

  # Extend the dnsmasq DBus policy to allow the dnsmasq user (not just
  # root) to own the uk.org.thekelleys.dnsmasq bus name.  The package shipped
  # policy only permits root; adding a supplemental policy file via
  # services.dbus.packages is the NixOS-idiomatic way to extend it.
  services.dbus.packages = [
    (pkgs.writeTextFile {
      name        = "dnsmasq-user-dbus-policy";
      destination = "/share/dbus-1/system.d/dnsmasq-user.conf";
      text        = ''
        <!DOCTYPE busconfig PUBLIC
         "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
         "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
        <busconfig>
          <policy user="dnsmasq">
            <allow own="uk.org.thekelleys.dnsmasq"/>
            <allow send_destination="uk.org.thekelleys.dnsmasq"/>
            <allow receive_sender="uk.org.thekelleys.dnsmasq"/>
          </policy>
        </busconfig>
      '';
    })
  ];

  # Prevent runtime user-database drift on net VMs. With this set,
  # NixOS owns /etc/passwd and /etc/group entirely; useradd/userdel at runtime
  # cannot persist across rebuilds.
  users.mutableUsers = false;

  # Small VM. No graphics, no TPM, no usbip — just routing.
  # The `microvm.shares` block (read-only /nix/store via virtiofs) is
  # injected by modules/d2b/store.nix as a per-VM hardlink farm so
  # this net VM only sees its own closure.
  microvm = {
    hypervisor = lib.mkDefault "cloud-hypervisor";
    vcpu = 1;
    mem = 512;

    volumes = [{
      image = "var.img";
      mountPoint = "/var";
      size = 1024;
      fsType = "ext4";
    }];

    interfaces = [
      {
        type = "tap";
        id = "${m.name}-u2";
        mac = m.netUplinkMac;
      }
      {
        type = "tap";
        id = "${m.name}-l1";
        mac = m.netLanMac;
      }
    ] ++ lib.optional homeLanEnabled {
      type = homeAttachment.mode;
      id = homeAttachment.hostIfName;
      mac = homeAttachment.macAddress;
      macvtap = {
        link = homeAttachment.interface;
        mode = homeAttachment.macvtapMode;
      };
    };
  };

  # SSH on by default but only key-auth so a forgotten password
  # can't be brute-forced. Use base.nix defaults; allow root-from-
  # host for the rare cases recovery needs it.
  services.openssh.settings.PermitRootLogin = lib.mkForce "prohibit-password";
  # Authorized keys for root in the net VM come from
  # `d2b.site.userAuthorizedKeys` via the `d2b-load-host-keys`
  # service (declared in base.nix); we keep
  # the keyFiles list empty here so a stale public-flake key never
  # accidentally winds up trusted.
  users.users.root.openssh.authorizedKeys.keyFiles = [ ];

  # The d2b-managed root SSH key + any consumer-supplied
  # `d2b.site.userAuthorizedKeys` are injected at boot by the
  # guest's `d2b-load-host-keys.service` (see base.nix), which
  # reads them from the virtiofs share at `/run/d2b-host-keys/`.
  # At NixOS module-eval time, root has neither password nor
  # authorized_keys — that would normally trip the
  # `users.allowNoPasswordLogin` assertion. Set the flag here with
  # an explicit comment: this is the framework's design, not an
  # oversight. The runtime guarantee that root *does* get a key
  # (refusing to boot if the share is empty) is provided by the
  # load-host-keys oneshot.
  users.allowNoPasswordLogin = lib.mkDefault true;
}
