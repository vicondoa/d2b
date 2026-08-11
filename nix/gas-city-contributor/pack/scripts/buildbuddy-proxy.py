#!/usr/bin/env python3
"""Run the credential-bearing Envoy BuildBuddy boundary."""

from __future__ import annotations

import argparse
import array
import ipaddress
import json
import os
import pathlib
import re
import selectors
import socket
import stat
import subprocess
import sys
import tempfile
import threading


UPSTREAM_HOST = "remote.buildbuddy.io"
UPSTREAM_PORT = 443
KEY_PLACEHOLDER = "__BUILDBUDDY_API_KEY_JSON__"
CA_PLACEHOLDER = "__CA_BUNDLE_JSON__"
UPSTREAM_IP_PLACEHOLDER = "__BUILDBUDDY_UPSTREAM_IP_JSON__"
EGRESS_PIPE_PLACEHOLDER = "__BUILDBUDDY_EGRESS_PIPE_JSON__"
LISTEN_PATTERN = re.compile(r"^127\.0\.0\.1:[0-9]{1,5}$")
DENIED_NETWORKS = (
    ipaddress.ip_network("0.0.0.0/8"),
    ipaddress.ip_network("10.0.0.0/8"),
    ipaddress.ip_network("127.0.0.0/8"),
    ipaddress.ip_network("169.254.0.0/16"),
    ipaddress.ip_network("172.16.0.0/12"),
    ipaddress.ip_network("192.168.0.0/16"),
    ipaddress.ip_network("::/128"),
    ipaddress.ip_network("::1/128"),
    ipaddress.ip_network("fc00::/7"),
    ipaddress.ip_network("fe80::/10"),
    ipaddress.ip_network("ff00::/8"),
)


class BuildBuddyProxyError(RuntimeError):
    """Raised when the BuildBuddy credential boundary is unsafe."""


def _absolute(value: str, label: str) -> pathlib.Path:
    path = pathlib.Path(value)
    if (
        not path.is_absolute()
        or any(part == ".." for part in path.parts)
        or os.path.normpath(value) != value
    ):
        raise BuildBuddyProxyError(f"{label} must be an absolute normalized path")
    return path


def validate_upstream(value: str) -> tuple[str, int]:
    if value != f"{UPSTREAM_HOST}:{UPSTREAM_PORT}":
        raise BuildBuddyProxyError(
            f"BuildBuddy upstream is fixed to {UPSTREAM_HOST}:{UPSTREAM_PORT}"
        )
    return UPSTREAM_HOST, UPSTREAM_PORT


def resolve_upstream() -> str:
    try:
        results = socket.getaddrinfo(
            UPSTREAM_HOST,
            UPSTREAM_PORT,
            type=socket.SOCK_STREAM,
        )
    except OSError as error:
        raise BuildBuddyProxyError("BuildBuddy upstream DNS resolution failed") from error
    for _family, _kind, _protocol, _canonname, sockaddr in results:
        address = ipaddress.ip_address(str(sockaddr[0]))
        if address.is_global and not any(address in network for network in DENIED_NETWORKS):
            return str(address)
    raise BuildBuddyProxyError("BuildBuddy upstream did not resolve to a public address")


def _read_api_key(path_value: str) -> str:
    path = _absolute(path_value, "BuildBuddy credential")
    try:
        info = os.lstat(path)
    except OSError as error:
        raise BuildBuddyProxyError("BuildBuddy credential is unavailable") from error
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise BuildBuddyProxyError("BuildBuddy credential must be a regular file")
    if info.st_uid not in {0, os.geteuid()} or info.st_mode & 0o022:
        raise BuildBuddyProxyError("BuildBuddy credential ownership or mode is unsafe")
    try:
        value = path.read_text(encoding="utf-8").strip()
    except (OSError, UnicodeError) as error:
        raise BuildBuddyProxyError("BuildBuddy credential is unreadable") from error
    if (
        not value
        or len(value.encode("utf-8")) > 4096
        or "\x00" in value
        or "\r" in value
        or "\n" in value
    ):
        raise BuildBuddyProxyError("BuildBuddy credential is malformed")
    return value


def render_config(
    template_path: str,
    api_key: str,
    ca_bundle: str = "/etc/ssl/certs/ca-bundle.crt",
    upstream_ip: str = UPSTREAM_HOST,
    egress_pipe: str = "/run/gascity-contributor/buildbuddy-upstream.sock",
) -> str:
    template = _absolute(template_path, "Envoy template")
    if not api_key or "\x00" in api_key or "\r" in api_key or "\n" in api_key:
        raise BuildBuddyProxyError("BuildBuddy API key is malformed")
    try:
        source = template.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        raise BuildBuddyProxyError("Envoy template is unreadable") from error
    if (
        source.count(KEY_PLACEHOLDER) != 1
        or source.count(CA_PLACEHOLDER) != 1
        or source.count(EGRESS_PIPE_PLACEHOLDER) != 1
    ):
        raise BuildBuddyProxyError("Envoy template has an invalid key placeholder")
    rendered = source.replace(KEY_PLACEHOLDER, json.dumps(api_key)).replace(
        CA_PLACEHOLDER, json.dumps(ca_bundle)
    ).replace(EGRESS_PIPE_PLACEHOLDER, json.dumps(egress_pipe))
    if UPSTREAM_IP_PLACEHOLDER in rendered:
        rendered = rendered.replace(UPSTREAM_IP_PLACEHOLDER, json.dumps(upstream_ip))
    if api_key not in rendered:
        raise BuildBuddyProxyError("Envoy key injection failed")
    return rendered


def _receive_egress_fd(channel: socket.socket, request_id: str) -> socket.socket:
    payload = bytearray()
    descriptors: list[int] = []
    item_size = array.array("i").itemsize
    while b"\n" not in payload:
        chunk, ancillary, flags, _address = channel.recvmsg(
            8192,
            socket.CMSG_SPACE(item_size),
        )
        if flags & getattr(socket, "MSG_CTRUNC", 0):
            raise BuildBuddyProxyError("egress response ancillary data was truncated")
        for level, kind, raw in ancillary:
            if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS:
                raise BuildBuddyProxyError("egress response has unauthorized ancillary data")
            values = array.array("i")
            values.frombytes(raw[: len(raw) - (len(raw) % item_size)])
            descriptors.extend(values)
        if not chunk:
            raise BuildBuddyProxyError("egress response closed")
        payload.extend(chunk)
        if len(payload) > 8192:
            raise BuildBuddyProxyError("egress response is too large")
    response = json.loads(bytes(payload).split(b"\n", 1)[0])
    if (
        not isinstance(response, dict)
        or response.get("version") != "fdproxy/1"
        or response.get("request_id") != request_id
        or response.get("ok") is not True
        or len(descriptors) != 1
    ):
        for descriptor in descriptors:
            os.close(descriptor)
        raise BuildBuddyProxyError("egress response is unauthorized or malformed")
    descriptor = descriptors[0]
    os.set_inheritable(descriptor, False)
    return socket.socket(fileno=descriptor)


def _open_egress_socket(socket_path: str, auth_token: str, request_id: str) -> socket.socket:
    channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        channel.connect(socket_path)
        channel.sendall(
            json.dumps(
                {
                    "version": "fdproxy/1",
                    "operation": "connect",
                    "request_id": request_id,
                    "auth": auth_token,
                    "host": UPSTREAM_HOST,
                    "port": UPSTREAM_PORT,
                },
                separators=(",", ":"),
            ).encode()
            + b"\n"
        )
        return _receive_egress_fd(channel, request_id)
    finally:
        channel.close()


def _relay(first: socket.socket, second: socket.socket) -> None:
    selector = selectors.DefaultSelector()
    selector.register(first, selectors.EVENT_READ, second)
    selector.register(second, selectors.EVENT_READ, first)
    try:
        while selector.get_map():
            for key, _mask in selector.select():
                data = key.fileobj.recv(64 * 1024)
                if not data:
                    return
                key.data.sendall(data)
    finally:
        selector.close()


def serve_egress_pipe(
    *,
    pipe_path: str,
    egress_socket: str,
    auth_token: str,
    stop: threading.Event,
) -> None:
    path = _absolute(pipe_path, "BuildBuddy egress pipe")
    if os.path.lexists(path):
        if not stat.S_ISSOCK(os.lstat(path).st_mode):
            raise BuildBuddyProxyError("BuildBuddy egress pipe path is occupied")
        path.unlink()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(path))
    os.chmod(path, 0o660)
    listener.listen(16)
    listener.settimeout(0.2)
    counter = 0

    def serve_one(client: socket.socket, request_id: str) -> None:
        upstream: socket.socket | None = None
        try:
            upstream = _open_egress_socket(egress_socket, auth_token, request_id)
            _relay(client, upstream)
        except (BuildBuddyProxyError, OSError, json.JSONDecodeError):
            pass
        finally:
            if upstream is not None:
                upstream.close()
            client.close()

    try:
        while not stop.is_set():
            try:
                client, _address = listener.accept()
            except socket.timeout:
                continue
            counter += 1
            threading.Thread(
                target=serve_one,
                args=(client, str(counter)),
                daemon=True,
            ).start()
    finally:
        listener.close()
        path.unlink(missing_ok=True)


def _listen(value: str) -> tuple[str, int]:
    if LISTEN_PATTERN.fullmatch(value) is None:
        raise BuildBuddyProxyError("BuildBuddy listener must be loopback-only")
    port = int(value.rsplit(":", 1)[1])
    if not 1024 <= port <= 65535:
        raise BuildBuddyProxyError("BuildBuddy listener port is outside the unprivileged range")
    return "127.0.0.1", port


def serve(args: argparse.Namespace) -> int:
    validate_upstream(args.upstream)
    _listen(args.listen)
    envoy = _absolute(args.envoy, "Envoy executable")
    if not envoy.is_file() or not os.access(envoy, os.X_OK):
        raise BuildBuddyProxyError("Envoy executable is unavailable")
    ca = _absolute(args.ca, "CA bundle")
    if not ca.is_file():
        raise BuildBuddyProxyError("CA bundle is unavailable")
    key = _read_api_key(args.credential)
    runtime_dir = _absolute(args.runtime_dir, "Envoy runtime directory")
    runtime_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    if os.stat(runtime_dir).st_uid != os.geteuid() or os.stat(runtime_dir).st_mode & 0o077:
        raise BuildBuddyProxyError("Envoy runtime directory is not private")
    pipe_path = runtime_dir / "buildbuddy-upstream.sock"
    rendered = render_config(
        args.template,
        key,
        str(ca),
        egress_pipe=str(pipe_path),
    )
    del key
    stop_pipe = threading.Event()
    pipe_thread = threading.Thread(
        target=serve_egress_pipe,
        kwargs={
            "pipe_path": str(pipe_path),
            "egress_socket": args.egress_socket,
            "auth_token": os.environ.get(args.auth_token_env, ""),
            "stop": stop_pipe,
        },
        daemon=True,
    )
    pipe_thread.start()
    descriptor, config_path = tempfile.mkstemp(
        prefix="envoy-",
        suffix=".yaml",
        dir=runtime_dir,
        text=True,
    )
    os.fchmod(descriptor, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(rendered)
            stream.flush()
            os.fsync(stream.fileno())
        environment = {
            "PATH": os.environ.get("PATH", "/run/current-system/sw/bin"),
            "SSL_CERT_FILE": str(ca),
            "LANG": "C",
        }
        process = subprocess.Popen(
            [str(envoy), "--config-path", config_path],
            close_fds=True,
            env=environment,
        )
        return process.wait()
    finally:
        stop_pipe.set()
        pipe_thread.join(timeout=1)
        pathlib.Path(config_path).unlink(missing_ok=True)
        rendered = ""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    serve_parser = subparsers.add_parser("serve")
    serve_parser.add_argument("--template", required=True)
    serve_parser.add_argument("--credential", required=True)
    serve_parser.add_argument("--envoy", required=True)
    serve_parser.add_argument("--listen", default="127.0.0.1:19801")
    serve_parser.add_argument("--ca", required=True)
    serve_parser.add_argument("--runtime-dir", default="/run/gascity-contributor/buildbuddy")
    serve_parser.add_argument("--upstream", default=f"{UPSTREAM_HOST}:{UPSTREAM_PORT}")
    serve_parser.add_argument("--egress-socket", required=True)
    serve_parser.add_argument("--auth-token-env", default="GC_FDPROXY_AUTH")
    args = parser.parse_args(argv)
    if args.command == "serve":
        return serve(args)
    raise BuildBuddyProxyError("unknown BuildBuddy proxy command")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (BuildBuddyProxyError, OSError, subprocess.SubprocessError) as error:
        print(f"BuildBuddy proxy rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
