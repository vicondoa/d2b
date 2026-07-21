# Type-G runNixOSTest: d2b daemon-only surface smoke.
#
# Boots a real NixOS VM with `d2b.daemonExperimental.enable = true` and
# asserts the accepted local-root end-state (ADR 0045, superseding ADR 0015's
# exactly-three-unit invariant): the fixed four root-visible local-root units
# start — `d2bd.socket`, `d2bd.service`, `d2b-priv-broker.socket`,
# `d2b-priv-broker.service` — the broker socket is socket-activated with the
# declared ACL, and the unprivileged public daemon comes up and binds its
# local-root public socket (`/run/d2b/root.sock`, `FileDescriptorName =
# "public.sock"` for activation matching only). This is the live successor of
# the eval-only + `D2B_LIVE` portions of `tests/d2bd-startup-smoke.sh` — it
# exercises real systemd activation ordering and socket binding that the
# pure-eval unit-surface gate cannot.
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

  # The fixed local-root end-state (ADR 0045): the framework declares EXACTLY
  # four root-visible units. The broker socket is socket-activated, so
  # `d2bd` keeps serving while the broker is idle; we assert the public
  # socket unit, the daemon, then the live public socket.
  testScript = ''
    start_all()

    # 1. The local-root public socket is created + listening before its
    #    service (socket activation): systemd binds/ACLs the AF_UNIX socket
    #    up front. `d2bd.service` Requires= it.
    machine.wait_for_unit("d2bd.socket")

    # 2. Broker socket is created + listening before its service (socket
    #    activation): systemd binds/ACLs the AF_UNIX socket up front.
    machine.wait_for_unit("d2b-priv-broker.socket")

    # 3. The unprivileged public daemon comes up. It Wants= (not Requires=) the
    #    broker socket, so it serves while the broker stays idle.
    machine.wait_for_unit("d2bd.service")
    machine.succeed("test \"$(systemctl show -P Type d2bd.service)\" = notify")
    machine.succeed("test \"$(systemctl show -P NotifyAccess d2bd.service)\" = main")
    machine.succeed("test \"$(systemctl show -P KillMode d2bd.service)\" = process")
    machine.succeed(
        "systemctl show -P ExecStop d2bd.service | grep -q d2b-host-shutdown-hook"
    )

    # 4. The live public wire surface: d2bd binds its AF_UNIX socket at the
    #    real local-root path (`/run/d2b/root.sock`; `public.sock` is only the
    #    systemd `FileDescriptorName` used for activation matching).
    machine.wait_for_file("/run/d2b/root.sock")
    machine.succeed("test -S /run/d2b/root.sock")
    machine.succeed(
        "tmp=$(mktemp) && "
        "jq --arg path \"$D2B_MANIFEST_PATH\" "
        "'.artifacts.publicManifestPath = $path' "
        "/etc/d2b/daemon-config.json > \"$tmp\" && "
        "install -m 0640 -o root -g d2bd \"$tmp\" /etc/d2b/daemon-config.json && "
        "rm -f \"$tmp\" && "
        "systemctl restart d2bd.service"
    )
    machine.wait_for_unit("d2bd.service")
    machine.succeed("test -S /run/d2b/root.sock")
    machine.succeed("runuser -u alice -- d2b list --json >/dev/null")

    # 4b. Service restart readiness + cgroup survival. The synthetic process is
    # moved into d2bd.service's cgroup so this verifies systemd KillMode
    # behavior directly without requiring a nested Cloud Hypervisor guest in this
    # fast smoke test. The actual VM runner-survival test lives in
    # daemon-restart-vm-survival.nix.
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
    machine.succeed("test -S /run/d2b/root.sock")
    machine.succeed("runuser -u alice -- d2b list --json >/dev/null")
    machine.succeed(f"test -d /proc/{survivor_pid}")
    machine.succeed(f"kill {survivor_pid}")

    # 5. Fixed local-root end-state (ADR 0045 "Verification gates"): the
    #    framework's root-visible SERVICE/SOCKET surface is exactly the four
    #    local-root units — the public socket + daemon, and the broker socket
    #    + service. No per-VM systemd template, no per-workload unit, no
    #    per-realm child unit, no microvms.target. d2b.slice is the broker's
    #    systemd-delegated cgroup slice (systemd.slices.d2b) — cgroup
    #    organization, not a framework service — so it is permitted to appear;
    #    everything else under the d2b/microvm prefix is forbidden.
    units = machine.succeed(
        "systemctl list-units --no-pager --all --plain "
        "| grep -E '^(d2b|microvm)' | awk '{print $1}' | sort"
    ).strip()
    print("d2b/microvm units:\n" + units)
    unit_names = set(units.split())
    required = {
        "d2bd.socket",
        "d2bd.service",
        "d2b-priv-broker.socket",
        "d2b-priv-broker.service",
    }
    # The delegated cgroup slice is legitimate cgroup infrastructure, not a
    # framework service/socket unit, so allow it alongside the four required.
    allowed = required | {"d2b.slice"}
    missing = required - unit_names
    assert not missing, f"local-root end-state: required units missing: {missing}"
    forbidden = unit_names - allowed
    assert not forbidden, (
        "local-root end-state violated: unexpected root-visible d2b/microvm "
        f"unit(s) {forbidden} (retired per-VM template / per-workload / "
        "per-realm child unit / host-singleton service / microvms.target?)"
    )

    # 6. The broker service is socket-activated (not running until a request),
    #    while the socket is listening. A clean idle posture.
    machine.succeed("systemctl is-active d2b-priv-broker.socket")
  '';
}
