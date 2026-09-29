//! The live Wayland proxy AF_UNIX relay check, ported from its fixture.
//!
//! It boots a minimal NixOS node and runs `d2b-wayland-proxy` against a fake
//! Wayland compositor socket, covering the live client-to-upstream relay path
//! and the socket posture that unit tests cannot exercise (rendered d2b DAG
//! wiring stays covered by the graphics smoke and eval cases). The assertions
//! are the fixture's, in the fixture's order and with the fixture's own
//! command text and bounds: the fake upstream comes up, the proxy binds a
//! socket in a directory only its user can enter, a minimal
//! `wl_display.get_registry` request survives the relay byte for byte, and
//! both processes are torn down.
//!
//! The fixture declared a plain NixOS node - it never wanted the d2b daemon
//! host - so the guest it boots, with the `alice` user the proxy runs as and
//! the two packages it runs from, is declared in
//! `nix/test-support/host-integration-node.nix` beside the reusable nodes.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use std::time::Duration;

use crate::legacy::{DiagRow, GuestControl, LegacyResult};

/// The bound the guest's own `multi-user.target` gets, the fixture's own.
const BOOT: Duration = Duration::from_secs(180);

/// The bound each of the two file waits gets, the fixture's own.
const WAIT: Duration = Duration::from_secs(30);

/// The directory the fixture keeps both sockets, both logs and both pid
/// files in.
const TEST_DIR: &str = "/run/d2b-wayland-proxy-test";

/// The file the fake compositor writes once it is listening.
const UPSTREAM_READY: &str = "/run/d2b-wayland-proxy-test/upstream.ready";

/// The file the fake compositor writes with the bytes it received.
const UPSTREAM_SEEN: &str = "/run/d2b-wayland-proxy-test/upstream.seen";

/// The socket the proxy binds.
const PROXY_SOCKET: &str = "/run/d2b-wayland-proxy-test/proxy.sock";

/// The fake compositor, written into the guest by the fixture's own command.
///
/// The literal is flush-left because it is the fixture's command text
/// byte-for-byte: the fixture built it by joining Python string literals with
/// `\n`, so every line here begins where it began there, and the `chown` that
/// follows the heredoc's terminator is the last line of the same command.
const FAKE_UPSTREAM: &str = r#"cat > /run/d2b-wayland-proxy-test/fake-upstream.py <<'PY'
import os, select, socket, time
path = '/run/d2b-wayland-proxy-test/upstream.sock'
ready = '/run/d2b-wayland-proxy-test/upstream.ready'
seen = '/run/d2b-wayland-proxy-test/upstream.seen'
for p in (path, ready, seen):
    try:
        os.unlink(p)
    except FileNotFoundError:
        pass
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(path)
os.chmod(path, 0o600)
srv.setblocking(False)
srv.listen(8)
open(ready, 'w').write('ready')
connections = []
deadline = time.monotonic() + 60
while time.monotonic() < deadline:
    readable = [srv] + connections
    ready, _, _ = select.select(readable, [], [], 0.2)
    for sock in ready:
        if sock is srv:
            conn, _ = srv.accept()
            conn.setblocking(False)
            connections.append(conn)
        else:
            data = sock.recv(12)
            if data:
                open(seen, 'wb').write(data)
                deadline = 0
                break
            connections.remove(sock)
            sock.close()
for conn in connections:
    conn.close()
srv.close()
PY
chown alice:users /run/d2b-wayland-proxy-test/fake-upstream.py"#;

/// Start the fake compositor as the proxy's own user, in the background, and
/// record its pid.
const START_UPSTREAM: &str = concat!(
    "runuser -u alice -- python3 /run/d2b-wayland-proxy-test/fake-upstream.py ",
    ">/run/d2b-wayland-proxy-test/upstream.log 2>&1 & ",
    "echo $! > /run/d2b-wayland-proxy-test/upstream.pid",
);

/// Start the proxy as the proxy's own user, in the background, and record its
/// pid. The target and provider kind are the fixture's own words.
const START_PROXY: &str = concat!(
    "runuser -u alice -- env XDG_RUNTIME_DIR=/run/d2b-wayland-proxy-test ",
    "d2b-wayland-proxy ",
    "--listen /run/d2b-wayland-proxy-test/proxy.sock ",
    "--connect /run/d2b-wayland-proxy-test/upstream.sock ",
    "--target acceptance-guest.local.d2b ",
    "--provider-kind local-vm ",
    ">/run/d2b-wayland-proxy-test/proxy.log 2>&1 & ",
    "echo $! > /run/d2b-wayland-proxy-test/proxy.pid",
);

/// Wait until the proxy has bound its socket, failing with the proxy's own log
/// if the process exited first. The fixture's own loop, with its own bound of
/// 300 attempts at a tenth of a second.
const WAIT_FOR_BIND: &str = concat!(
    "for attempt in $(seq 1 300); do ",
    "test -S /run/d2b-wayland-proxy-test/proxy.sock && exit 0; ",
    "kill -0 $(cat /run/d2b-wayland-proxy-test/proxy.pid) 2>/dev/null || ",
    "{ echo 'd2b-wayland-proxy exited before binding its socket:'; ",
    "cat /run/d2b-wayland-proxy-test/proxy.log; exit 1; }; ",
    "sleep 0.1; done; ",
    "echo 'd2b-wayland-proxy did not bind its socket within 30s:'; ",
    "cat /run/d2b-wayland-proxy-test/proxy.log; exit 1",
);

/// Send a minimal `wl_display.get_registry` request through the proxy: twelve
/// bytes, `object_id=1`, `size=12` in the high half of the second word, and
/// `opcode=2` in the low half, which is the wire form a client's first
/// request has.
const SEND_REQUEST: &str = concat!(
    "python3 - <<'PY'\n",
    "import socket, struct\n",
    "import time\n",
    "sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n",
    "sock.connect('/run/d2b-wayland-proxy-test/proxy.sock')\n",
    "sock.sendall(struct.pack('<III', 1, (12 << 16) | 1, 2))\n",
    "time.sleep(1)\n",
    "sock.close()\n",
    "PY",
);

/// The same twelve bytes, asserted against what the compositor actually
/// received: the relay is proven by the bytes and not by a socket that was
/// bound.
const ASSERT_REQUEST: &str = concat!(
    "python3 - <<'PY'\n",
    "import pathlib, struct\n",
    "data = pathlib.Path('/run/d2b-wayland-proxy-test/upstream.seen').read_bytes()\n",
    "assert data == struct.pack('<III', 1, (12 << 16) | 1, 2), data\n",
    "PY",
);

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");
    control.wait_for_unit("multi-user.target", None, BOOT)?;

    // The fake upstream, written and started as the user that will own both
    // sockets, with the directory only that user can enter.
    control.stage("fake-upstream");
    control.succeed(
        &["install -d -m 0700 -o alice -g users /run/d2b-wayland-proxy-test"],
        None,
    )?;
    control.succeed(&[FAKE_UPSTREAM], None)?;
    control.succeed(&[START_UPSTREAM], None)?;
    let upstream_rows: [DiagRow<'_>; 2] = [
        (
            "upstream log",
            "cat /run/d2b-wayland-proxy-test/upstream.log 2>/dev/null || true",
        ),
        (
            "test dir",
            "ls -la /run/d2b-wayland-proxy-test 2>&1 || true",
        ),
    ];
    control.diag_file("upstream-ready", UPSTREAM_READY, WAIT, &upstream_rows)?;

    // The proxy itself: it binds a socket, and refuses to run in a world
    // where it did not.
    control.stage("proxy-start");
    control.succeed(&[START_PROXY], None)?;
    control.succeed(&[WAIT_FOR_BIND], None)?;
    control.succeed(&[&format!("test -S {PROXY_SOCKET}")], None)?;
    control.succeed(
        &[&format!("test \"$(stat -c %a {TEST_DIR})\" = 700")],
        None,
    )?;

    // The live relay: a client's first request must reach the upstream socket
    // unchanged, so the proxy is relaying protocol traffic rather than only
    // binding a socket.
    control.stage("relay-proof");
    control.succeed(&[SEND_REQUEST], None)?;
    let relay_rows: [DiagRow<'_>; 2] = [
        (
            "upstream log",
            "cat /run/d2b-wayland-proxy-test/upstream.log 2>/dev/null || true",
        ),
        (
            "proxy log",
            "cat /run/d2b-wayland-proxy-test/proxy.log 2>/dev/null || true",
        ),
    ];
    control.diag_file("relay-observed", UPSTREAM_SEEN, WAIT, &relay_rows)?;
    control.succeed(&[ASSERT_REQUEST], None)?;

    // Both processes are the fixture's own, so both are the fixture's to end.
    control.stage("teardown");
    control.succeed(&["kill $(cat /run/d2b-wayland-proxy-test/proxy.pid) || true"], None)?;
    control.succeed(
        &["kill $(cat /run/d2b-wayland-proxy-test/upstream.pid) || true"],
        None,
    )?;

    Ok(())
}
