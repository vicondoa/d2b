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

  # `nodes.machine` here is the fully evaluated NixOS config for the built
  # node (see nixos/lib/testing/testScript.nix), so `corpVmWorkload` can
  # resolve the rendered (hashed, deterministic) realmId/workloadId and the
  # canonical `/var/lib/d2b/r/<realmId>/w/<workloadId>` state dir without
  # guessing or hardcoding the retired compatibility `/var/lib/d2b/vms/<name>`
  # shape. The pidfd-table's `.vm` field is the fully-qualified canonical
  # target (`<workload>.<realmPath>.d2b`, see `RealmTarget::to_canonical` in
  # packages/d2b-realm-core/src/target.rs), not the bare workload id, so jq
  # selectors below must match `corpVmCanonicalTarget`.
  testScript = { nodes, ... }:
    let
      corpVm = d2bLib.corpVmWorkload nodes.machine;
      canonicalTarget = d2bLib.corpVmCanonicalTarget;
    in
    ''
      start_all()
      machine.wait_for_unit("d2bd.service")
      machine.wait_for_file("/run/d2b/root.sock")
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
      # The broker is the sole creator of realm/workload storage (ADR 0034); it
      # lazily creates `/var/lib/d2b/r/<realmId>/w/<workloadId>` (and the
      # `store-view/` hardlink farm beneath it) the first time the workload
      # starts. Pre-create the canonical leaf directory ourselves and bind-mount
      # a same-filesystem `/nix/store` scratch dir onto it so the farm's
      # hardlinks can land on a writable, same-fs target inside this
      # read-only-store runNixOSTest image.
      store_fixture = machine.succeed(
          "if mkdir -p /nix/store/zz-d2b-workload-test 2>/dev/null; then "
          "mkdir -p '${corpVm.stateDir}' && "
          "mount --bind /nix/store/zz-d2b-workload-test '${corpVm.stateDir}' && "
          "echo ready; "
          "else echo skipped-read-only-store; fi"
      ).strip()
      if store_fixture != "ready":
          print(
              "SKIP: actual VM survival requires a writable same-filesystem "
              "/nix/store fixture for the per-workload hardlink farm; this "
              "runNixOSTest store is read-only."
          )
          machine.succeed("systemctl restart d2bd.service")
          machine.wait_for_unit("d2bd.service")
          machine.succeed("runuser -u alice -- d2b list --json >/dev/null")
      else:
          machine.succeed("runuser -u alice -- d2b vm start corp-vm --apply --no-wait-api --json")
          machine.wait_until_succeeds(
              "jq -e '.entries[] | select(.vm == \"${canonicalTarget}\" and .role == \"ch-runner\")' "
              "/var/lib/d2b/daemon-state/pidfd-table.json"
          )

          runner = machine.succeed(
              "jq -r '.entries[] | select(.vm == \"${canonicalTarget}\" and .role == \"ch-runner\") "
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
              f"jq -e '.entries[] | select(.vm == \"${canonicalTarget}\" and .role == \"ch-runner\" "
              f"and .pid == ({runner_pid}|tonumber) and .startTimeTicks == ({runner_start}|tonumber))' "
              "/var/lib/d2b/daemon-state/pidfd-table.json"
          )
          machine.wait_until_succeeds("ping -c1 -W1 10.20.0.10")

          machine.succeed("runuser -u alice -- d2b vm stop corp-vm --apply --force --json")
    '';
}
