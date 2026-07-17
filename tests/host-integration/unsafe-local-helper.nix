{ pkgs, self }:

let
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-unsafe-local-helper";

  nodes.machine = d2bLib.d2bDaemonNode {
    extra = { pkgs, ... }: {
      users.users.bob = {
        isNormalUser = true;
        uid = 1001;
      };
      d2b.site.adminUsers = [ "alice" ];
      d2b.realms.host = {
        allowedUsers = [ "alice" ];
        policy.allowUnsafeLocal = true;
        workloads.tools = {
          kind = "unsafe-local";
          launcher.items.probe = {
            type = "exec";
            name = "Probe";
            argv = [ "true" ];
          };
        };
      };
      environment.systemPackages = [ pkgs.python3 ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.fail("test -e /run/d2b/unsafe-local-helper.sock")
    machine.succeed("id -nG alice | tr ' ' '\n' | grep -qx d2b-unsafe-local")
    machine.fail("id -nG bob | tr ' ' '\n' | grep -qx d2b-unsafe-local")

    machine.succeed("systemctl start user@1000.service")
    alice_user = (
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "systemctl --user"
    )
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-runtime-systemd-user.socket",
        timeout=60,
    )
    endpoint = "/run/d2b/u/1000/runtime-agent.sock"
    machine.wait_for_file(endpoint)
    machine.succeed(f"test -S {endpoint}")
    machine.succeed(f"test \"$(stat -c %a {endpoint})\" = 600")
    machine.succeed(f"test \"$(stat -c %U {endpoint})\" = alice")
    machine.fail("test -e /run/d2b/u/1000/unsafe-local-helper.sock")

    machine.fail(
        "runuser -u alice -- "
        "/run/current-system/sw/bin/d2b-unsafe-local-helper"
    )
    machine.succeed(
        "runuser -u alice -- python3 -c "
        "'import socket; s=socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET); "
        f"s.connect(\"{endpoint}\")'"
    )
    machine.wait_until_succeeds(
        "journalctl _SYSTEMD_USER_UNIT=d2b-runtime-systemd-user.service "
        "_UID=1000 --no-pager "
        "| grep -q component-session-unavailable",
        timeout=60,
    )

    machine.succeed("systemctl start user@1001.service")
    bob_user = (
        "runuser -u bob -- env XDG_RUNTIME_DIR=/run/user/1001 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1001/bus "
        "systemctl --user"
    )
    machine.wait_until_succeeds(
        bob_user
        + " show -P ConditionResult d2b-runtime-systemd-user.socket | grep -qx no",
        timeout=60,
    )
    machine.fail(bob_user + " is-active d2b-runtime-systemd-user.socket")
    machine.fail("test -e /run/d2b/u/1001/runtime-agent.sock")
  '';
}
