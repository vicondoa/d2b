# Type-G runNixOSTest: host remains isolated from realm relay credentials.
{ pkgs, self }:

let
  nixlingLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "nixling-host-realm-isolation";

  nodes.machine = nixlingLib.nixlingDaemonNode {
    extra = { ... }: {
      environment.systemPackages = [
        pkgs.iproute2
        pkgs.jq
      ];

      nixling.site.usePrebuiltHostTools = false;
      nixling.gateways.work = {
        env = "work";
        index = 20;
        relay.namespace = "relns-example.servicebus.windows.net";
        relay.entity = "hc-nixling-display";
        aca = {
          subscription = "00000000-0000-0000-0000-000000000000";
          resourceGroup = "rg-nixling-centralus";
          sandboxGroup = "casbx-nixling-demo";
          region = "centralus";
          image = "registry.example.invalid/nixling-wayland:mi";
          diskName = "nixling-wayland-mi";
        };
      };
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("nixlingd.service")
    machine.wait_for_unit("nixling-priv-broker.socket")

    policy = "/etc/nixling/host-realm-relay-egress-policy.json"
    machine.succeed(f"test -r {policy}")
    machine.succeed(
      f"jq -e '.mode == \"host-realm-relay-deny\" "
      f"and (.gatewayInterfaces == [\"work-l20\"]) "
      f"and (.diagnostics.redacted == true) "
      f"and (.diagnostics.rateLimited == true)' {policy}"
    )
    policy_forbidden = [
      "relns-example.servicebus.windows.net",
      "hc-nixling-display",
      "registry.example.invalid/nixling-wayland:mi",
      "/var/lib/nixling/gateways/work/credential.sealed.json",
      "/var/lib/nixling/gateways/work/seal.key",
      "SharedAccessKey",
    ]
    for token in policy_forbidden:
      machine.fail(f"grep -F {repr(token)} {policy}")

    runtime_forbidden = policy_forbidden + ["NIXLING_RELAY_"]

    machine.fail("test -e /etc/nixling/gateway.json")
    machine.fail("systemd-tmpfiles --cat-config | grep -F '/var/lib/nixling/gateways/work'")

    pids = machine.succeed("pgrep -x nixlingd").strip().split()
    assert pids, "nixlingd pid missing"
    machine.succeed("systemctl start nixling-priv-broker.service")
    broker_pid = machine.succeed(
      "for i in $(seq 1 50); do "
      "pid=$(systemctl show -p MainPID --value nixling-priv-broker.service); "
      "if [ -n \"$pid\" ] && [ \"$pid\" != 0 ]; then echo \"$pid\"; exit 0; fi; "
      "sleep 0.2; done; exit 1"
    ).strip()
    pids.append(broker_pid)

    for pid in pids:
      env = machine.succeed(f"tr '\\0' '\\n' < /proc/{pid}/environ || true")
      cmd = machine.succeed(f"tr '\\0' ' ' < /proc/{pid}/cmdline || true")
      fds = machine.succeed(f"ls -l /proc/{pid}/fd || true")
      for token in runtime_forbidden:
        assert token not in env, f"forbidden token leaked in environ for pid {pid}"
        assert token not in cmd, f"forbidden token leaked in cmdline for pid {pid}"
        assert token not in fds, f"forbidden token leaked in fd table for pid {pid}"

    sockets = machine.succeed("ss -Htanp || true")
    assert "servicebus.windows.net" not in sockets
    assert "nixling-provider-relay" not in sockets
    assert "nixling-gateway-relay" not in sockets
  '';
}
