# Type-G runNixOSTest: Linux bridge port isolation semantics.
#
# This is the hermetic VM successor to the retired shell gate. It exercises the
# same bridge shape as root inside the test VM: one non-isolated net-VM port and
# two isolated workload ports on br-work-lan.
{ pkgs, self }:

pkgs.testers.runNixOSTest {
  name = "d2b-bridge-isolation";

  nodes.machine = { pkgs, ... }: {
    environment.systemPackages = [
      pkgs.iproute2
      pkgs.iputils
    ];
    system.stateVersion = "25.11";
  };

  testScript = ''
    start_all()

    machine.succeed("mkdir -p /run/netns")

    for ns in ["netvm", "vm10", "vm11"]:
        machine.succeed(f"ip netns add {ns}")

    machine.succeed("ip link add br-work-lan type bridge")
    machine.succeed("ip link set br-work-lan up")

    for port, ns in [
        ("work-l1", "netvm"),
        ("work-l10", "vm10"),
        ("work-l11", "vm11"),
    ]:
        machine.succeed(f"ip link add {port} type veth peer name eth0 netns {ns}")
        machine.succeed(f"ip link set {port} master br-work-lan")
        machine.succeed(f"ip link set {port} up")

    machine.succeed("bridge link set dev work-l10 isolated on")
    machine.succeed("bridge link set dev work-l11 isolated on")

    for ns in ["netvm", "vm10", "vm11"]:
        machine.succeed(f"ip netns exec {ns} ip link set lo up")
        machine.succeed(f"ip netns exec {ns} ip link set eth0 up")

    machine.succeed("ip netns exec netvm ip addr add 10.20.0.1/24 dev eth0")
    machine.succeed("ip netns exec vm10 ip addr add 10.20.0.10/24 dev eth0")
    machine.succeed("ip netns exec vm11 ip addr add 10.20.0.11/24 dev eth0")

    work_l1 = machine.succeed("bridge -d link show dev work-l1")
    assert "isolated on" not in work_l1, (
        "net-VM bridge port work-l1 must remain non-isolated"
    )
    work_l10 = machine.succeed("bridge -d link show dev work-l10")
    assert "isolated on" in work_l10, "workload bridge port work-l10 is not isolated"
    work_l11 = machine.succeed("bridge -d link show dev work-l11")
    assert "isolated on" in work_l11, "workload bridge port work-l11 is not isolated"

    machine.succeed("ip netns exec vm10 ping -c1 -W1 10.20.0.1 >/dev/null")
    machine.succeed("ip netns exec vm11 ping -c1 -W1 10.20.0.1 >/dev/null")

    machine.fail("ip netns exec vm10 ping -c1 -W1 10.20.0.11 >/dev/null 2>&1")

    machine.succeed("ip netns exec vm10 ip link set dev eth0 address 02:20:00:00:00:11")
    machine.fail("ip netns exec vm10 ping -c1 -W1 10.20.0.11 >/dev/null 2>&1")
  '';
}
