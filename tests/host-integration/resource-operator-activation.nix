# Type-G runNixOSTest: authenticated Resource operator and daemon census.
#
# This fixture is intentionally separate from the native controller canaries:
# it reaches the installed d2b CLI, public socket, systemd restart boundary,
# and the exact daemon-only unit surface in a real NixOS guest.
{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-resource-operator-activation";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { pkgs, ... }: {
      environment.systemPackages = [ pkgs.jq ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2b-priv-broker.socket")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
    machine.succeed("runuser -u alice -- d2b auth status --json >/run/d2b-auth-before.json")

    # The authenticated operator path must reach the public Resource API for
    # every Wave 6 type, even when the default bundle has no row of that type.
    for resource_type in ["Volume", "Network", "Device", "Guest"]:
        path = f"/run/d2b-resource-{resource_type.lower()}-before.json"
        machine.succeed(
            f"runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone work --json resource list "
            f"{resource_type} >{path}"
        )
        machine.succeed(f"jq -e '.resources | type == \"array\"' {path}")

    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/public.sock")
    machine.succeed("runuser -u alice -- d2b auth status --json >/run/d2b-auth-after.json")
    for resource_type in ["Volume", "Network", "Device", "Guest"]:
        path = f"/run/d2b-resource-{resource_type.lower()}-after.json"
        machine.succeed(
            f"runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
            f"d2b --zone work --json resource list "
            f"{resource_type} >{path}"
        )
        machine.succeed(f"jq -e '.resources | type == \"array\"' {path}")

    units = machine.succeed(
        "systemctl list-units --no-pager --all --plain "
        "| grep -E '^(d2b|microvm)' | awk '{print $1}' | sort"
    ).strip()
    unit_names = set(units.split())
    required = {
        "d2bd.service",
        "d2b-priv-broker.socket",
        "d2b-priv-broker.service",
    }
    allowed = required | {"d2b.slice"}
    assert required <= unit_names, f"daemon-only units missing: {required - unit_names}"
    assert unit_names <= allowed, f"unexpected d2b/microvm units: {unit_names - allowed}"

    # Provider packages are code loaded by d2bd, never persistent services.
    provider_units = machine.succeed(
        "systemctl list-unit-files --no-pager --no-legend "
        "| awk '{print $1}' | grep -E '(^|[-.])d2b[-.]provider|provider[-.]d2b' || true"
    ).strip()
    assert not provider_units, f"Provider-owned persistent units found: {provider_units}"
  '';
}
