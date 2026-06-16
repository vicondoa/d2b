# Type-G runNixOSTest: nixling daemon-only surface smoke.
#
# Boots a real NixOS VM with `nixling.daemonExperimental.enable = true` and
# asserts the daemon-only end-state on a live system (ADR 0015): exactly the
# three root-visible units start, the broker socket is socket-activated with the
# declared ACL, and the unprivileged public daemon comes up and binds
# `/run/nixling/public.sock`. This is the live successor of the eval-only +
# `NL_LIVE` portions of `tests/nixlingd-startup-smoke.sh` — it exercises real
# systemd activation ordering and socket binding that the pure-eval unit-surface
# gate cannot.
{ pkgs, self }:

let
  nixlingLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "nixling-daemon-smoke";

  nodes.machine = nixlingLib.nixlingDaemonNode { };

  # The daemon-only end-state contract (ADR 0015): the framework declares
  # EXACTLY three root-visible units. The broker socket is socket-activated, so
  # `nixlingd` keeps serving while the broker is idle; we assert the socket and
  # the daemon, then the live public socket.
  testScript = ''
    start_all()

    # 1. Broker socket is created + listening before its service (socket
    #    activation): systemd binds/ACLs the AF_UNIX socket up front.
    machine.wait_for_unit("nixling-priv-broker.socket")

    # 2. The unprivileged public daemon comes up. It Wants= (not Requires=) the
    #    broker socket, so it serves while the broker stays idle.
    machine.wait_for_unit("nixlingd.service")

    # 3. The live public wire surface: nixlingd binds its AF_UNIX socket.
    machine.wait_for_file("/run/nixling/public.sock")
    machine.succeed("test -S /run/nixling/public.sock")

    # 4. Daemon-only end-state exit criterion (ADR 0015 "Verification gates"):
    #    exactly three root-visible nixling/microvm units exist — the public
    #    daemon, the broker socket, and the broker service. No per-VM systemd
    #    template, no host-singleton framework service, no microvms.target.
    units = machine.succeed(
        "systemctl list-units --no-pager --all --plain "
        "| grep -E '^(nixling|microvm)' | awk '{print $1}' | sort"
    ).strip()
    print("nixling/microvm units:\n" + units)
    unit_names = set(units.split())
    expected = {
        "nixlingd.service",
        "nixling-priv-broker.socket",
        "nixling-priv-broker.service",
    }
    assert unit_names == expected, (
        f"daemon-only end-state violated: expected exactly {expected}, "
        f"got {unit_names}"
    )

    # 5. The broker service is socket-activated (not running until a request),
    #    while the socket is listening. A clean idle posture.
    machine.succeed("systemctl is-active nixling-priv-broker.socket")
  '';
}
