# Type-G runNixOSTest: authenticated Resource operator and framework census.
#
# This fixture is intentionally separate from the native controller canaries:
# it reaches the installed d2b CLI, public socket, systemd restart boundary,
# and the framework-declared daemon unit surface in a real NixOS guest. The
# census does not sweep every d2b-prefixed unit on an operator host, because
# optional or managed infrastructure is outside this fixture's ownership.
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

    declared = set(
        machine.succeed("cat /etc/d2b/daemon-acceptance-units").split()
    )
    required = {
        "d2bd.service",
        "d2b-priv-broker.socket",
        "d2b-priv-broker.service",
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
    assert required <= unit_names, (
        f"framework daemon units missing: {required - unit_names}"
    )

    # Provider packages are code loaded by d2bd, never framework-declared
    # persistent services. Optional or managed host units are outside this
    # fixture's census.
    provider_units = sorted(
        unit
        for unit in declared
        if "provider" in unit and (unit.endswith(".service") or unit.endswith(".socket"))
    )
    assert not provider_units, f"Provider-owned persistent units found: {provider_units}"
  '';
}
