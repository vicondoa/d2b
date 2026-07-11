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
      cat > /run/d2b/shell-client.py <<'PY'
import base64
import json
import os
import socket
import struct
import sys
import time

SOCKET = "/run/d2b/public.sock"
TARGET = "tools.host.d2b"

def send_frame(conn, value):
    body = json.dumps(value, separators=(",", ":")).encode()
    conn.sendall(struct.pack("<I", len(body)) + body)

def recv_frame(conn):
    packet = conn.recv(1048580)
    if len(packet) < 4:
        raise RuntimeError("short public frame")
    size = struct.unpack("<I", packet[:4])[0]
    if size != len(packet) - 4:
        raise RuntimeError("invalid public frame length")
    return json.loads(packet[4:])

def connect():
    conn = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
    conn.connect(SOCKET)
    send_frame(conn, {
        "type": "hello",
        "clientVersion": ">=0.4.0, <0.5.0",
        "supportedFeatures": [
            "typed-errors",
            "configured-launch-v1",
            "unsafe-local-provider-v1",
            "unsafe-local-shell-v1"
        ]
    })
    hello = recv_frame(conn)
    if hello.get("type") != "helloOk":
        raise RuntimeError("hello rejected")
    if "unsafe-local-shell-v1" not in hello.get("capabilities", []):
        raise RuntimeError("unsafe-local shell feature missing")
    return conn

def shell(conn, op, args, op_id):
    send_frame(conn, {"type": "shell", "op": op, "args": args, "opId": op_id})
    response = recv_frame(conn)
    if response.get("type") == "error":
        print(json.dumps(response), file=sys.stderr)
        raise RuntimeError(response["error"]["kind"])
    if response.get("type") != "shellResponse":
        raise RuntimeError("unexpected shell response")
    return response["result"]

def attach(mode):
    conn = connect()
    attached = shell(conn, "attach", {
        "vm": TARGET,
        "name": "primary",
        "force": False,
        "initialTerminalSize": {"rows": 24, "cols": 80}
    }, 1)
    session = attached["session"]
    shell(conn, "resize", {
        "session": session, "rows": 33, "cols": 101, "opId": 1
    }, 2)
    if mode == "hold":
        open("/run/user/1000/d2b-shell-hold.ready", "w").close()
        cursor = 0
        op_id = 3
        while True:
            result = shell(conn, "readOutput", {
                "session": session,
                "stream": "stdout",
                "offset": cursor,
                "maxLen": 65536,
                "wait": True,
                "timeoutMs": 250
            }, op_id)
            cursor = result["nextOffset"]
            op_id += 1
    command = os.environ.get("SHELL_COMMAND", "printf shell-roundtrip-canary")
    expected = command.split()[-1].encode()
    data = (command + "\n").encode()
    shell(conn, "writeStdin", {
        "session": session,
        "offset": 0,
        "chunkBase64": base64.b64encode(data).decode(),
        "eof": False
    }, 3)
    cursor = 0
    output = bytearray()
    deadline = time.monotonic() + 15
    op_id = 4
    while time.monotonic() < deadline:
        chunk = shell(conn, "readOutput", {
            "session": session,
            "stream": "stdout",
            "offset": cursor,
            "maxLen": 65536,
            "wait": True,
            "timeoutMs": 250
        }, op_id)
        output.extend(base64.b64decode(chunk["dataBase64"]))
        cursor = chunk["nextOffset"]
        op_id += 1
        if expected in output:
            break
    else:
        raise RuntimeError("command output timeout")
    print(output.decode(errors="replace"))
    shell(conn, "closeAttach", {"session": session}, op_id)

def management(op):
    conn = connect()
    args = {"vm": TARGET}
    if op in ("detach", "kill"):
        args["name"] = "primary"
    print(json.dumps(shell(conn, op, args, 1), sort_keys=True))

if sys.argv[1] in ("attach", "hold"):
    attach(sys.argv[1])
else:
    management(sys.argv[1])
PY
      chmod 0755 /run/d2b/shell-client.py
    """)

    shell_client = (
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "python3 /run/d2b/shell-client.py"
    )
    machine.succeed(r"""
      cat > /run/d2b/cli-shell-e2e.py <<'PY'
import errno
import os
import pty
import select
import time

pid, master = pty.fork()
if pid == 0:
    os.execv(
        "/run/current-system/sw/bin/d2b",
        ["d2b", "shell", "tools.host.d2b", "--name", "cli-e2e"],
    )

os.write(master, b"printf cli-shell-canary\\n")
deadline = time.monotonic() + 30
output = bytearray()
while b"cli-shell-canary" not in output and time.monotonic() < deadline:
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

if b"cli-shell-canary" not in output:
    raise SystemExit("real d2b shell CLI did not round-trip terminal output")

os.write(master, b"\\x00\\x11")
deadline = time.monotonic() + 15
while time.monotonic() < deadline:
    waited, status = os.waitpid(pid, os.WNOHANG)
    if waited == pid:
        if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
            raise SystemExit(f"real d2b shell CLI exited with status {status}")
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
    machine.succeed(
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/user/1000 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "d2b shell tools.host.d2b kill --name cli-e2e --json "
        "| jq -e '.result == \"killed\"'"
    )
    machine.succeed(
        "runuser -u alice -- sh -c 'setsid sleep 300 >/dev/null 2>&1 & "
        "echo $! > /run/user/1000/unrelated-same-uid.pid'"
    )
    unrelated_pid = machine.succeed(
        "cat /run/user/1000/unrelated-same-uid.pid"
    ).strip()

    machine.succeed(
        "SHELL_COMMAND='printf shell-roundtrip-canary' "
        + shell_client + " attach | grep -q shell-roundtrip-canary"
    )
    machine.succeed(
        shell_client + " list | jq -e "
        "'.defaultName == \"primary\" and "
        "(.sessions | any(.name == \"primary\" and .attached == false))'"
    )
    machine.succeed(
        "SHELL_COMMAND='printf reattach-continuity-canary' "
        + shell_client + " attach | grep -q reattach-continuity-canary"
    )

    machine.succeed("rm -f /run/user/1000/d2b-shell-hold.ready")
    machine.succeed(
        shell_client + " hold >/run/user/1000/d2b-shell-hold.log 2>&1 & "
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
        "SHELL_COMMAND='printf daemon-restart-canary' "
        + shell_client + " attach | grep -q daemon-restart-canary"
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
        "SHELL_COMMAND='printf helper-adoption-canary' "
        + shell_client + " attach | grep -q helper-adoption-canary"
    )
    machine.succeed(shell_client + " kill | jq -e '.killed == true'")
    machine.succeed(f"kill -0 {unrelated_pid}")
    machine.succeed(shell_client + " list | jq -e '.sessions | length == 0'")

    machine.succeed(
        "SHELL_COMMAND='printf logout-canary' "
        + shell_client + " attach >/run/user/1000/logout-shell.log"
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
        "! " + shell_client + " attach >/run/d2b/no-manager-client.log 2>&1 && "
        "grep -q unsafe-local-shell-user-manager-unavailable "
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
