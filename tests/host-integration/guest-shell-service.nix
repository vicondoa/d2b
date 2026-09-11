# Type-G runNixOSTest: guest persistent-shell service wiring.
#
# Applies the component-session module directly to a NixOS test node and asserts the
# real systemd/PAM/linger boundary for the guest-local shell pool. This avoids a
# nested d2b-managed VM while still exercising NixOS module realization.
{ pkgs, self }:

let
  # The Guest target agent boots from enrollment-owner inputs (it never
  # generates them): a 32-byte ComponentSession key pair and a bundle whose
  # sha256 self-hash covers the canonical JSON without `bundleHash`.
  fixtureKeys = pkgs.runCommand "guest-shell-component-session-keys" { } ''
    mkdir -p "$out"
    printf '\001\002\003\004\005\006\007\010\011\012\013\014\015\016\017\020\021\022\023\024\025\026\027\030\031\032\033\034\035\036\037\040' > "$out/guest.key"
    printf '\130\151\257\364\120\124\227\062\313\252\355\136\135\371\263\012\155\243\034\260\345\164\053\255\132\324\241\247\150\361\246\173' > "$out/parent.pub"
  '';

  guestBundle = pkgs.runCommand "guest-shell-guest-bundle" {
    nativeBuildInputs = [ pkgs.python3 ];
  } ''
    mkdir -p "$out"
    printf '%s\n' '{"schemaVersion":"v2","site":{"allowUnsafeEastWest":false},"environments":[],"nftables":{"family":"inet","table":"d2b","chains":[],"tableHashAfterApply":null,"ownershipId":"guest-shell-service"},"networkManager":{"filePath":"/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf","matchCriteria":[],"reloadBehavior":"atomic-reload","ownership":{"owner":"root","group":"root","mode":"0644","driftPolicy":"replace"}},"hostsFile":{"startMarker":"# d2b-managed begin","endMarker":"# d2b-managed end","rule":"replace-managed-block"},"kernelModules":[],"fdOwnership":[],"cloudHypervisorCapabilities":[],"ifNameMappings":[],"ch":null,"firewallCoexistencePolicy":null}' > "$out/host.json"
    printf '%s\n' '{"schemaVersion":"v2","vms":[]}' > "$out/processes.json"
    printf '%s\n' '{"schemaVersion":"v2","publicOperations":[],"brokerOperations":[]}' > "$out/privileges.json"
    printf '%s\n' '{"_manifest":{"manifestVersion":6},"_observability":{"enabled":false,"signozUrl":"http://127.0.0.1:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318,"obsVsockCid":0,"obsVsockHostSocket":"","vmName":""}}' > "$out/vms.json"
    python3 - "$out/bundle.json" <<'PY'
    import hashlib
    import json
    import sys

    bundle = {
        "artifactHashes": None,
        "bundleVersion": 4,
        "closures": [],
        "generation": {
            "generatedAt": None,
            "generator": "guest-shell-service",
            "sourceRevision": None,
        },
        "hostPath": "host.json",
        "managedKeys": {
            "keysDir": "/var/lib/d2b/keys",
            "knownHostsPath": "/var/lib/d2b/known_hosts.d2b",
            "overrides": [],
        },
        "minijailProfiles": [],
        "privilegesPath": "privileges.json",
        "processesPath": "processes.json",
        "publicManifestPath": "vms.json",
        "schemaVersion": "v2",
    }
    canonical = json.dumps(bundle, sort_keys=True, separators=(",", ":")).encode()
    bundle["bundleHash"] = "sha256:" + hashlib.sha256(canonical).hexdigest()
    with open(sys.argv[1], "w", encoding="utf-8") as output:
        json.dump(bundle, output, sort_keys=True, separators=(",", ":"))
        output.write("\n")
    PY
  '';
in
pkgs.testers.runNixOSTest {
  name = "d2b-guest-shell-service";

  nodes.machine = { lib, pkgs, ... }: {
    imports = [
      ../../nixos-modules/component-session.nix
      ../../nixos-modules/guest-broker.nix
      {
        options.d2b.sshUser = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
        };
      }
      {
        _module.args = {
          d2bInputs = { inherit self; };
          d2bHostTools = {
            broker = self.packages.${pkgs.system}.d2b-broker-guest-static;
          };
          d2bHostToolOverrides = self.lib.d2bHostToolOverrides;
        };

        users.users.alice = {
          isNormalUser = true;
          uid = 1000;
        };

        # The host consumer supplies the target user through this guest field.
        d2b.sshUser = "alice";
        d2b.componentSession = {
          enable = lib.mkForce true;
          guestConfigPath = lib.mkForce null;
          shell = {
            enable = lib.mkForce true;
            defaultName = lib.mkForce "default";
            maxSessions = lib.mkForce 8;
            maxAttached = lib.mkForce 1;
          };
        };

        # The Guest target agent binds an AF_VSOCK ComponentSession listener.
        # The lane's QEMU ships vhost-vsock-pci and now passes /dev/vhost-vsock
        # into the build sandbox, so this node can carry the same device the
        # enrolled Guest gets, plus a fixture bundle and key pair installed at
        # the production owner/mode the resolver verifies (root:d2bd 0640).
        virtualisation.qemu.options = [ "-device" "vhost-vsock-pci,guest-cid=3" ];
        boot.kernelModules = [ "vmw_vsock_virtio_transport" ];

        environment.etc."d2b/component-session/guest.key".source =
          "${fixtureKeys}/guest.key";
        environment.etc."d2b/component-session/parent.pub".source =
          "${fixtureKeys}/parent.pub";

        d2b.componentSession.localPrivateKeyPath =
          "/etc/d2b/component-session/guest.key";
        d2b.componentSession.parentPublicKeyPath =
          "/etc/d2b/component-session/parent.pub";
        d2b.componentSession.bundlePath = "/var/lib/d2b/guest-bundle/bundle.json";
        d2b.guestBroker.bundlePath = "/var/lib/d2b/guest-bundle/bundle.json";

        systemd.services.d2b-install-guest-bundle = {
          requiredBy = [ "d2bd-guest.service" "d2b-broker-guest.service" ];
          before = [ "d2bd-guest.service" "d2b-broker-guest.service" ];
          serviceConfig.Type = "oneshot";
          script = ''
            install -d -o root -g d2bd -m 0750 /var/lib/d2b/guest-bundle
            for file in bundle.json host.json processes.json privileges.json; do
              install -o root -g d2bd -m 0640 \
                ${guestBundle}/"$file" /var/lib/d2b/guest-bundle/"$file"
            done
            install -o root -g d2bd -m 0644 \
              ${guestBundle}/vms.json /var/lib/d2b/guest-bundle/vms.json
          '';
        };

        system.stateVersion = "25.11";
      }
    ];
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target", timeout=180)

    # The Guest target agent must boot from the enrolled bundle and key pair
    # and reach its AF_VSOCK listener; a bundle or key it cannot read fails
    # closed here instead of restart-looping unnoticed.
    machine.wait_for_unit("d2bd-guest.service", timeout=120)
    machine.succeed("systemctl is-active --quiet d2bd-guest.service")
    machine.wait_until_succeeds(
        "journalctl -u d2bd-guest.service --no-pager -b "
        "| grep -F 'Guest ComponentSession listener bound'",
        timeout=60,
    )
    machine.fail(
        "journalctl --no-pager -b "
        "| grep -F 'Guest process bundle validation failed'"
    )

    # The shell pool daemon is declared but dormant: the target-local Process
    # owner starts or adopts the pool.
    machine.succeed("systemctl cat d2b-shpool-daemon.service")
    machine.succeed(
        "test \"$(systemctl show -P PAMName d2b-shpool-daemon.service)\" = d2b-shpool-daemon"
    )
    machine.succeed(
        "test \"$(systemctl show -P User d2b-shpool-daemon.service)\" = alice"
    )
    machine.succeed(
        "test \"$(systemctl show -P KillMode d2b-shpool-daemon.service)\" = control-group"
    )
    machine.succeed(
        "test \"$(systemctl show -P Delegate d2b-shpool-daemon.service)\" = yes"
    )
    machine.succeed(
        "! find /etc/systemd/system -path '*wants/d2b-shpool-daemon.service' | grep -q ."
    )
    machine.fail("systemctl is-active --quiet d2b-shpool-daemon.service")

    pam_file = "/etc/pam.d/d2b-shpool-daemon"
    machine.succeed(f"test -f {pam_file}")
    pam = machine.succeed(f"cat {pam_file}")
    assert "pam_loginuid.so" in pam, pam
    assert "pam_systemd.so" not in pam, (
        "d2b-shpool-daemon must not start a pam_systemd session; "
        "that would migrate the daemon out of the delegated service cgroup"
    )

    machine.succeed("test -f /var/lib/systemd/linger/alice")
    machine.succeed("id -u alice | grep -qx 1000")
  '';
}
