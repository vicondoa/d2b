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
      systemd.services.d2bd.environment.D2B_SKIP_KERNEL_MODULE_CHECK = "1";
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
    os.execv(
        "/run/current-system/sw/bin/d2b",
        ["d2b", "shell", "attach", "ShellSession/primary"],
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
if os.environ.get("HOLD") == "1":
    open("/run/user/1000/d2b-shell-hold.ready", "w").close()
    while True:
        waited, _status = os.waitpid(pid, os.WNOHANG)
        if waited == pid:
            break
        time.sleep(1)
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
        "SHELL_MARKER=hold-canary HOLD=1 "
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
    machine.succeed(f"kill -0 {unrelated_pid}")
    machine.succeed(shell_client + " list | jq -e '.sessions | length == 0'")

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
