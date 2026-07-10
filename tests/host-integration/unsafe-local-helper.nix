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
      environment.systemPackages = [ pkgs.jq pkgs.python3 ];
    };
  };

  testScript = ''
    start_all()
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/unsafe-local-helper.sock", timeout=60)
    machine.succeed("test \"$(stat -c %a /run/d2b/unsafe-local-helper.sock)\" = 660")
    machine.succeed(
        "test \"$(stat -c %G /run/d2b/unsafe-local-helper.sock)\" = d2b-unsafe-local"
    )
    machine.succeed("id -nG alice | tr ' ' '\\n' | grep -qx d2b-unsafe-local")
    machine.fail("id -nG bob | tr ' ' '\\n' | grep -qx d2b-unsafe-local")

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

    machine.succeed("systemctl restart d2bd.service")
    machine.wait_for_unit("d2bd.service")
    machine.wait_for_file("/run/d2b/unsafe-local-helper.sock", timeout=60)
    machine.wait_until_succeeds(
        alice_user + " is-active d2b-unsafe-local-helper.service",
        timeout=60,
    )
    machine.wait_until_succeeds(
        "test \"$(journalctl -u d2bd.service --no-pager "
        "| grep -c 'unsafe-local helper registered')\" -ge 2",
        timeout=60,
    )

    machine.succeed("systemctl stop d2bd.service")
    machine.succeed(alice_user + " stop d2b-unsafe-local-helper.service")
    machine.succeed("rm -f /run/d2b/unsafe-local-helper.sock")
    machine.succeed(
        "cat > /run/d2b/helper-mock.py <<'PY'\n"
        "import grp, json, os, socket, struct, time\n"
        "path = '/run/d2b/unsafe-local-helper.sock'\n"
        "def recv_frame(conn):\n"
        "    data = conn.recv(262149)\n"
        "    if len(data) < 4:\n"
        "        raise RuntimeError('short frame')\n"
        "    size = struct.unpack('<I', data[:4])[0]\n"
        "    if size != len(data) - 4:\n"
        "        raise RuntimeError('bad frame length')\n"
        "    return json.loads(data[4:])\n"
        "def send_frame(conn, value):\n"
        "    data = json.dumps(value, separators=(',', ':')).encode()\n"
        "    conn.sendall(struct.pack('<I', len(data)) + data)\n"
        "def accept_helper(listener):\n"
        "    conn, _ = listener.accept()\n"
        "    hello = recv_frame(conn)\n"
        "    generation = hello['payload']['generation']\n"
        "    send_frame(conn, {'type':'helloAccepted','payload':{\n"
        "        'protocolVersion':1,'generation':generation,\n"
        "        'heartbeatIntervalSecs':5,'operationTimeoutSecs':30}})\n"
        "    snapshot = recv_frame(conn)\n"
        "    return conn, snapshot\n"
        "listener = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)\n"
        "listener.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 262144)\n"
        "listener.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 262144)\n"
        "listener.bind(path)\n"
        "os.chmod(path, 0o660)\n"
        "os.chown(path, -1, grp.getgrnam('d2b-unsafe-local').gr_gid)\n"
        "listener.listen(8)\n"
        "open('/run/d2b/helper-mock.ready', 'w').close()\n"
        "conn, snapshot = accept_helper(listener)\n"
        "open('/run/d2b/helper-snapshot-1.json', 'w').write(json.dumps(snapshot))\n"
        "request = {'type':'launch','payload':{\n"
        "  'requestId':41,'operationId':'op-host-scope-1',\n"
        "  'workload':{'workloadId':'tools','realmId':'host',\n"
        "    'realmPath':['host'],'canonicalTarget':'tools.host.d2b'},\n"
        "  'itemId':'probe','argv':['sh','-c',\n"
        "    'printf \"stdout=%s\\\\nstderr=%s\\\\nenv=%s\\\\ncwd=%s\\\\n\" '\n"
        "    '\"$(readlink /proc/$$/fd/1)\" \"$(readlink /proc/$$/fd/2)\" '\n"
        "    '\"$D2B_MANAGER_CANARY\" \"$PWD\" '\n"
        "    '> /run/user/1000/d2b-helper-child-state; sleep 60'],\n"
        "  'graphical':False}}\n"
        "send_frame(conn, request)\n"
        "response = recv_frame(conn)\n"
        "open('/run/d2b/helper-launch-response.json', 'w').write(json.dumps(response))\n"
        "conn.close()\n"
        "conn, snapshot = accept_helper(listener)\n"
        "open('/run/d2b/helper-snapshot-2.json', 'w').write(json.dumps(snapshot))\n"
        "while os.path.exists('/run/d2b/keep-helper-connected'):\n"
        "    send_frame(conn, {'type':'heartbeat','payload':{\n"
        "      'generation':snapshot['payload']['generation'],'sequence':1}})\n"
        "    try:\n"
        "        recv_frame(conn)\n"
        "    except Exception:\n"
        "        break\n"
        "    time.sleep(1)\n"
        "conn.close()\n"
        "conn, snapshot = accept_helper(listener)\n"
        "send_frame(conn, {'type':'launch','payload':{\n"
        "  'requestId':42,'operationId':'op-no-user-manager',\n"
        "  'workload':{'workloadId':'tools','realmId':'host',\n"
        "    'realmPath':['host'],'canonicalTarget':'tools.host.d2b'},\n"
        "  'itemId':'probe','argv':['true'],'graphical':False}})\n"
        "response = recv_frame(conn)\n"
        "open('/run/d2b/helper-no-manager-response.json', 'w').write(json.dumps(response))\n"
        "conn.close()\n"
        "PY\n"
        "chown d2bd:d2bd /run/d2b/helper-mock.py"
    )
    machine.succeed(
        "runuser -u d2bd -- python3 /run/d2b/helper-mock.py "
        ">/run/d2b/helper-mock.log 2>&1 & "
        "echo $! > /run/d2b/helper-mock.pid"
    )
    machine.wait_for_file("/run/d2b/helper-mock.ready", timeout=60)
    machine.succeed(
        alice_user + " set-environment D2B_MANAGER_CANARY=manager-canary"
    )
    machine.succeed("touch /run/d2b/keep-helper-connected")
    machine.succeed(alice_user + " start d2b-unsafe-local-helper.service")
    machine.wait_for_file("/run/d2b/helper-launch-response.json", timeout=60)
    machine.succeed(
        "jq -e '.type == \"operation\" and "
        ".payload.disposition == \"committed\" and "
        ".payload.scope.kind == \"launcher-app\"' "
        "/run/d2b/helper-launch-response.json "
        "|| { cat /run/d2b/helper-launch-response.json >&2; exit 1; }"
    )
    invocation = machine.succeed(
        "jq -r .payload.scope.invocationId /run/d2b/helper-launch-response.json"
    ).strip()
    scope = machine.succeed(
        alice_user
        + " list-units --state=active --plain --no-legend "
        "'d2b-unsafe-local-app-*.scope' | awk 'NR == 1 {print $1}'"
    ).strip()
    assert scope.endswith(".scope"), f"unsafe-local app scope missing: {scope!r}"
    machine.succeed(
        alice_user + f" show -P InvocationID {scope} | grep -qx {invocation}"
    )
    machine.wait_until_succeeds(
        "grep -qx 'cwd=/home/alice' /run/user/1000/d2b-helper-child-state",
        timeout=60,
    )
    machine.succeed(
        "grep -qx 'stdout=/dev/null' /run/user/1000/d2b-helper-child-state"
    )
    machine.succeed(
        "grep -qx 'stderr=/dev/null' /run/user/1000/d2b-helper-child-state"
    )
    machine.succeed(
        "grep -qx 'env=manager-canary' /run/user/1000/d2b-helper-child-state"
    )
    control_group = machine.succeed(
        alice_user + f" show -P ControlGroup {scope}"
    ).strip()
    app_pid = machine.succeed(
        f"awk 'NR == 1 {{print $1}}' /sys/fs/cgroup{control_group}/cgroup.procs"
    ).strip()
    machine.succeed(f"test -d /proc/{app_pid}")
    machine.wait_for_file("/run/d2b/helper-snapshot-2.json", timeout=60)
    machine.succeed(
        f"jq -e --arg invocation {invocation} "
        "'.payload.scopes | any("
        ".scope.invocationId == $invocation and .state == \"active\")' "
        "/run/d2b/helper-snapshot-2.json"
    )

    machine.succeed("loginctl show-user alice -p Linger --value | grep -qx no")
    machine.succeed("rm -f /run/d2b/keep-helper-connected")
    machine.succeed("systemctl stop user@1000.service")
    machine.wait_until_fails(f"test -d /proc/{app_pid}", timeout=60)

    machine.succeed(
        "runuser -u alice -- env "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/missing "
        "XDG_RUNTIME_DIR=/run/user/1000 "
        "/run/current-system/sw/bin/d2b-unsafe-local-helper "
        ">/run/d2b/no-manager-helper.log 2>&1 &"
    )
    machine.wait_for_file("/run/d2b/helper-no-manager-response.json", timeout=60)
    machine.succeed(
        "jq -e '.type == \"rejected\" and "
        ".payload.code == \"user-manager-unavailable\"' "
        "/run/d2b/helper-no-manager-response.json"
    )

    units = machine.succeed(
        "systemctl list-unit-files --no-pager --no-legend "
        "| awk '{print $1}' | grep -E '^(d2b|microvm)' | sort"
    ).strip().split()
    assert "d2b-unsafe-local-helper.service" not in units, (
        "unsafe-local helper must be a user unit, not a fourth root unit"
    )
  '';
}
