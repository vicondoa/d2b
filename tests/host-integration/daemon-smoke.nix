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

  nodes.machine = nixlingLib.nixlingDaemonNode {
    extra = { pkgs, ... }: {
      environment.systemPackages = [ pkgs.jq ];
    };
  };

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
    machine.succeed("test \"$(systemctl show -P Type nixlingd.service)\" = notify")
    machine.succeed("test \"$(systemctl show -P NotifyAccess nixlingd.service)\" = main")
    machine.succeed("test \"$(systemctl show -P KillMode nixlingd.service)\" = process")
    machine.succeed(
        "systemctl show -P ExecStop nixlingd.service | grep -q nixling-host-shutdown-hook"
    )

    # 3. The live public wire surface: nixlingd binds its AF_UNIX socket.
    machine.wait_for_file("/run/nixling/public.sock")
    machine.succeed("test -S /run/nixling/public.sock")
    machine.succeed(
        "tmp=$(mktemp) && "
        "jq --arg path \"$NIXLING_MANIFEST_PATH\" "
        "'.artifacts.publicManifestPath = $path' "
        "/etc/nixling/daemon-config.json > \"$tmp\" && "
        "install -m 0640 -o root -g nixlingd \"$tmp\" /etc/nixling/daemon-config.json && "
        "rm -f \"$tmp\" && "
        "systemctl restart nixlingd.service"
    )
    machine.wait_for_unit("nixlingd.service")
    machine.succeed("test -S /run/nixling/public.sock")
    machine.succeed("runuser -u alice -- nixling list --json >/dev/null")

    # 3b. Service restart readiness + cgroup survival. The synthetic process is
    # moved into nixlingd.service's cgroup so this verifies systemd KillMode
    # behavior directly without requiring a nested Cloud Hypervisor guest in this
    # fast smoke test. The actual VM runner-survival test lives in
    # daemon-restart-vm-survival.nix.
    survivor_pid = machine.succeed(
        "set -euo pipefail; "
        "cg=$(systemctl show -P ControlGroup nixlingd.service); "
        "rm -f /run/nixling-smoke-survivor.pid; "
        "setsid -f sh -c 'echo $$ > /run/nixling-smoke-survivor.pid; exec sleep 3600' "
        "</dev/null >/dev/null 2>&1; "
        "for _ in $(seq 1 50); do "
        "  test -s /run/nixling-smoke-survivor.pid && break; "
        "  sleep 0.1; "
        "done; "
        "pid=$(cat /run/nixling-smoke-survivor.pid); "
        "echo \"$pid\" > \"/sys/fs/cgroup$cg/cgroup.procs\"; "
        "echo \"$pid\""
    ).strip()
    machine.succeed("systemctl restart nixlingd.service")
    machine.wait_for_unit("nixlingd.service")
    machine.succeed("test -S /run/nixling/public.sock")
    machine.succeed("runuser -u alice -- nixling list --json >/dev/null")
    machine.succeed(f"test -d /proc/{survivor_pid}")
    machine.succeed(f"kill {survivor_pid}")

    # 4. Daemon-only end-state (ADR 0015 "Verification gates"): the framework's
    #    root-visible SERVICE/SOCKET surface is exactly the public daemon, the
    #    broker socket, and the broker service. No per-VM systemd template, no
    #    host-singleton framework service, no microvms.target. nixling.slice is
    #    the broker's systemd-delegated cgroup slice (systemd.slices.nixling) —
    #    cgroup organization, not a framework service — so it is permitted to
    #    appear; everything else under the nixling/microvm prefix is forbidden.
    units = machine.succeed(
        "systemctl list-units --no-pager --all --plain "
        "| grep -E '^(nixling|microvm)' | awk '{print $1}' | sort"
    ).strip()
    print("nixling/microvm units:\n" + units)
    unit_names = set(units.split())
    required = {
        "nixlingd.service",
        "nixling-priv-broker.socket",
        "nixling-priv-broker.service",
    }
    # The delegated cgroup slice is legitimate cgroup infrastructure, not a
    # framework service/socket unit, so allow it alongside the three required.
    allowed = required | {"nixling.slice"}
    missing = required - unit_names
    assert not missing, f"daemon-only end-state: required units missing: {missing}"
    forbidden = unit_names - allowed
    assert not forbidden, (
        "daemon-only end-state violated: unexpected root-visible nixling/microvm "
        f"unit(s) {forbidden} (retired per-VM template / host-singleton service / "
        "microvms.target?)"
    )

    # 5. The broker service is socket-activated (not running until a request),
    #    while the socket is listening. A clean idle posture.
    machine.succeed("systemctl is-active nixling-priv-broker.socket")
  '';
}
