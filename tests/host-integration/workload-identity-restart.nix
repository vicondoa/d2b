# Type-G runNixOSTest: workload identity survives daemon restart.
#
# Boots a NixOS VM with `d2b.daemonExperimental.enable = true` and a realm
# workload declaration (`d2b.realms.work.workloads.corp-vm`) that ties the
# legacy `d2b.vms.corp-vm` entry to a realm-native identity. The test asserts
# the W13/W16 plan requirement: after daemon restart, `d2b list --json` still
# emits a `workloadIdentity` block for realm-registered workloads.
#
# Architecture of the assertion
# ------------------------------
# Workload identity in the read model is config-driven, not state-driven:
#   1. `d2bd` builds a `WorkloadTargetIndex` from `realm-controllers.json` on
#      every public request (not stored in `ServerState`).
#   2. Runner snapshot records carry only `(pid, start_time_ticks, role)` —
#      no workload identity — so adoption is purely about process lineage.
#   3. After restart the daemon reloads `realm-controllers.json` from disk and
#      rebuilds the index; as long as the config file is stable, the
#      `workloadIdentity` field in list/status is identical to the pre-restart
#      response.
#
# The live process-identity (PID/pidfd) adoption gate is in
# `daemon-restart-vm-survival.nix`; this test focuses exclusively on the
# read-model workload-identity layer.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-workload-identity-restart";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { config, pkgs, lib, ... }: {
      environment.systemPackages = with pkgs; [ jq ];

      # Declare a realm "work" with an explicit workload entry that associates
      # the existing `d2b.vms.corp-vm` with a realm-native identity.  Using an
      # explicit `workloads.corp-vm` declaration (rather than a transitional
      # env-based entry) causes the Nix emitter to populate the `identity` block
      # in `realm-controllers.json`, which the daemon's `WorkloadTargetIndex`
      # indexes under vm_name "corp-vm".  The `network.envs` list associates the
      # realm with the existing `d2b.envs.work` declaration in the base config.
      d2b.realms.work = {
        name = "Work";
        allowedUsers = [ "alice" ];
        network.envs = [ "work" ];
        workloads.corp-vm = {
          enable = true;
          kind = "local-vm";
          legacyVmName = "corp-vm";
        };
      };
    };
  };

  testScript = ''
    import json

    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")

    # Patch the daemon config to point at the real manifest path (mirrors the
    # pattern used in other host-integration tests so the daemon can resolve
    # bundle artifacts).
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
    machine.wait_for_file("/run/d2b/public.sock")

    # ── Pre-restart: verify workloadIdentity is present in list output ────

    list_before_raw = machine.succeed("runuser -u alice -- d2b list --json")
    try:
        list_before = json.loads(list_before_raw)
    except json.JSONDecodeError as e:
        raise Exception(f"d2b list --json produced invalid JSON before restart: {e}\n{list_before_raw!r}")

    entries_before = list_before.get("entries", [])
    corp_before = next(
        (e for e in entries_before if e.get("vm") == "corp-vm"),
        None,
    )
    assert corp_before is not None, (
        f"corp-vm not found in list output before restart; entries: {entries_before}"
    )
    assert "workloadIdentity" in corp_before, (
        f"workloadIdentity missing from corp-vm list entry before restart; "
        f"entry: {corp_before}"
    )
    assert corp_before["workloadIdentity"].get("canonicalTarget") == "corp-vm.work.d2b", (
        f"wrong canonicalTarget in workloadIdentity before restart; "
        f"got: {corp_before['workloadIdentity'].get('canonicalTarget')!r}"
    )
    assert corp_before["workloadIdentity"].get("realmId") == "work", (
        f"wrong realmId in workloadIdentity before restart; "
        f"got: {corp_before['workloadIdentity'].get('realmId')!r}"
    )

    # ── Restart the daemon ────────────────────────────────────────────────

    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")

    # ── Post-restart: workloadIdentity must be identical ─────────────────

    list_after_raw = machine.succeed("runuser -u alice -- d2b list --json")
    try:
        list_after = json.loads(list_after_raw)
    except json.JSONDecodeError as e:
        raise Exception(f"d2b list --json produced invalid JSON after restart: {e}\n{list_after_raw!r}")

    entries_after = list_after.get("entries", [])
    corp_after = next(
        (e for e in entries_after if e.get("vm") == "corp-vm"),
        None,
    )
    assert corp_after is not None, (
        f"corp-vm not found in list output after restart; entries: {entries_after}"
    )
    assert "workloadIdentity" in corp_after, (
        f"workloadIdentity LOST after daemon restart — "
        f"this violates the W13/W16 restart/adoption invariant; "
        f"entry after restart: {corp_after}"
    )
    assert corp_after["workloadIdentity"].get("canonicalTarget") == "corp-vm.work.d2b", (
        f"canonicalTarget changed after daemon restart; "
        f"before: corp-vm.work.d2b  "
        f"after: {corp_after['workloadIdentity'].get('canonicalTarget')!r}"
    )
    assert corp_after["workloadIdentity"].get("realmId") == "work", (
        f"realmId changed after daemon restart; "
        f"before: work  "
        f"after: {corp_after['workloadIdentity'].get('realmId')!r}"
    )

    # Confirm the full identity block is stable across the restart.
    before_identity = corp_before["workloadIdentity"]
    after_identity = corp_after["workloadIdentity"]
    assert before_identity == after_identity, (
        f"workloadIdentity block changed across daemon restart — "
        f"config-driven identity must be deterministic; "
        f"before: {before_identity}  after: {after_identity}"
    )
  '';
}
