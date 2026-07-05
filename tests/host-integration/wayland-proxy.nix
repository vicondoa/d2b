# Type-G runNixOSTest: live Wayland proxy AF_UNIX relay.
#
# Boots a minimal NixOS node and runs d2b-wayland-proxy against a fake Wayland
# compositor socket. This covers the live client-to-upstream relay path and
# socket posture that unit tests cannot exercise; rendered d2b DAG wiring remains
# covered by the graphics smoke/eval cases.
{ pkgs, self }:

let
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
    start_all()
    machine.wait_for_unit("multi-user.target")

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
    machine.wait_for_file("/run/d2b-wayland-proxy-test/upstream.ready")

    machine.succeed(
        "runuser -u alice -- env XDG_RUNTIME_DIR=/run/d2b-wayland-proxy-test "
        "d2b-wayland-proxy "
        "--listen /run/d2b-wayland-proxy-test/proxy.sock "
        "--connect /run/d2b-wayland-proxy-test/upstream.sock "
        "--vm-name corp-vm "
        ">/run/d2b-wayland-proxy-test/proxy.log 2>&1 & "
        "echo $! > /run/d2b-wayland-proxy-test/proxy.pid"
    )
    machine.wait_for_file("/run/d2b-wayland-proxy-test/proxy.sock")
    machine.succeed("test -S /run/d2b-wayland-proxy-test/proxy.sock")
    machine.succeed("test \"$(stat -c %a /run/d2b-wayland-proxy-test)\" = 700")

    # Send a minimal wl_display.get_registry request through the proxy. The fake
    # compositor must observe the same 12-byte Wayland request on its upstream
    # socket, proving the live proxy accepted a client and relayed protocol
    # traffic rather than only binding a socket.
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
    machine.wait_for_file("/run/d2b-wayland-proxy-test/upstream.seen")
    machine.succeed(
        "python3 - <<'PY'\n"
        "import pathlib, struct\n"
        "data = pathlib.Path('/run/d2b-wayland-proxy-test/upstream.seen').read_bytes()\n"
        "assert data == struct.pack('<III', 1, (12 << 16) | 1, 2), data\n"
        "PY"
    )

    machine.succeed("kill $(cat /run/d2b-wayland-proxy-test/proxy.pid) || true")
    machine.succeed("kill $(cat /run/d2b-wayland-proxy-test/upstream.pid) || true")
  '';
}
