# Type-G runNixOSTest: Guest process adoption across a daemon restart.
#
# This is the lowest public host selector that can exercise the installed
# Cloud Hypervisor runner and pidfd recovery state. It is intentionally not a
# substitute for the native controller acceptance: it proves the production
# daemon restart/process-adoption boundary when the VM prerequisites are
# available.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-runtime-cloud-hypervisor-guest-acceptance";

  nodes.machine = d2bLib.d2bDaemonNode {
    writableStore = true;
    extra = { config, pkgs, ... }: {
      d2b.site.adminUsers = [ "alice" ];
      environment.variables.D2B_MANIFEST_PATH = config.d2b._manifestJsonPath;
      environment.systemPackages = with pkgs; [ jq iputils ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")

    # The VM runner requires the writable same-filesystem store fixture. Keep
    # this exact target fail-closed rather than claiming a native pass when the
    # runNixOSTest image cannot provide it.
    machine.succeed(
        "mkdir -p /nix/store/zz-d2b-vms-test /var/lib/d2b/vms && "
        "mount --bind /nix/store/zz-d2b-vms-test /var/lib/d2b/vms"
    )
    machine.succeed(
        "runuser -u alice -- d2b vm start corp-vm --apply --no-wait-api --json"
    )
    machine.wait_until_succeeds(
        "jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\")' "
        "/var/lib/d2b/daemon-state/pidfd-table.json"
    )
    runner = machine.succeed(
        "jq -r '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\") "
        "| \"\\(.pid) \\(.startTimeTicks)\"' "
        "/var/lib/d2b/daemon-state/pidfd-table.json"
    ).strip()
    runner_pid, runner_start = runner.split()
    machine.succeed(f"test -d /proc/{runner_pid}")
    machine.succeed(
        f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
    )
    machine.succeed(
        "runuser -u alice -- d2b --zone work --json resource list Guest "
        "> /run/d2b-guest-before.json"
    )
    machine.succeed(
        "jq -e '.items[] | select(.resourceRef == \"Guest/corp-vm\")' "
        "/run/d2b-guest-before.json"
    )

    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
    machine.succeed("runuser -u alice -- d2b vm status corp-vm --json")
    machine.succeed(f"test -d /proc/{runner_pid}")
    machine.succeed(
        f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
    )
    machine.wait_until_succeeds(
        f"jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\" "
        f"and .pid == ({runner_pid}|tonumber) and "
        f".startTimeTicks == ({runner_start}|tonumber))' "
        "/var/lib/d2b/daemon-state/pidfd-table.json"
    )
    machine.succeed(
        "runuser -u alice -- d2b vm stop corp-vm --apply --force --json"
    )
  '';
}
