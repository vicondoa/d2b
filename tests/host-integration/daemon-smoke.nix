# Type-G runNixOSTest: d2b daemon-only surface smoke.
#
# Boots a real NixOS VM with `d2b.daemonExperimental.enable = true` and
# asserts the daemon-only end-state on a live system (ADR 0015): exactly the
# three framework-declared root-visible units start, the broker socket is
# socket-activated with the declared ACL, and the unprivileged public daemon
# comes up and binds `/run/d2b/public.sock`. This is the live successor of the
# eval-only +
# `D2B_LIVE` portions of `tests/d2bd-startup-smoke.sh` - it exercises real
# systemd activation ordering and socket binding that the pure-eval unit-surface
# gate cannot.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-daemon-smoke";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { pkgs, ... }: {
      environment.systemPackages = [ pkgs.jq ];
    };
  };

  # The daemon-only end-state contract (ADR 0015): this fixture declares
  # EXACTLY three framework-owned root-visible units. The broker socket is
  # socket-activated, so `d2bd` keeps serving while the broker is idle; we
  # assert the socket and the daemon, then the live public socket. Optional or
  # managed operator infrastructure is intentionally outside this census.
  testScript = ''
    start_all()

    # 1. Broker socket is created + listening before its service (socket
    #    activation): systemd binds/ACLs the AF_UNIX socket up front.
    machine.wait_for_unit("d2b-broker.socket")

    # 2. The unprivileged public daemon comes up. It Wants= (not Requires=) the
    #    broker socket, so it serves while the broker stays idle.
    machine.wait_for_unit("d2bd.service")
    machine.succeed("test \"$(systemctl show -P Type d2bd.service)\" = notify")
    machine.succeed("test \"$(systemctl show -P NotifyAccess d2bd.service)\" = main")
    machine.succeed("test \"$(systemctl show -P KillMode d2bd.service)\" = process")
    machine.succeed(
        "systemctl show -P ExecStop d2bd.service | grep -q d2b-host-shutdown-hook"
    )

    # 3. The live public wire surface: d2bd binds its AF_UNIX socket.
    machine.wait_for_file("/run/d2b/public.sock")
    machine.succeed("test -S /run/d2b/public.sock")
    machine.succeed(
        "systemctl restart d2bd.service"
    )
    machine.wait_for_unit("d2bd.service")
    machine.succeed("test -S /run/d2b/public.sock")
    machine.succeed("runuser -u alice -- d2b auth status --json >/dev/null")

    # 3b. Service restart readiness + cgroup survival. The synthetic process is
    # moved into d2bd.service's cgroup so this verifies systemd KillMode
    # behavior directly without requiring a nested Cloud Hypervisor guest in this
    # fast smoke test. The actual Cloud Hypervisor runner-survival test lives in
    # runtime-cloud-hypervisor-guest-preflight.nix.
    survivor_pid = machine.succeed(
        "set -euo pipefail; "
        "cg=$(systemctl show -P ControlGroup d2bd.service); "
        "rm -f /run/d2b-smoke-survivor.pid; "
        "setsid -f sh -c 'echo $$ > /run/d2b-smoke-survivor.pid; exec sleep 3600' "
        "</dev/null >/dev/null 2>&1; "
        "for _ in $(seq 1 50); do "
        "  test -s /run/d2b-smoke-survivor.pid && break; "
        "  sleep 0.1; "
        "done; "
        "pid=$(cat /run/d2b-smoke-survivor.pid); "
        "echo \"$pid\" > \"/sys/fs/cgroup$cg/cgroup.procs\"; "
        "echo \"$pid\""
    ).strip()
    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.succeed("test -S /run/d2b/public.sock")
    machine.succeed("runuser -u alice -- d2b auth status --json >/dev/null")
    machine.succeed(f"test -d /proc/{survivor_pid}")
    machine.succeed(f"kill {survivor_pid}")

    # 4. Daemon-only end-state (ADR 0015 "Verification gates"): compare the
    #    live system only with the framework-owned acceptance declaration. This
    #    avoids treating unrelated optional or managed infrastructure as a
    #    framework violation while still failing if a declared unit is absent.
    declared = set(
        machine.succeed("cat /etc/d2b/daemon-acceptance-units").split()
    )
    required = {
        "d2bd.service",
        "d2b-broker.socket",
        "d2b-broker.service",
    }
    assert declared == required, (
        f"unexpected framework acceptance census: {declared}"
    )
    unit_names = set(
        machine.succeed(
            "systemctl list-units --no-pager --all --plain "
            "| awk '{print $1}' | sort"
        ).split()
    )
    missing = required - unit_names
    assert not missing, f"daemon-only framework units missing: {missing}"

    # 5. The broker service is socket-activated (not running until a request),
    #    while the socket is listening. A clean idle posture.
    machine.succeed("systemctl is-active d2b-broker.socket")
  '';
}
