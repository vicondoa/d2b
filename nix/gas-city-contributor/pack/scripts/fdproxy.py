#!/usr/bin/env python3
"""Expose loopback HTTP CONNECT through an authenticated fd-passing channel.

The process never opens an outbound socket.  The inherited Unix channel is
owned by the service-side allowlisting peer.  One authenticated request is
sent for each CONNECT client and the peer returns exactly one connected
upstream socket with SCM_RIGHTS.  The returned socket is private to that
client, so requests can be served concurrently without sharing a raw tunnel.
"""

from __future__ import annotations

import argparse
import array
import ctypes
import errno
import json
import os
import selectors
import socket
import subprocess
import sys
import threading
from collections.abc import Callable, Sequence


PROTOCOL = "fdproxy/1"
AUTH_ENVIRONMENT = "GC_FDPROXY_AUTH"
MAX_HEADER_BYTES = 32 * 1024
MAX_CONTROL_LINE_BYTES = 8 * 1024
MAX_AUTH_BYTES = 512
MAX_PASSED_FDS = 1


class FDProxyError(RuntimeError):
    """Raised for a malformed local proxy request or sidecar response."""


def set_nondumpable() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(4, 0, 0, 0, 0) != 0:
        error_number = ctypes.get_errno()
        raise OSError(error_number, os.strerror(error_number))


def _auth_token(value: str | None = None) -> str:
    token = value if value is not None else os.environ.get(AUTH_ENVIRONMENT)
    if (
        not token
        or len(token.encode("utf-8")) > MAX_AUTH_BYTES
        or any(ord(character) < 0x21 or ord(character) > 0x7E for character in token)
    ):
        raise FDProxyError("fdproxy authentication token is not configured")
    return token


def _close_descriptors(descriptors: Sequence[int]) -> None:
    for descriptor in descriptors:
        try:
            os.close(descriptor)
        except OSError:
            pass


def _extract_rights(
    ancillary: Sequence[tuple[int, int, bytes]],
) -> list[int]:
    descriptors: list[int] = []
    item_size = array.array("i").itemsize
    for level, kind, data in ancillary:
        if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS:
            _close_descriptors(descriptors)
            raise FDProxyError("allowlisting response has unauthorized ancillary data")
        if len(data) % item_size:
            complete = len(data) - (len(data) % item_size)
            if complete:
                values = array.array("i")
                values.frombytes(data[:complete])
                _close_descriptors(values)
            _close_descriptors(descriptors)
            raise FDProxyError("allowlisting response SCM_RIGHTS data is malformed")
        values = array.array("i")
        values.frombytes(data)
        descriptors.extend(int(value) for value in values)
    for descriptor in descriptors:
        try:
            flags = fcntl_getfd(descriptor)
            fcntl_setfd(descriptor, flags | fcntl_cloexec())
        except OSError:
            _close_descriptors(descriptors)
            raise FDProxyError("allowlisting response passed an invalid fd")
    return descriptors


def fcntl_getfd(descriptor: int) -> int:
    import fcntl

    return fcntl.fcntl(descriptor, fcntl.F_GETFD)


def fcntl_setfd(descriptor: int, flags: int) -> None:
    import fcntl

    fcntl.fcntl(descriptor, fcntl.F_SETFD, flags)


def fcntl_cloexec() -> int:
    import fcntl

    return fcntl.FD_CLOEXEC


def _read_until_headers(client: socket.socket) -> bytes:
    data = bytearray()
    while b"\r\n\r\n" not in data:
        chunk = client.recv(4096)
        if not chunk:
            raise FDProxyError("client closed before CONNECT headers")
        data.extend(chunk)
        if len(data) > MAX_HEADER_BYTES:
            raise FDProxyError("proxy request headers exceed limit")
    return bytes(data)


def _parse_authority(request: bytes) -> tuple[str, int]:
    header, _separator, _remainder = request.partition(b"\r\n\r\n")
    first_line = header.split(b"\r\n", 1)[0]
    try:
        method, authority, version = first_line.decode("ascii").split(" ", 2)
    except ValueError as error:
        raise FDProxyError("proxy request line is malformed") from error
    if method != "CONNECT" or version != "HTTP/1.1":
        raise FDProxyError("only HTTP/1.1 CONNECT is permitted")
    if not authority or any(character.isspace() for character in authority):
        raise FDProxyError("CONNECT authority is malformed")
    if authority.startswith("["):
        closing = authority.find("]")
        if closing < 0 or closing + 2 > len(authority) or authority[closing + 1] != ":":
            raise FDProxyError("IPv6 CONNECT authority is malformed")
        host = authority[1:closing]
        port_text = authority[closing + 2 :]
    else:
        if ":" not in authority:
            raise FDProxyError("CONNECT authority has no port")
        host, port_text = authority.rsplit(":", 1)
        if ":" in host:
            raise FDProxyError("IPv6 CONNECT authority must be bracketed")
    if not host or len(host) > 253 or any(
        ord(character) < 0x21 or ord(character) > 0x7E or character in "/\\"
        for character in host
    ):
        raise FDProxyError("CONNECT host is malformed")
    try:
        port = int(port_text, 10)
    except ValueError as error:
        raise FDProxyError("CONNECT port is malformed") from error
    if not 1 <= port <= 65535:
        raise FDProxyError("CONNECT port is outside the TCP range")
    return host, port


def _receive_response(
    channel: socket.socket,
) -> tuple[dict[str, object], list[int]]:
    data = bytearray()
    descriptors: list[int] = []
    cmsg_space = socket.CMSG_SPACE(array.array("i").itemsize * MAX_PASSED_FDS)
    try:
        while b"\n" not in data:
            if len(data) >= MAX_CONTROL_LINE_BYTES:
                raise FDProxyError("allowlisting response exceeds limit")
            chunk, ancillary, flags, _address = channel.recvmsg(
                min(4096, MAX_CONTROL_LINE_BYTES - len(data)),
                cmsg_space,
            )
            descriptors.extend(_extract_rights(ancillary))
            if flags & getattr(socket, "MSG_CTRUNC", 0):
                raise FDProxyError("allowlisting response ancillary data was truncated")
            if not chunk:
                raise FDProxyError("allowlisting proxy closed its channel")
            data.extend(chunk)
        line, remainder = bytes(data).split(b"\n", 1)
        if remainder:
            raise FDProxyError("allowlisting response pipelined multiple messages")
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise FDProxyError("allowlisting response is not JSON") from error
        if not isinstance(value, dict):
            raise FDProxyError("allowlisting response is not an object")
        return value, descriptors
    except Exception:
        _close_descriptors(descriptors)
        raise


def _request_upstream(
    channel: socket.socket,
    host: str,
    port: int,
    *,
    auth_token: str | None = None,
    request_id: str = "1",
) -> socket.socket | None:
    token = _auth_token(auth_token)
    request = {
        "version": PROTOCOL,
        "operation": "connect",
        "request_id": request_id,
        "auth": token,
        "host": host,
        "port": port,
    }
    channel.sendall(json.dumps(request, separators=(",", ":")).encode("ascii") + b"\n")
    response, descriptors = _receive_response(channel)
    try:
        if response.get("version") != PROTOCOL:
            raise FDProxyError("allowlisting proxy protocol version mismatch")
        if response.get("request_id") != request_id:
            raise FDProxyError("allowlisting proxy request id mismatch")
        if type(response.get("ok")) is not bool:
            raise FDProxyError("allowlisting proxy response has no boolean decision")
        if response["ok"]:
            if len(descriptors) != 1:
                raise FDProxyError(
                    "allowlisting proxy must pass exactly one upstream socket"
                )
            descriptor = descriptors.pop()
            try:
                return socket.socket(fileno=descriptor)
            except OSError:
                os.close(descriptor)
                raise
        if descriptors:
            raise FDProxyError("denied proxy response passed an unexpected fd")
        return None
    finally:
        _close_descriptors(descriptors)


class MultiplexedChannel:
    """Serialize request/SCM_RIGHTS response pairs on one inherited channel."""

    def __init__(self, channel_fd: int, *, auth_token: str | None = None):
        if channel_fd < 3:
            raise FDProxyError("proxy channel fd must not overlap stdio")
        self.channel = socket.socket(fileno=os.dup(channel_fd))
        self.channel.setblocking(True)
        self.auth_token = _auth_token(auth_token)
        self.lock = threading.Lock()
        self._next_request = 0

    def request(self, host: str, port: int) -> socket.socket | None:
        with self.lock:
            self._next_request += 1
            request_id = str(self._next_request)
            return _request_upstream(
                self.channel,
                host,
                port,
                auth_token=self.auth_token,
                request_id=request_id,
            )

    def close(self) -> None:
        self.channel.close()


def _relay(client: socket.socket, upstream: socket.socket) -> None:
    selector = selectors.DefaultSelector()
    selector.register(client, selectors.EVENT_READ, client)
    selector.register(upstream, selectors.EVENT_READ, upstream)
    try:
        while selector.get_map():
            events = selector.select()
            for key, _mask in events:
                source = key.fileobj
                try:
                    data = source.recv(64 * 1024)
                except BlockingIOError:
                    continue
                if not data:
                    return
                destination = upstream if source is client else client
                try:
                    destination.sendall(data)
                except (BrokenPipeError, ConnectionResetError):
                    return
    finally:
        selector.close()


def _send_http_error(client: socket.socket, status: bytes) -> None:
    try:
        client.sendall(
            b"HTTP/1.1 "
            + status
            + b"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
        )
    except (BrokenPipeError, ConnectionResetError):
        pass


def handle_client(
    client: socket.socket,
    channel: MultiplexedChannel,
) -> None:
    request = _read_until_headers(client)
    host, port = _parse_authority(request)
    upstream = channel.request(host, port)
    if upstream is None:
        _send_http_error(client, b"403 Forbidden")
        return
    try:
        client.sendall(
            b"HTTP/1.1 200 Connection Established\r\n"
            b"Proxy-Agent: gascity-fdproxy/1\r\n\r\n"
        )
        _relay(client, upstream)
    finally:
        upstream.close()


def _listen(host: str, port: int) -> socket.socket:
    if host not in {"127.0.0.1", "localhost"}:
        raise FDProxyError("fdproxy listener must remain loopback-only")
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    try:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
        listener.bind((host, port))
        listener.listen(64)
        return listener
    except OSError:
        listener.close()
        raise


def serve(
    channel_fd: int,
    *,
    host: str = "127.0.0.1",
    port: int = 3128,
    once: bool = False,
    auth_token: str | None = None,
    on_bound: Callable[[int], None] | None = None,
) -> None:
    listener = _listen(host, port)
    channel = MultiplexedChannel(channel_fd, auth_token=auth_token)
    try:
        _serve_listener(
            listener,
            channel,
            once=once,
            on_bound=on_bound,
        )
    finally:
        listener.close()
        channel.close()


def _serve_listener(
    listener: socket.socket,
    channel: MultiplexedChannel,
    *,
    once: bool = False,
    on_bound: Callable[[int], None] | None = None,
    stop_event: threading.Event | None = None,
) -> None:
    if on_bound is not None:
        on_bound(listener.getsockname()[1])
    if stop_event is not None:
        listener.settimeout(0.2)
    threads: list[threading.Thread] = []

    def serve_one(client: socket.socket) -> None:
        try:
            try:
                handle_client(client, channel)
            except (ConnectionError, FDProxyError, OSError) as error:
                _send_http_error(client, b"400 Bad Request")
                print(f"fdproxy request rejected: {error}", file=sys.stderr)
        finally:
            client.close()

    while stop_event is None or not stop_event.is_set():
        try:
            client, _address = listener.accept()
        except socket.timeout:
            continue
        thread = threading.Thread(
            target=serve_one,
            args=(client,),
            name="fdproxy-connect",
            daemon=True,
        )
        threads.append(thread)
        thread.start()
        if once:
            break
    for thread in threads:
        thread.join(timeout=1.0)


def run_with_command(
    channel_fd: int,
    command: list[str],
    *,
    host: str = "127.0.0.1",
    port: int = 3128,
    progress_fd: int | None = None,
    auth_token: str | None = None,
) -> int:
    """Run the Copilot child beside the local multiplexed proxy."""

    if not command:
        raise FDProxyError("proxy command must not be empty")
    if progress_fd is not None and progress_fd < 3:
        raise FDProxyError("progress fd must not overlap stdio")
    if progress_fd == channel_fd:
        raise FDProxyError("proxy and progress fds must be distinct")
    token = _auth_token(auth_token)
    listener = _listen(host, port)
    stop_event = threading.Event()
    channel = MultiplexedChannel(channel_fd, auth_token=token)

    def serve_proxy() -> None:
        try:
            _serve_listener(listener, channel, stop_event=stop_event)
        except (OSError, FDProxyError) as error:
            if not stop_event.is_set():
                print(f"fdproxy stopped: {error}", file=sys.stderr)

    proxy_thread = threading.Thread(target=serve_proxy, name="fdproxy", daemon=True)
    proxy_thread.start()
    try:
        child_environment = dict(os.environ)
        child_environment.pop(AUTH_ENVIRONMENT, None)
        child = subprocess.Popen(
            command,
            close_fds=True,
            pass_fds=(progress_fd,) if progress_fd is not None else (),
            env=child_environment,
        )
        if progress_fd is not None:
            os.close(progress_fd)
        return child.wait()
    finally:
        stop_event.set()
        listener.close()
        proxy_thread.join(timeout=1.0)
        channel.close()


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--channel-fd", type=int, required=True)
    parser.add_argument("--listen", default="127.0.0.1:3128")
    parser.add_argument("--once", action="store_true")
    parser.add_argument("--print-contract", action="store_true")
    parser.add_argument("--progress-fd", type=int)
    parser.add_argument("--auth-token")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    set_nondumpable()
    if args.print_contract:
        json.dump(
            {
                "protocol": PROTOCOL,
                "channel": "authenticated-multiplexed-scm-rights",
                "egress": "sidecar-authorized",
                "fd_per_connect": True,
                "outbound_sockets": False,
                "listen": "loopback-only",
            },
            sys.stdout,
        )
        sys.stdout.write("\n")
        return 0
    if ":" not in args.listen:
        raise SystemExit("--listen must be HOST:PORT")
    host, port_text = args.listen.rsplit(":", 1)
    try:
        port = int(port_text, 10)
    except ValueError as error:
        raise SystemExit("--listen port is malformed") from error
    command = list(args.command)
    if command[:1] == ["--"]:
        command = command[1:]
    if command:
        return run_with_command(
            args.channel_fd,
            command,
            host=host,
            port=port,
            progress_fd=args.progress_fd,
            auth_token=args.auth_token,
        )
    serve(
        args.channel_fd,
        host=host,
        port=port,
        once=args.once,
        auth_token=args.auth_token,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
