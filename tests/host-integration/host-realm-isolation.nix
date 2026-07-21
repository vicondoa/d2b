# Type-G runNixOSTest: host-local realm isolation invariants.
#
# W7 deleted the gateway-vm relay feature this test used to exercise (realm
# `relay`/`inherit-env` config, `providers.aca`, and the
# `/etc/d2b/host-realm-relay-egress-policy.json` artifact no longer exist
# anywhere in nixos-modules/* — see docs/reference/realm-policy.md, which is
# explicit that the current realm surface only materializes host-local
# control-plane scaffolding and that "the removed gateway option surface is
# not a policy mode"). Declaring a second `placement = "gateway-vm"` realm
# today would just re-invent an unimplemented feature, not exercise live
# behaviour.
#
# What IS live and testable today (nixos-modules/realm-controller-config-json.nix,
# realm-users.nix, realm-access.nix): every additional host-local realm gets
# its own deterministic realmId, its own `d2bd-r-<realmId>` / `d2bbr-r-<realmId>`
# principals and `d2bcg-r-<realmId>` / `d2b-r-<realmId>` groups, its own
# disjoint resource paths under `/var/lib/d2b/r/<realmId>`, and a
# `/etc/d2b/realm-controllers.json` row — while runtime routing for those
# child realms remains inert ("metadata-only": no per-realm systemd
# unit/socket is materialized; only the fixed local-root `d2bd.socket` /
# `d2bd.service` / `d2b-priv-broker.socket` / `d2b-priv-broker.service` are
# live). This rewrite declares a second host-local realm (`ops`, alongside the
# shared `work` realm from lib.nix) and proves: (1) the two realms render
# disjoint identities/principals/paths in `realm-controllers.json`; (2) that
# artifact's `runtimeState`/`invariants` confirm no live per-realm unit
# surface; (3) OS-level group membership actually enforces the declared
# per-realm `allowedUsers` boundary (a realm-scoped user is a member of only
# their own realm's public group, while a site admin is a member of both);
# (4) no `d2bd-r-*` / `d2bbr-r-*` unit ever appears live, extending
# daemon-smoke's fixed-four-unit closure check to the multi-realm case.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-host-realm-isolation";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { ... }: {
      environment.systemPackages = [
        pkgs.iproute2
        pkgs.jq
      ];

      d2b.site.usePrebuiltHostTools = false;
      # alice (lib.nix's shared `work` realm's allowedUsers) is also a site
      # admin, so she is expected to show up in EVERY realm's public group
      # (realm-access.nix: authorizedUsers = allowedUsers ++ adminUsers).
      # carol is scoped only to the new `ops` realm and must not leak into
      # `work`'s public group.
      d2b.site.adminUsers = [ "alice" ];
      users.users.carol = {
        isNormalUser = true;
        uid = 1001;
      };

      d2b.realms.ops = {
        path = "ops";
        placement = "host-local";
        allowedUsers = [ "carol" ];
        broker = {
          enable = true;
          hostMutation = true;
        };
        network = {
          mode = "declared";
          lanSubnet = "10.40.0.0/24";
          uplinkSubnet = "192.0.2.8/30";
        };
        providers.runtime = {
          type = "runtime";
          implementationId = "cloud-hypervisor";
        };
        workloads.ops-vm = {
          providerRefs.runtime = "runtime";
          config = { lib, ... }: {
            networking.hostName = lib.mkDefault "ops-vm";
          };
        };
      };
    };
  };

  testScript =
    { nodes, ... }:
    let
      cfg = nodes.machine.d2b;
      workRealmId = cfg._index.realms.byPath."work.local-root".realmId;
      opsRealmId = cfg._index.realms.byPath."ops.local-root".realmId;
      workGroup = "d2b-r-${workRealmId}";
      opsGroup = "d2b-r-${opsRealmId}";
    in
    ''
      start_all()
      machine.wait_for_unit("d2bd.service")
      machine.wait_for_unit("d2b-priv-broker.socket")

      controllers = "/etc/d2b/realm-controllers.json"
      machine.succeed(f"test -r {controllers}")

      # 1. Two distinct, non-gateway host-local realm rows, each with its own
      #    realmId/principals/paths, and NO relay/gateway artifact reappears.
      machine.succeed(
          f"jq -e '.controllers | length == 2' {controllers}"
      )
      machine.succeed(
          f"jq -e '[.controllers[].placement] | sort == [\"host-local\", \"host-local\"]' "
          f"{controllers}"
      )
      machine.succeed(
          f"jq -e '[.controllers[].realmId] | unique | length == 2' {controllers}"
      )
      machine.succeed(
          f"jq -e '[.controllers[].daemon.user] | unique | length == 2' {controllers}"
      )
      machine.succeed(
          f"jq -e '[.controllers[].paths.stateDir] | unique | length == 2' {controllers}"
      )
      work_row = machine.succeed(
          f"jq -e '.controllers[] | select(.realmId == \"${workRealmId}\")' {controllers}"
      )
      ops_row = machine.succeed(
          f"jq -e '.controllers[] | select(.realmId == \"${opsRealmId}\")' {controllers}"
      )
      assert '"realmPath":"work.local-root"' in work_row.replace(" ", "").replace("\n", "")
      assert '"realmPath":"ops.local-root"' in ops_row.replace(" ", "").replace("\n", "")

      # 2. access.allowedUsers is exactly what each realm declared, and
      #    inheritedAdminUsers carries the site admin into both rows.
      machine.succeed(
          f"jq -e '.controllers[] | select(.realmId == \"${workRealmId}\") "
          f"| .access.allowedUsers == [\"alice\"] "
          f"and (.access.inheritedAdminUsers | index(\"alice\") != null)' {controllers}"
      )
      machine.succeed(
          f"jq -e '.controllers[] | select(.realmId == \"${opsRealmId}\") "
          f"| .access.allowedUsers == [\"carol\"] "
          f"and (.access.inheritedAdminUsers | index(\"alice\") != null)' {controllers}"
      )

      # 3. Runtime state is metadata-only for BOTH child realms: no live
      #    per-realm controller/broker unit is materialized yet.
      machine.succeed(
          f"jq -e '.runtimeState == \"metadata-only\" "
          f"and .invariants.metadataOnly == true "
          f"and .invariants.noSystemdUnitsMaterialized == true "
          f"and .invariants.preservesGlobalDaemonBehavior == true "
          f"and .invariants.preservesDirectUnixSocketSemantics == true' {controllers}"
      )
      machine.succeed(
          f"jq -e '[.controllers[].localRuntime] == [null, null]' {controllers}"
      )

      # 4. No removed gateway/relay artifact or config surface has reappeared.
      machine.fail("test -e /etc/d2b/host-realm-relay-egress-policy.json")
      machine.fail("test -e /etc/d2b/gateway.json")
      machine.fail(
          "systemd-tmpfiles --cat-config | grep -F '/var/lib/d2b/gateways'"
      )

      # 5. The fixed local-root unit surface still holds with two child
      #    realms declared: no `d2bd-r-*` / `d2bbr-r-*` per-realm unit is
      #    live, extending daemon-smoke's unit-closure invariant.
      units = machine.succeed(
          "systemctl list-units --no-pager --all --plain "
          "| grep -E '^(d2b|microvm)' | awk '{print $1}' | sort"
      ).strip()
      unit_names = set(units.split())
      required = {
          "d2bd.socket",
          "d2bd.service",
          "d2b-priv-broker.socket",
          "d2b-priv-broker.service",
      }
      allowed = required | {"d2b.slice"}
      missing = required - unit_names
      assert not missing, f"required local-root units missing: {missing}"
      forbidden = unit_names - allowed
      assert not forbidden, (
          "host-realm isolation violated: unexpected root-visible d2b/microvm "
          f"unit(s) {forbidden} (a per-realm child controller/broker unit "
          "must never be PID1-materialized)"
      )

      # 6. Live OS-level group-membership isolation: carol (ops-only) must
      #    NOT be a member of work's public group, but IS a member of ops's;
      #    alice (site admin, plus work's own allowedUsers) is a member of
      #    BOTH public groups.
      machine.succeed("id -Gn carol | tr ' ' '\\n' | grep -qx '${opsGroup}'")
      machine.fail("id -Gn carol | tr ' ' '\\n' | grep -qx '${workGroup}'")
      machine.succeed("id -Gn alice | tr ' ' '\\n' | grep -qx '${workGroup}'")
      machine.succeed("id -Gn alice | tr ' ' '\\n' | grep -qx '${opsGroup}'")

      # 7. The per-realm principals themselves exist as real OS users/groups
      #    (realm-users.nix), disjoint between the two realms.
      machine.succeed("getent passwd d2bd-r-${workRealmId}")
      machine.succeed("getent passwd d2bbr-r-${workRealmId}")
      machine.succeed("getent passwd d2bd-r-${opsRealmId}")
      machine.succeed("getent passwd d2bbr-r-${opsRealmId}")
      machine.succeed(
          "test \"$(id -u d2bd-r-${workRealmId})\" != \"$(id -u d2bd-r-${opsRealmId})\""
      )
    '';
}
