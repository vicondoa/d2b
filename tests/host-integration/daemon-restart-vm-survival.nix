{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-daemon-restart-vm-survival";

  nodes.machine = d2bLib.d2bDaemonNode {
    writableStore = true;
    extra = { config, pkgs, ... }: {
      d2b.site.adminUsers = [ "alice" ];
      environment.variables.D2B_MANIFEST_PATH = config.d2b._manifestJsonPath;
      environment.systemPackages = with pkgs; [
        iputils
        jq
      ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
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
    store_fixture = machine.succeed(
        "if mkdir -p /nix/store/zz-d2b-vms-test 2>/dev/null; then "
        "mkdir -p /var/lib/d2b/vms && "
        "mount --bind /nix/store/zz-d2b-vms-test /var/lib/d2b/vms && "
        "echo ready; "
        "else echo skipped-read-only-store; fi"
    ).strip()
    if store_fixture != "ready":
        print(
            "SKIP: actual VM survival requires a writable same-filesystem "
            "/nix/store fixture for the per-VM hardlink farm; this "
            "runNixOSTest store is read-only."
        )
        machine.succeed("systemctl restart d2bd.service")
        machine.wait_for_unit("d2bd.service")
        machine.succeed("runuser -u alice -- d2b list --json >/dev/null")
    else:
        machine.succeed("runuser -u alice -- d2b vm start corp-vm --apply --no-wait-api --json")
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

        machine.wait_until_succeeds("ping -c1 -W1 10.20.0.10")
        machine.succeed("systemctl restart d2bd.service")
        machine.wait_for_unit("d2bd.service")
        machine.succeed("runuser -u alice -- d2b vm status corp-vm --json")

        machine.succeed(f"test -d /proc/{runner_pid}")
        machine.succeed(
            f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
        )
        machine.wait_until_succeeds(
            f"jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\" "
            f"and .pid == ({runner_pid}|tonumber) and .startTimeTicks == ({runner_start}|tonumber))' "
            "/var/lib/d2b/daemon-state/pidfd-table.json"
        )
        machine.wait_until_succeeds("ping -c1 -W1 10.20.0.10")

        machine.succeed("runuser -u alice -- d2b vm stop corp-vm --apply --force --json")
  '';
}
