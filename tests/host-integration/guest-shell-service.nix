# Type-G runNixOSTest: guest persistent-shell service wiring.
#
# Applies the guest-control module directly to a NixOS test node and asserts the
# real systemd/PAM/linger boundary for the guest-local shell pool. This avoids a
# nested nixling-managed VM while still exercising NixOS module realization.
{ pkgs, self }:

pkgs.testers.runNixOSTest {
  name = "nixling-guest-shell-service";

  nodes.machine = { lib, ... }: {
    imports = [
      ../../nixos-modules/guest-control.nix
      {
        _module.args = {
          nixlingInputs = { inherit self; };
        };

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        nixling.guestControl = {
          enable = lib.mkForce true;
          exec = {
            enable = lib.mkForce true;
            execUser = lib.mkForce "alice";
            detachedMaxRuntimeSec = lib.mkForce 0;
            interactiveMaxRuntimeSec = lib.mkForce 0;
          };
          guestConfigPath = lib.mkForce null;
          usbipPath = lib.mkForce null;
          shell = {
            enable = lib.mkForce true;
            defaultName = lib.mkForce "default";
            maxSessions = lib.mkForce 8;
            maxAttached = lib.mkForce 1;
          };
        };

        system.stateVersion = "25.11";
      }
    ];
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")

    # The shell pool daemon is declared but dormant: guestd owns when it starts
    # or adopts the pool.
    machine.succeed("systemctl cat nixling-shpool-daemon.service")
    machine.succeed(
        "test \"$(systemctl show -P PAMName nixling-shpool-daemon.service)\" = nixling-shpool-daemon"
    )
    machine.succeed(
        "test \"$(systemctl show -P User nixling-shpool-daemon.service)\" = alice"
    )
    machine.succeed(
        "test \"$(systemctl show -P KillMode nixling-shpool-daemon.service)\" = control-group"
    )
    machine.succeed(
        "test \"$(systemctl show -P Delegate nixling-shpool-daemon.service)\" = yes"
    )
    machine.succeed(
        "! find /etc/systemd/system -path '*wants/nixling-shpool-daemon.service' | grep -q ."
    )
    machine.fail("systemctl is-active --quiet nixling-shpool-daemon.service")

    pam_file = "/etc/pam.d/nixling-shpool-daemon"
    machine.succeed(f"test -f {pam_file}")
    pam = machine.succeed(f"cat {pam_file}")
    assert "pam_loginuid.so" in pam, pam
    assert "pam_systemd.so" not in pam, (
        "nixling-shpool-daemon must not start a pam_systemd session; "
        "that would migrate the daemon out of the delegated service cgroup"
    )

    machine.succeed("test -f /var/lib/systemd/linger/alice")
    machine.succeed("id -u alice | grep -qx 1000")
  '';
}
