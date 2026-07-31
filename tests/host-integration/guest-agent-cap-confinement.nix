# Type-G runNixOSTest: guest network-agent capability confinement.
#
# This check gives a live process the three capabilities required by the
# network agent inside a dedicated Linux network namespace. It verifies the
# effective set and proves that starting the process adds no such capability to
# any process sharing the host network namespace.
{ pkgs, self }:

pkgs.testers.runNixOSTest {
  name = "d2b-guest-agent-cap-confinement";

  nodes.machine = { ... }: {
    users.groups.d2b-net-agent-test = { };
    users.users.d2b-net-agent-test = {
      isSystemUser = true;
      group = "d2b-net-agent-test";
    };

    environment.systemPackages = [ pkgs.iproute2 ];

    systemd.services.d2b-test-agent-netns = {
      description = "Create the isolated network-agent test namespace";
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = pkgs.writeShellScript "d2b-test-agent-netns-up" ''
          set -eu
          install -d -m 0755 /run/netns
          ${pkgs.iproute2}/bin/ip netns add d2b-test-agent
          ${pkgs.iproute2}/bin/ip -n d2b-test-agent link set lo up
        '';
        ExecStop = "${pkgs.iproute2}/bin/ip netns delete d2b-test-agent";
      };
    };

    systemd.services.d2b-test-guest-agent = {
      description = "Network agent capability-confinement test process";
      requires = [ "d2b-test-agent-netns.service" ];
      after = [ "d2b-test-agent-netns.service" ];
      serviceConfig = {
        Type = "simple";
        User = "d2b-net-agent-test";
        Group = "d2b-net-agent-test";
        ExecStart = "${pkgs.coreutils}/bin/sleep infinity";
        NetworkNamespacePath = "/run/netns/d2b-test-agent";
        CapabilityBoundingSet = [
          "CAP_NET_ADMIN"
          "CAP_NET_BIND_SERVICE"
          "CAP_NET_RAW"
        ];
        AmbientCapabilities = [
          "CAP_NET_ADMIN"
          "CAP_NET_BIND_SERVICE"
          "CAP_NET_RAW"
        ];
        NoNewPrivileges = true;
      };
    };

    system.stateVersion = "25.11";
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")
    machine.succeed("systemctl start d2b-test-agent-netns.service")

    capability_mask = (1 << 10) | (1 << 12) | (1 << 13)

    def network_namespace(pid):
        return machine.succeed(f"readlink /proc/{pid}/ns/net").strip()

    def network_namespace_inode(path):
        return machine.succeed(f"stat -Lc '%d:%i' {path}").strip()

    def effective_capabilities(pid):
        status = machine.succeed(f"cat /proc/{pid}/status")
        for line in status.splitlines():
            if line.startswith("CapEff:"):
                return int(line.split(":", 1)[1].strip(), 16)
        raise AssertionError(f"process {pid} has no CapEff status field")

    def privileged_host_processes():
        rows = machine.succeed(
            "host_ns=$(readlink /proc/1/ns/net); "
            "for status in /proc/[0-9]*/status; do "
            "pid=''${status#/proc/}; pid=''${pid%/status}; "
            "ns=$(readlink /proc/$pid/ns/net 2>/dev/null) || continue; "
            "test \"$ns\" = \"$host_ns\" || continue; "
            "cap=$(while IFS=: read -r key value; do "
            "test \"$key\" = CapEff && { printf '%s' \"$value\"; break; }; done < \"$status\"); "
            "start=$(cut -d' ' -f22 /proc/$pid/stat 2>/dev/null) || continue; "
            "printf '%s %s %s\\n' \"$pid\" \"$start\" \"$cap\"; "
            "done"
        )
        result = set()
        for row in rows.splitlines():
            pid, start, cap = row.split()
            if int(cap, 16) & capability_mask:
                result.add((pid, start))
        return result

    host_namespace = network_namespace(1)
    baseline = privileged_host_processes()

    machine.succeed("systemctl start d2b-test-guest-agent.service")
    machine.wait_for_unit("d2b-test-guest-agent.service")
    agent_pid = machine.succeed(
        "systemctl show -P MainPID d2b-test-guest-agent.service"
    ).strip()
    assert agent_pid not in ("", "0"), "network agent did not start"

    agent_namespace = network_namespace(agent_pid)
    agent_namespace_inode = network_namespace_inode(f"/proc/{agent_pid}/ns/net")
    declared_namespace_inode = network_namespace_inode("/run/netns/d2b-test-agent")
    assert agent_namespace_inode == declared_namespace_inode, (
        "network agent did not inherit the declared Guest network namespace"
    )
    assert agent_namespace != host_namespace, (
        "network agent unexpectedly shares the host network namespace"
    )

    agent_capabilities = effective_capabilities(agent_pid)
    assert agent_capabilities & capability_mask == capability_mask, (
        "network agent is missing a required effective network capability"
    )
    assert agent_capabilities & ~capability_mask == 0, (
        "network agent received an undeclared effective capability"
    )

    after = privileged_host_processes()
    leaked = after - baseline
    assert not leaked, (
        "starting the network agent added effective network capabilities to "
        f"host-network-namespace processes: {sorted(leaked)}"
    )
  '';
}
