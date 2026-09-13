# Type-G runNixOSTest: live Wayland proxy AF_UNIX relay.
#
# Boots a minimal NixOS node and runs d2b-wayland-proxy against a fake Wayland
# compositor socket. This covers the live client-to-upstream relay path and
# socket posture that unit tests cannot exercise; rendered d2b DAG wiring remains
# covered by the graphics smoke/eval cases.
{ pkgs, self }:

let
  # Shared fixture diagnostics (issue #513): row dumps and per-stage markers.
  d2bLib = import ./lib.nix {
    inherit self;
    inherit (pkgs) lib;
  };
  proxyPackage = self.packages.${pkgs.stdenv.hostPlatform.system}.d2b-wayland-proxy;
in
pkgs.testers.runNixOSTest {
  name = "d2b-wayland-proxy";

  nodes.machine = {
    users.users.alice = {
      isNormalUser = true;
      uid = 1000;
    };

    environment.systemPackages = [
      pkgs.python3
      proxyPackage
    ];

    system.stateVersion = "25.11";
  };

  testScript = ''
    ${d2bLib.fixtureDiagnostics}

    start_all()
    stage("boot")
    machine.wait_for_unit("multi-user.target", timeout=180)

    stage("fake-upstream")
    machine.succeed("install -d -m 0700 -o alice -g users /run/d2b-wayland-proxy-test")
    machine.succeed(
        "cat > /run/d2b-wayland-proxy-test/fake-upstream.py <<'PY'\n"
        "import os, select, socket, time\n"
        "path = '/run/d2b-wayland-proxy-test/upstream.sock'\n"
        "ready = '/run/d2b-wayland-proxy-test/upstream.ready'\n"
        "seen = '/run/d2b-wayland-proxy-test/upstream.seen'\n"
        "for p in (path, ready, seen):\n"
        "    try:\n"
        "        os.unlink(p)\n"
        "    except FileNotFoundError:\n"
        "        pass\n"
        "srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n"
        "srv.bind(path)\n"
        "os.chmod(path, 0o600)\n"
        "srv.setblocking(False)\n"
        "srv.listen(8)\n"
        "open(ready, 'w').write('ready')\n"
        "connections = []\n"
        "deadline = time.monotonic() + 60\n"
        "while time.monotonic() < deadline:\n"
        "    readable = [srv] + connections\n"
        "    ready, _, _ = select.select(readable, [], [], 0.2)\n"
        "    for sock in ready:\n"
        "        if sock is srv:\n"
        "            conn, _ = srv.accept()\n"
        "            conn.setblocking(False)\n"
        "            connections.append(conn)\n"
        "        else:\n"
        "            data = sock.recv(12)\n"
        "            if data:\n"
        "                open(seen, 'wb').write(data)\n"
        "                deadline = 0\n"
        "                break\n"
        "            connections.remove(sock)\n"
        "            sock.close()\n"
        "for conn in connections:\n"
        "    conn.close()\n"
        "srv.close()\n"
        "PY\n"
        "chown alice:users /run/d2b-wayland-proxy-test/fake-upstream.py"
    )
    machine.succeed(
        "runuser -u alice -- python3 /run/d2b-wayland-proxy-test/fake-upstream.py "
        ">/run/d2b-wayland-proxy-test/upstream.log 2>&1 & "
        "echo $! > /run/d2b-wayland-proxy-test/upstream.pid"
    )
    diag_step(
        "upstream-ready",
        lambda: machine.wait_for_file(
            "/run/d2b-wayland-proxy-test/upstream.ready", timeout=30
        ),
        rows=[
            (
                "upstream log",
                "cat /run/d2b-wayland-proxy-test/upstream.log "
                "2>/dev/null || true",
            ),
            (
                "test dir",
                "ls -la /run/d2b-wayland-proxy-test 2>&1 || true",
            ),
        ],
    )

    stage("proxy-start")
    machine.succeed(
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/d2b-wayland-proxy-test "
        "d2b-wayland-proxy "
        "--listen /run/d2b-wayland-proxy-test/proxy.sock "
        "--connect /run/d2b-wayland-proxy-test/upstream.sock "
        "--target acceptance-guest.local.d2b "
        "--provider-kind local-vm "
        ">/run/d2b-wayland-proxy-test/proxy.log 2>&1 & "
        "echo $! > /run/d2b-wayland-proxy-test/proxy.pid"
    )
    machine.succeed(
        "for attempt in $(seq 1 300); do "
        "test -S /run/d2b-wayland-proxy-test/proxy.sock && exit 0; "
        "kill -0 $(cat /run/d2b-wayland-proxy-test/proxy.pid) 2>/dev/null || "
        "{ echo 'd2b-wayland-proxy exited before binding its socket:'; "
        "cat /run/d2b-wayland-proxy-test/proxy.log; exit 1; }; "
        "sleep 0.1; done; "
        "echo 'd2b-wayland-proxy did not bind its socket within 30s:'; "
        "cat /run/d2b-wayland-proxy-test/proxy.log; exit 1"
    )
    machine.succeed("test -S /run/d2b-wayland-proxy-test/proxy.sock")
    machine.succeed("test \"$(stat -c %a /run/d2b-wayland-proxy-test)\" = 700")

    # Send a minimal wl_display.get_registry request through the proxy. The fake
    # compositor must observe the same 12-byte Wayland request on its upstream
    # socket, proving the live proxy accepted a client and relayed protocol
    # traffic rather than only binding a socket.
    stage("relay-proof")
    machine.succeed(
        "python3 - <<'PY'\n"
        "import socket, struct\n"
        "import time\n"
        "sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n"
        "sock.connect('/run/d2b-wayland-proxy-test/proxy.sock')\n"
        "sock.sendall(struct.pack('<III', 1, (12 << 16) | 1, 2))\n"
        "time.sleep(1)\n"
        "sock.close()\n"
        "PY"
    )
    diag_step(
        "relay-observed",
        lambda: machine.wait_for_file(
            "/run/d2b-wayland-proxy-test/upstream.seen", timeout=30
        ),
        rows=[
            (
                "upstream log",
                "cat /run/d2b-wayland-proxy-test/upstream.log "
                "2>/dev/null || true",
            ),
            (
                "proxy log",
                "cat /run/d2b-wayland-proxy-test/proxy.log "
                "2>/dev/null || true",
            ),
        ],
    )
    machine.succeed(
        "python3 - <<'PY'\n"
        "import pathlib, struct\n"
        "data = pathlib.Path('/run/d2b-wayland-proxy-test/upstream.seen').read_bytes()\n"
        "assert data == struct.pack('<III', 1, (12 << 16) | 1, 2), data\n"
        "PY"
    )

    stage("teardown")
    machine.succeed("kill $(cat /run/d2b-wayland-proxy-test/proxy.pid) || true")
    machine.succeed("kill $(cat /run/d2b-wayland-proxy-test/upstream.pid) || true")
  '';
}
