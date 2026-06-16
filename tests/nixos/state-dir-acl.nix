# Type-G runNixOSTest: nixling state-dir traversal ACL.
#
# Boots the shared daemon node and asserts the live activation posture for
# /var/lib/nixling: launcher-group members get traverse-only access to the
# state-dir parent, can reach/read their known per-VM key, and outsiders cannot
# traverse or list the protected key tree.
{ pkgs, self }:

let
  nixlingLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
in
pkgs.testers.runNixOSTest {
  name = "nixling-state-dir-acl";

  nodes.machine = nixlingLib.nixlingDaemonNode {
    extra = {
      users.users.mallory = {
        isNormalUser = true;
        uid = 1001;
      };
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")

    state_dir = "/var/lib/nixling"
    keys_dir = f"{state_dir}/keys"
    key_path = f"{keys_dir}/corp-vm_ed25519"

    machine.wait_for_file(key_path)
    machine.succeed(f"test -f {key_path}")
    print("generated key entries:\n" + machine.succeed(f"ls -l {keys_dir}"))

    state_acl = machine.succeed(f"getfacl -p {state_dir}")
    print("state-dir ACL:\n" + state_acl)
    assert "group:nixling:--x" in state_acl, (
        "expected g:nixling:--x traversal ACL on /var/lib/nixling"
    )

    keys_posture = machine.succeed(f"stat -c '%a %U %G' {keys_dir}").strip()
    assert keys_posture == "710 root nixling", (
        f"expected /var/lib/nixling/keys to be 0710 root:nixling, got {keys_posture}"
    )

    machine.succeed("id -nG alice | grep -qw nixling")
    machine.fail("id -nG mallory | grep -qw nixling")

    # 1. Launcher member CAN stat a known per-VM key (state-dir traversal works).
    machine.succeed(f"sudo -u alice stat {key_path}")

    # 2. Launcher member CAN read the key (per-key group read is effective).
    machine.succeed(f"sudo -u alice cat {key_path} >/dev/null")

    # 3. Non-launcher CANNOT stat the key (no traversal).
    machine.fail(f"sudo -u mallory stat {key_path}")

    # 4. Non-launcher CANNOT list the keys directory.
    machine.fail(f"sudo -u mallory ls {keys_dir}")

    # 5. Launcher member CANNOT list the state-dir contents; g:nixling is --x only.
    machine.fail(f"sudo -u alice ls {state_dir}")
  '';
}
