{ pkgs, self }:

let
  nixlingLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "nixling-daemon-restart-vm-survival";

  nodes.machine = nixlingLib.nixlingDaemonNode {
    writableStore = true;
    extra = { config, pkgs, ... }: {
      nixling.site.adminUsers = [ "alice" ];
      environment.variables.NIXLING_MANIFEST_PATH = config.nixling._manifestJsonPath;
      environment.systemPackages = with pkgs; [
        iputils
        jq
      ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("nixlingd.service")
    machine.wait_for_file("/run/nixling/public.sock")
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
    store_fixture = machine.succeed(
        "if mkdir -p /nix/store/zz-nixling-vms-test 2>/dev/null; then "
        "mkdir -p /var/lib/nixling/vms && "
        "mount --bind /nix/store/zz-nixling-vms-test /var/lib/nixling/vms && "
        "echo ready; "
        "else echo skipped-read-only-store; fi"
    ).strip()
    if store_fixture != "ready":
        print(
            "SKIP: actual VM survival requires a writable same-filesystem "
            "/nix/store fixture for the per-VM hardlink farm; this "
            "runNixOSTest store is read-only."
        )
        machine.succeed("systemctl restart nixlingd.service")
        machine.wait_for_unit("nixlingd.service")
        machine.succeed("runuser -u alice -- nixling list --json >/dev/null")
    else:
        machine.succeed("runuser -u alice -- nixling vm start corp-vm --apply --no-wait-api --json")
        machine.wait_until_succeeds(
            "jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\")' "
            "/var/lib/nixling/daemon-state/pidfd-table.json"
        )

        runner = machine.succeed(
            "jq -r '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\") "
            "| \"\\(.pid) \\(.startTimeTicks)\"' "
            "/var/lib/nixling/daemon-state/pidfd-table.json"
        ).strip()
        runner_pid, runner_start = runner.split()
        machine.succeed(f"test -d /proc/{runner_pid}")
        machine.succeed(
            f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
        )

        machine.wait_until_succeeds("ping -c1 -W1 10.20.0.10")
        machine.succeed("systemctl restart nixlingd.service")
        machine.wait_for_unit("nixlingd.service")
        machine.succeed("runuser -u alice -- nixling vm status corp-vm --json")

        machine.succeed(f"test -d /proc/{runner_pid}")
        machine.succeed(
            f"test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}"
        )
        machine.wait_until_succeeds(
            f"jq -e '.entries[] | select(.vm == \"corp-vm\" and .role == \"ch-runner\" "
            f"and .pid == ({runner_pid}|tonumber) and .startTimeTicks == ({runner_start}|tonumber))' "
            "/var/lib/nixling/daemon-state/pidfd-table.json"
        )
        machine.wait_until_succeeds("ping -c1 -W1 10.20.0.10")

        machine.succeed("runuser -u alice -- nixling vm stop corp-vm --apply --force --json")
  '';
}
