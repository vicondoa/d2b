{ pkgs, self }:

let
  inherit (pkgs) lib;
  d2bLib = import ./lib.nix {
    inherit self;
    inherit lib;
  };
  acceptancePublisherKey = ''
    -----BEGIN PUBLIC KEY-----
    MCowBQYDK2VwAyEA6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw=
    -----END PUBLIC KEY-----
  '';
  providerPackage = pkgs.runCommand "d2b-acceptance-provider" {
    nativeBuildInputs = [ pkgs.coreutils ];
  } ''
    install -Dm644 ${../../tests/fixtures/provider-acceptance/provider-manifest.json} \
      "$out/share/d2b/provider/provider-manifest.json"
    install -Dm644 ${../../tests/fixtures/provider-acceptance/config-schema.json} \
      "$out/share/d2b/provider/config-schema.json"
    install -d -m755 "$out/share/d2b/provider"
    install -Dm755 ${pkgs.coreutils}/bin/true \
      "$out/bin/acceptance-controller"
    base64 -d ${../../tests/fixtures/provider-acceptance/provider-manifest.sig.b64} \
      >"$out/share/d2b/provider/provider-manifest.json.sig"
  '';
  providerCatalog = {
    providerName = "acceptance-provider";
    packageName = "d2b-acceptance-provider";
    version = "0.0.0";
    systems = [ "x86_64-linux" ];
    platform = "x86_64-linux";
    apiCompatibility = "d2b.zone.v3";
    serviceCompatibility = "d2bd.resource";
    signature = "default";
    rootEpoch = 1;
    revocationStatus = "clear";
    denyStatus = "clear";
    provenanceEvidence = "accepted";
    sbomEvidence = "accepted";
    licenseEvidence = "accepted";
    vulnerabilityEvidence = "accepted";
    conformanceAttestation = "accepted";
    supportChannel = "stable";
    supportContact = "d2b-acceptance@localhost";
    publisher = "d2b-acceptance";
    packageDigest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    executableDigest = "sha256:f84125779653dba770042fd2af2bd01299b05ae892c039c497e6b5ce45029d9c";
    manifestDigest = "sha256:5f8d852ba3ecd89883afdcf2330f3f752eb1d68a572698035177bcd4b8595e6c";
    componentDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    descriptorDigest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    configDigest = "sha256:ccb5a9d66e068ea8f4e205788589675a48e9e3754a840d8ac10120d14238e914";
  };
  providerArtifact = {
    package = providerPackage;
    type = "provider";
    catalog = providerCatalog;
  };
  acceptanceArtifactCatalogDigest =
    "sha256:${lib.concatStringsSep "" (lib.replicate 64 "a")}";
  acceptanceArtifactCatalog = pkgs.writeText "d2b-acceptance-artifact-catalog.json"
    (builtins.toJSON {
      schemaVersion = 3;
      catalogDigest = acceptanceArtifactCatalogDigest;
      entries = [
        {
          artifactId = "acceptance-provider";
          type = "provider";
          storePath = "${providerPackage}";
          packageDigest = providerCatalog.packageDigest;
          closureDigest = acceptanceArtifactCatalogDigest;
          closureSize = 0;
        }
      ];
    });
  artifacts = {
    acceptance-provider = providerArtifact;
  };
in
pkgs.testers.runNixOSTest {
  name = "d2b-unsafe-local-helper";

  nodes.machine = d2bLib.d2bDaemonNode {
    writableStore = true;
    extra = { pkgs, ... }: {
      users.users.bob = {
        isNormalUser = true;
        uid = 1001;
      };
      d2b.site.adminUsers = [ "alice" ];
      systemd.services.d2bd.environment.D2B_SKIP_KERNEL_MODULE_CHECK = "1";
      d2b.artifacts = artifacts;
      d2b._artifactCatalogV3 = lib.mkForce {
        catalogDigest = acceptanceArtifactCatalogDigest;
        path = acceptanceArtifactCatalog;
      };
      d2b._bundle.extraArtifacts.artifactCatalog = lib.mkForce {
        data = { schemaVersion = 3; catalogDigest = acceptanceArtifactCatalogDigest; entries = [ ]; };
        jsonText = builtins.readFile acceptanceArtifactCatalog;
        path = lib.mkForce acceptanceArtifactCatalog;
        installFileName = "artifact-catalog.json";
        classification = "contractPrivateNonSecret";
        sensitivity = "nonSecret";
      };
      d2b.zones = {
        local-root = {
          trustedPublishers.d2b-acceptance.signingKey = acceptancePublisherKey;
          resources = {
          alice = {
            type = "User";
            spec = {
              displayName = "Alice";
              groups = [ ];
              osUsername = "alice";
            };
          };
          host-system = {
            type = "Host";
            spec = {
              providerRef = "Provider/system-core";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
          acceptance-guest = {
            type = "Guest";
            spec = {
              providerRef = "Provider/system-core";
              defaultDomain = "system";
              allowedDomains = [ "system" ];
              budget = { };
              networkAttachments = [ ];
              deviceAttachments = [ ];
              volumeAttachmentDefaults = [ ];
            };
          };
          display-wayland = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config.runtimeVolumePolicyId = "display-wayland.wlproxy-runtime.v1";
            };
          };
          clipboard-wayland = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                hostExecutionRef = "Host/host-system";
                hostUserRef = "User/alice";
                displayWaylandRef = "Provider/display-wayland";
                guestSources = [ { guestRef = "Guest/acceptance-guest"; } ];
              };
            };
          };
          notification-desktop = {
            type = "Provider";
            spec = {
              artifactId = "acceptance-provider";
              config = {
                hostExecutionRef = "Host/host-system";
                hostUserRef = "User/alice";
                displayWaylandRef = "Provider/display-wayland";
                guestSources = [
                  {
                    guestRef = "Guest/acceptance-guest";
                    categories = [ "system.info" ];
                  }
                ];
              };
            };
          };
          };
        };
        other.parentZone = "local-root";
      };
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
          launcher.items.terminal = {
            type = "shell";
            name = "Terminal";
          };
          shell = {
            enable = true;
            defaultName = "primary";
            maxSessions = 4;
          };
        };
        workloads.extra = {
          kind = "unsafe-local";
          shell = {
            enable = true;
            defaultName = "primary";
            maxSessions = 4;
          };
        };
      };
      environment.systemPackages = [ pkgs.jq pkgs.python3 ];
    };
  };

  testScript = ''
if True:
    import json

    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/unsafe-local-helper.sock", timeout=60)
    machine.succeed("test \"$(stat -c %a /run/d2b/unsafe-local-helper.sock)\" = 660")
    machine.succeed(
        "test \"$(stat -c %G /run/d2b/unsafe-local-helper.sock)\" = d2b-unsafe-local"
    )
    machine.succeed("id -nG alice | tr ' ' '\n' | grep -qx d2b-unsafe-local")
    machine.fail("id -nG bob | tr ' ' '\n' | grep -qx d2b-unsafe-local")
    machine.succeed(
        "jq --arg path \"$D2B_MANIFEST_PATH\" "
        "'.publicManifestPath = $path' /etc/d2b/bundle.json "
        "> /run/d2b/test-bundle.json && "
        "python3 -c 'import hashlib,json,sys; "
        "p=sys.argv[1]; d=json.load(open(p)); h=dict(d); "
        "h.pop(\"bundleHash\",None); h[\"artifactHashes\"]=None; "
        "d[\"bundleHash\"]=\"sha256:\"+hashlib.sha256("
        "json.dumps(h,sort_keys=True,separators=(\",\",\":\")).encode()"
        ").hexdigest(); open(p,\"w\").write("
        "json.dumps(d,sort_keys=True,separators=(\",\",\":\")))' "
        "/run/d2b/test-bundle.json && "
        "install -o root -g d2bd -m 0640 "
        "/run/d2b/test-bundle.json /etc/d2b/bundle.json"
    )
    machine.succeed(
        "jq --arg path \"$D2B_MANIFEST_PATH\" "
        "'.artifacts.publicManifestPath = $path' /etc/d2b/daemon-config.json "
        "> /run/d2b/test-daemon-config.json && "
        "install -o root -g d2bd -m 0640 "
        "/run/d2b/test-daemon-config.json /etc/d2b/daemon-config.json"
    )
    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_until_succeeds(
        "journalctl -u d2bd.service --no-pager | grep -Eq "
        "'interaction_runtime_ready[=: ]+true'",
        timeout=60,
    )
    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone local-root --json resource list Provider "
        ">/run/d2b-interaction-providers.json"
    )
    machine.succeed(
        "jq -e '"
        "(.resources | map(select(.resourceRef == \"Provider/display-wayland\")) | length == 1) and "
        "(.resources | map(select(.resourceRef == \"Provider/clipboard-wayland\")) | length == 1) and "
        "(.resources | map(select(.resourceRef == \"Provider/notification-desktop\")) | length == 1) and "
        "(.resources[] | select(.resourceRef == \"Provider/display-wayland\") | "
        ".spec.config.runtimeVolumePolicyId == \"display-wayland.wlproxy-runtime.v1\") and "
        "(.resources[] | select(.resourceRef == \"Provider/clipboard-wayland\") | ("
        ".spec.config.hostExecutionRef == \"Host/host-system\" and "
        ".spec.config.hostUserRef == \"User/alice\" and "
        ".spec.config.displayWaylandRef == \"Provider/display-wayland\" and "
        "(.spec.config.guestSources | length == 1) and "
        ".spec.config.guestSources[0].guestRef == \"Guest/acceptance-guest\")) and "
        "(.resources[] | select(.resourceRef == \"Provider/notification-desktop\") | ("
        ".spec.config.hostExecutionRef == \"Host/host-system\" and "
        ".spec.config.hostUserRef == \"User/alice\" and "
        ".spec.config.displayWaylandRef == \"Provider/display-wayland\" and "
        "(.spec.config.guestSources | length == 1) and "
        ".spec.config.guestSources[0].guestRef == \"Guest/acceptance-guest\" and "
        ".spec.config.guestSources[0].categories == [\"system.info\"]))' "
        "/run/d2b-interaction-providers.json"
    )
    machine.succeed(
        "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "d2b --zone local-root --json resource reconcile Guest/acceptance-guest "
        ">/run/d2b-guest-reconcile.json"
    )
    machine.succeed(
        "jq -e '.authenticated == true and .ready == true and "
        ".effect == \"cloud-hypervisor-adopted\" and "
        ".resourceRef == \"Guest/acceptance-guest\"' "
        "/run/d2b-guest-reconcile.json"
    )
    for zone in ["local-root", "other"]:
        machine.succeed(
            f"test \"$(stat -c '%U:%G %a' /var/lib/d2b/zones/{zone})\" = "
            "\"root:d2bd 750\" && "
            f"test \"$(stat -c '%U:%G %a' /var/lib/d2b/zones/{zone}/audit)\" = "
            "\"d2bd:d2bd 700\" && "
            f"test \"$(stat -c '%U:%G %a' /var/lib/d2b/zones/{zone}/telemetry)\" = "
            "\"d2bd:d2bd 700\""
        )
    machine.succeed(
        "test -f /etc/d2b/zones/local-root/storage.json && "
        "test -f /etc/d2b/zones/work/storage.json && "
        "getent passwd d2b-zonert >/dev/null && "
        "getent group d2b-zonert >/dev/null && "
        "test -d /var/lib/d2b"
    )

    machine.succeed("systemctl start user@1000.service")
    alice_user = (
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "systemctl --user"
    )
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-unsafe-local-helper.service",
        timeout=60,
    )
    machine.wait_until_succeeds(
        "journalctl -u d2bd.service --no-pager | grep -q "
        "'unsafe-local helper registered'",
        timeout=60,
    )
    helper_pid = machine.succeed(
        alice_user + " show -P MainPID d2b-unsafe-local-helper.service"
    ).strip()
    machine.succeed(
        f"test \"$(readlink /proc/{helper_pid}/ns/net)\" = "
        "\"$(readlink /proc/1/ns/net)\""
    )

    machine.succeed("systemctl start user@1001.service")
    bob_user = (
        "runuser -u bob -- env XDG_RUNTIME_DIR=/run/user/1001 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1001/bus "
        "systemctl --user"
    )
    machine.wait_until_succeeds(
        bob_user
        + " show -P ConditionResult d2b-unsafe-local-helper.service | grep -qx no",
        timeout=60,
    )
    machine.fail(bob_user + " is-active d2b-unsafe-local-helper.service")

    machine.succeed(r"""
      cat > /run/d2b/cli-shell-e2e.py <<'PY'
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

pid, master = pty.fork()
if pid == 0:
    os.execv(
        "/run/current-system/sw/bin/d2b",
        ["d2b", "shell", "open", "Host/tools", "--name", "primary"],
    )
fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
attributes = termios.tcgetattr(master)
attributes[3] &= ~termios.ECHO
termios.tcsetattr(master, termios.TCSANOW, attributes)

output = bytearray()

def read_until(marker, timeout):
    deadline = time.monotonic() + timeout
    while marker not in output and time.monotonic() < deadline:
        readable, _, _ = select.select([master], [], [], 1)
        if not readable:
            continue
        try:
            chunk = os.read(master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not chunk:
            break
        output.extend(chunk)
    if marker not in output:
        raise SystemExit(
            f"real d2b shell CLI missed {marker!r}: {bytes(output)!r}"
        )

read_until(b"alice@machine:~]", 30)
output.clear()
expected = b"cli-shell-executed-canary"
escaped = "".join(f"\\x{byte:02x}" for byte in expected)
os.write(master, f"printf '{escaped}'\n".encode())
read_until(expected, 30)

os.close(master)
deadline = time.monotonic() + 15
while time.monotonic() < deadline:
    waited, status = os.waitpid(pid, os.WNOHANG)
    if waited == pid:
        break
    time.sleep(0.05)
else:
    os.kill(pid, 9)
    os.waitpid(pid, 0)
    raise SystemExit("real d2b shell CLI did not detach")
PY
      chmod 0755 /run/d2b/cli-shell-e2e.py
    """)
    machine.succeed(
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "python3 /run/d2b/cli-shell-e2e.py"
    )
    machine.sleep(3)
    machine.succeed(r"""
      cat > /run/d2b/cli-shell-attach-e2e.py <<'PY'
import errno
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

pid, master = pty.fork()
if pid == 0:
    shell_name = os.environ.get("SHELL_NAME", "primary")
    os.execv(
        "/run/current-system/sw/bin/d2b",
        ["d2b", "shell", "attach", f"ShellSession/{shell_name}"],
    )
fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
attributes = termios.tcgetattr(master)
attributes[3] &= ~termios.ECHO
termios.tcsetattr(master, termios.TCSANOW, attributes)
output = bytearray()
expected = os.environ.get("SHELL_MARKER", "cli-shell-attach-canary").encode()
escaped = "".join(f"\\x{byte:02x}" for byte in expected)
command = f"printf '{escaped}'"

def read_until(marker, timeout):
    deadline = time.monotonic() + timeout
    while marker not in output and time.monotonic() < deadline:
        readable, _, _ = select.select([master], [], [], 1)
        if not readable:
            continue
        try:
            output.extend(os.read(master, 65536))
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
    if marker not in output:
        print(bytes(output).decode(errors="replace"), file=sys.stderr)
        raise SystemExit(f"typed shell attach missed {marker!r}: {bytes(output)!r}")

read_until(b"alice@machine:~]", 30)
output.clear()
os.write(master, (command + "\n").encode())
read_until(expected, 30)
if os.environ.get("CHECK_RESIZE") == "1":
    output.clear()
    fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", 37, 101, 0, 0))
    os.kill(pid, signal.SIGWINCH)
    deadline = time.monotonic() + 30
    next_probe = 0.0
    while b"37 101" not in output and time.monotonic() < deadline:
        now = time.monotonic()
        if now >= next_probe:
            os.write(master, b"stty size\n")
            next_probe = now + 0.5
        if select.select([master], [], [], 0.5)[0]:
            try:
                output.extend(os.read(master, 65536))
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
    if b"37 101" not in output:
        raise SystemExit(f"typed shell resize missed geometry: {bytes(output)!r}")
if os.environ.get("EXIT_REMOTE") == "1":
    os.write(master, b"exit\n")
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        waited, status = os.waitpid(pid, os.WNOHANG)
        if waited == pid:
            if status != 0:
                raise SystemExit(f"typed shell CLI failed after remote exit: {status}")
            raise SystemExit(0)
        time.sleep(0.05)
    os.kill(pid, 9)
    os.waitpid(pid, 0)
    raise SystemExit("typed shell CLI did not exit after remote shell EOF")
if os.environ.get("HOLD") == "1":
    open("/run/user/1000/d2b-shell-hold.ready", "w").close()
    while True:
        waited, status = os.waitpid(pid, os.WNOHANG)
        if waited == pid:
            break
        time.sleep(1)
    if os.environ.get("EXPECT_CLI_FAILURE") == "1" and status == 0:
        raise SystemExit("typed shell transport loss returned success")
    raise SystemExit(0)
os.kill(pid, signal.SIGTERM)
os.waitpid(pid, 0)
PY
      chmod 0755 /run/d2b/cli-shell-attach-e2e.py
    """)
    shell_client = (
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "/run/current-system/sw/bin/d2b --json shell"
    )
    cli_shell = (
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "python3 /run/d2b/cli-shell-attach-e2e.py"
    )
    attach_status, attach_output = machine.execute(
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "python3 /run/d2b/cli-shell-attach-e2e.py"
    )
    if attach_status != 0:
        print(attach_output)
        print(machine.succeed("journalctl -u d2bd.service --no-pager -n 100"))
        print(machine.succeed(
            "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
            "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
            "journalctl --user -u d2b-unsafe-local-helper.service --no-pager -n 100"
        ))
        raise Exception("typed shell attach probe failed")
    machine.succeed(
        "runuser -u alice -- sh -c 'setsid sleep 300 >/dev/null 2>&1 & "
        "echo $! > /run/user/1000/unrelated-same-uid.pid'"
    )
    unrelated_pid = machine.succeed(
        "cat /run/user/1000/unrelated-same-uid.pid"
    ).strip()

    machine.succeed(
        "SHELL_MARKER=shell-roundtrip-canary "
        + cli_shell
    )
    shell_list = json.loads(machine.succeed(shell_client + " list"))
    print(shell_list)
    assert shell_list["defaultName"] == "primary"
    assert any(
        session["name"] == "primary" and not session["attached"]
        for session in shell_list["sessions"]
    )
    machine.succeed(
        "! runuser -u alice -- env "
        "D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "/run/current-system/sw/bin/d2b --zone missing --json "
        "shell status ShellSession/primary "
        ">/run/d2b/missing-zone-shell.log 2>&1 && "
        "grep -q '\"errorClass\":\"internal-error\"' "
        "/run/d2b/missing-zone-shell.log"
    )
    machine.succeed(
        "! runuser -u alice -- env "
        "D2B_PUBLIC_SOCKET=/run/d2b/public.sock "
        "XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "/run/current-system/sw/bin/d2b --zone other --json "
        "shell status ShellSession/primary "
        ">/run/d2b/other-zone-shell.log 2>&1 && "
        "grep -q '\"errorClass\":\"internal-error\"' "
        "/run/d2b/other-zone-shell.log"
    )
    json_open = json.loads(machine.succeed(
        shell_client + " open Host/tools --name exit-session"
    ))
    assert json_open["resourceRef"] == (
        "shell-terminal.d2bus.org.ShellSession/exit-session"
    )
    assert json_open["attached"] is False
    assert json_open["status"]["state"] == "detached"
    machine.succeed(
        "SHELL_NAME=exit-session SHELL_MARKER=remote-exit-canary EXIT_REMOTE=1 "
        + cli_shell
    )
    json_reopen = json.loads(machine.succeed(
        shell_client + " open Host/tools --name primary"
    ))
    assert json_reopen["resourceRef"] == (
        "shell-terminal.d2bus.org.ShellSession/primary"
    )
    assert json_reopen["attached"] is False
    machine.succeed(
        shell_client + " status ShellSession/primary | jq -e "
        "'.name == \"primary\" and .attached == false'"
    )
    machine.succeed(
        "CHECK_RESIZE=1 SHELL_MARKER=resize-canary "
        + cli_shell
    )
    machine.succeed(
        "SHELL_MARKER=reattach-continuity-canary "
        + cli_shell
    )

    machine.succeed("rm -f /run/user/1000/d2b-shell-hold.ready")
    machine.succeed(
        "SHELL_MARKER=detach-hold-canary HOLD=1 "
        + cli_shell + " >/run/user/1000/d2b-shell-hold.log 2>&1 & "
        "echo $! > /run/user/1000/d2b-shell-hold.pid"
    )
    machine.wait_for_file("/run/user/1000/d2b-shell-hold.ready", timeout=60)
    machine.succeed(
        shell_client + " status ShellSession/primary | jq -e "
        "'.name == \"primary\" and .attached == true'"
    )
    machine.succeed(
        shell_client + " detach ShellSession/primary | jq -e "
        "'.resolvedName == \"primary\" and .detached == true'"
    )
    machine.wait_until_fails(
        "kill -0 $(cat /run/user/1000/d2b-shell-hold.pid)", timeout=60
    )
    machine.succeed(
        "SHELL_MARKER=post-detach-canary "
        + cli_shell
    )

    machine.succeed("rm -f /run/user/1000/d2b-shell-hold.ready")
    machine.succeed(
        "SHELL_MARKER=hold-canary HOLD=1 EXPECT_CLI_FAILURE=1 "
        + cli_shell + " >/run/user/1000/d2b-shell-hold.log 2>&1 & "
        "echo $! > /run/user/1000/d2b-shell-hold.pid"
    )
    machine.wait_for_file("/run/user/1000/d2b-shell-hold.ready", timeout=60)
    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_until_fails(
        "kill -0 $(cat /run/user/1000/d2b-shell-hold.pid)", timeout=60
    )
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-unsafe-local-helper.service", timeout=60
    )
    machine.wait_until_succeeds(
        shell_client + " list | jq -e '.sessions | any(.name == \"primary\")'",
        timeout=60,
    )
    machine.succeed(
        "SHELL_MARKER=daemon-restart-canary "
        + cli_shell
    )

    machine.succeed(alice_user + " restart d2b-unsafe-local-helper.service")
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-unsafe-local-helper.service", timeout=60
    )
    machine.wait_until_succeeds(
        shell_client + " list | jq -e '.sessions | any(.name == \"primary\")'",
        timeout=60,
    )
    machine.succeed(
        "SHELL_MARKER=helper-adoption-canary "
        + cli_shell
    )
    machine.succeed(shell_client + " kill ShellSession/primary | jq -e '.killed == true'")
    machine.succeed(
        shell_client
        + " kill ShellSession/primary | jq -e "
        "'.name == \"primary\" and .killed == false and .state == \"killed\"'"
    )
    machine.succeed(
        shell_client
        + " detach ShellSession/primary | jq -e "
        "'.resolvedName == \"primary\" and .detached == false'"
    )
    machine.succeed(f"kill -0 {unrelated_pid}")
    machine.succeed(
        shell_client
        + " list | jq -e '.sessions | any(.name == \"exit-session\" and .state == \"killed\") and all(.name != \"primary\")'"
    )
    machine.succeed(shell_client + " open Host/tools --name primary >/dev/null")

    machine.succeed(
        "SHELL_MARKER=logout-canary "
        + cli_shell + " >/run/user/1000/logout-shell.log"
    )
    shell_scope = machine.succeed(
        alice_user
        + " list-units --state=active --plain --no-legend "
        "'d2b-unsafe-local-shell-*.scope' | awk 'NR == 1 {print $1}'"
    ).strip()
    assert shell_scope.endswith(".scope"), f"persistent shell scope missing: {shell_scope!r}"
    shell_control_group = machine.succeed(
        alice_user + f" show -P ControlGroup {shell_scope}"
    ).strip()
    shell_pid = machine.succeed(
        f"awk 'NR == 1 {{print $1}}' /sys/fs/cgroup{shell_control_group}/cgroup.procs"
    ).strip()
    machine.succeed("loginctl show-user alice -p Linger --value | grep -qx no")
    machine.succeed("systemctl stop user@1000.service")
    machine.wait_until_fails(f"test -d /proc/{shell_pid}", timeout=60)

    machine.succeed(
        "install -d -o alice -g users -m 0700 /run/user/1000 && "
        "runuser -u alice -- env "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/missing "
        "XDG_RUNTIME_DIR=/run/user/1000 "
        "setsid -f /run/current-system/sw/bin/d2b-unsafe-local-helper "
        "</dev/null >/run/d2b/no-manager-helper.log 2>&1"
    )
    machine.wait_until_succeeds(
        "! SHELL_MARKER=no-manager-canary " + cli_shell
        + " >/run/d2b/no-manager-client.log 2>&1 && "
        "grep -q provider-unavailable "
        "/run/d2b/no-manager-client.log",
        timeout=60,
    )
    machine.succeed(f"kill {unrelated_pid}")

    machine.succeed("systemctl show d2bd.service >/dev/null")
    machine.succeed("systemctl show d2b-priv-broker.service >/dev/null")
    machine.succeed("systemctl show d2b-priv-broker.socket >/dev/null")
    machine.succeed(
        "! systemctl list-units --all --no-pager --no-legend "
        "| grep -E 'd2b-unsafe-local-(helper|shell)'"
    )
  '';
}
