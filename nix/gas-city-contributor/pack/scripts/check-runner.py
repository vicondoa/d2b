#!/usr/bin/env python3
"""Run approved checks against a read-only worktree and a local Nix store."""

from __future__ import annotations

import argparse
import array
import json
import os
import pathlib
import re
import selectors
import signal
import socket
import stat
import subprocess
import sys
import threading
import time
from collections.abc import Sequence


MAX_REQUEST_BYTES = 64 * 1024
LOCAL_PROXY = re.compile(r"^http://127\.0\.0\.1:[0-9]{1,5}$")
FIXED_SUBSTITUTERS = ("https://cache.nixos.org",)
FIXED_TRUSTED_KEYS = (
    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=",
)
FDPROXY_PROTOCOL = "fdproxy/1"


class CheckRunnerError(RuntimeError):
    """Raised when a local check would leave the contributor boundary."""


def _path(value: str, label: str) -> pathlib.Path:
    path = pathlib.Path(value)
    if (
        not path.is_absolute()
        or any(part == ".." for part in path.parts)
        or os.path.normpath(value) != value
    ):
        raise CheckRunnerError(f"{label} must be an absolute normalized path")
    return path


def _private_directory(value: str, label: str) -> pathlib.Path:
    path = _path(value, label)
    path.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
        raise CheckRunnerError(f"{label} is not a private directory")
    if info.st_uid != os.geteuid() or info.st_mode & 0o077:
        raise CheckRunnerError(f"{label} is not service-owned and private")
    return path


def _under(path: pathlib.Path, root: pathlib.Path, label: str) -> None:
    try:
        path.relative_to(root)
    except ValueError as error:
        raise CheckRunnerError(f"{label} is outside the approved root") from error


def _validate_proxy(value: str) -> str:
    if LOCAL_PROXY.fullmatch(value) is None:
        raise CheckRunnerError("check proxy must be a loopback HTTP endpoint")
    port = int(value.rsplit(":", 1)[1])
    if not 1024 <= port <= 65535:
        raise CheckRunnerError("check proxy port is outside the unprivileged range")
    return value


def _validate_substituters(values: Sequence[str]) -> list[str]:
    if tuple(values) != FIXED_SUBSTITUTERS:
        raise CheckRunnerError("substituters are fixed to cache.nixos.org")
    return list(values)


def _read_headers(client: socket.socket) -> tuple[str, int]:
    payload = bytearray()
    while b"\r\n\r\n" not in payload:
        chunk = client.recv(4096)
        if not chunk:
            raise CheckRunnerError("proxy client closed before CONNECT")
        payload.extend(chunk)
        if len(payload) > 32 * 1024:
            raise CheckRunnerError("proxy headers exceed the size limit")
    first = bytes(payload).split(b"\r\n", 1)[0]
    try:
        method, authority, version = first.decode("ascii").split(" ", 2)
        host, port_text = authority.rsplit(":", 1)
        port = int(port_text)
    except (UnicodeDecodeError, ValueError) as error:
        raise CheckRunnerError("proxy CONNECT request is malformed") from error
    if method != "CONNECT" or version != "HTTP/1.1" or not host or port != 443:
        raise CheckRunnerError("only HTTPS CONNECT is permitted")
    if not 1 <= port <= 65535:
        raise CheckRunnerError("proxy port is malformed")
    return host, port


def _receive_fd(channel: socket.socket, request_id: str) -> socket.socket | None:
    data = bytearray()
    fds: list[int] = []
    item_size = array.array("i").itemsize
    while b"\n" not in data:
        chunk, ancillary, flags, _address = channel.recvmsg(
            8192,
            socket.CMSG_SPACE(item_size),
        )
        if flags & getattr(socket, "MSG_CTRUNC", 0):
            raise CheckRunnerError("egress response ancillary data was truncated")
        for level, kind, raw in ancillary:
            if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS:
                raise CheckRunnerError("egress response has unauthorized ancillary data")
            values = array.array("i")
            values.frombytes(raw[: len(raw) - (len(raw) % item_size)])
            fds.extend(values)
        if not chunk:
            raise CheckRunnerError("egress response closed")
        data.extend(chunk)
        if len(data) > 8192:
            raise CheckRunnerError("egress response is too large")
    response = json.loads(bytes(data).split(b"\n", 1)[0])
    if (
        not isinstance(response, dict)
        or response.get("version") != FDPROXY_PROTOCOL
        or response.get("request_id") != request_id
        or type(response.get("ok")) is not bool
    ):
        raise CheckRunnerError("egress response is malformed")
    if not response["ok"]:
        for descriptor in fds:
            os.close(descriptor)
        return None
    if len(fds) != 1:
        for descriptor in fds:
            os.close(descriptor)
        raise CheckRunnerError("egress response did not pass exactly one fd")
    descriptor = fds[0]
    os.set_inheritable(descriptor, False)
    return socket.socket(fileno=descriptor)


def _connect_egress(
    socket_path: str,
    auth_token: str,
    host: str,
    port: int,
    request_id: str,
) -> socket.socket | None:
    channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        channel.connect(socket_path)
        channel.sendall(
            json.dumps(
                {
                    "version": FDPROXY_PROTOCOL,
                    "operation": "connect",
                    "request_id": request_id,
                    "auth": auth_token,
                    "host": host,
                    "port": port,
                },
                separators=(",", ":"),
            ).encode()
            + b"\n"
        )
        return _receive_fd(channel, request_id)
    finally:
        channel.close()


def _relay(client: socket.socket, upstream: socket.socket) -> None:
    selector = selectors.DefaultSelector()
    selector.register(client, selectors.EVENT_READ, upstream)
    selector.register(upstream, selectors.EVENT_READ, client)
    try:
        while selector.get_map():
            for key, _mask in selector.select():
                data = key.fileobj.recv(64 * 1024)
                if not data:
                    return
                key.data.sendall(data)
    finally:
        selector.close()


def serve_local_proxy(
    *,
    socket_path: str,
    auth_token: str,
    listen_port: int,
    stop: threading.Event,
) -> None:
    """Provide the runner's local HTTP CONNECT side of fdproxy/1."""

    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
    listener.bind(("127.0.0.1", listen_port))
    listener.listen(32)
    listener.settimeout(0.2)

    def handle(client: socket.socket, request_id: str) -> None:
        upstream: socket.socket | None = None
        try:
            host, port = _read_headers(client)
            upstream = _connect_egress(socket_path, auth_token, host, port, request_id)
            if upstream is None:
                client.sendall(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                return
            client.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            _relay(client, upstream)
        except (CheckRunnerError, OSError, json.JSONDecodeError):
            try:
                client.sendall(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            except OSError:
                pass
        finally:
            if upstream is not None:
                upstream.close()
            client.close()

    counter = 0
    try:
        while not stop.is_set():
            try:
                client, _address = listener.accept()
            except socket.timeout:
                continue
            counter += 1
            threading.Thread(
                target=handle,
                args=(client, str(counter)),
                daemon=True,
            ).start()
    finally:
        listener.close()


def check_environment(
    *,
    store_root: str,
    output_root: str,
    proxy: str,
    max_jobs: int,
    build_cores: int,
    substituters: Sequence[str] = FIXED_SUBSTITUTERS,
    trusted_keys: Sequence[str] = FIXED_TRUSTED_KEYS,
) -> dict[str, str]:
    if max_jobs < 1 or build_cores < 1:
        raise CheckRunnerError("local Nix jobs and cores must be positive")
    store = _private_directory(store_root, "local Nix store")
    output = _private_directory(output_root, "check output root")
    _validate_proxy(proxy)
    _validate_substituters(substituters)
    if tuple(trusted_keys) != FIXED_TRUSTED_KEYS:
        raise CheckRunnerError("trusted Nix keys are fixed")
    config = "\n".join(
        (
            "connect-timeout = 5",
            f"max-jobs = {max_jobs}",
            f"cores = {build_cores}",
            "builders = ",
            f"substituters = {' '.join(FIXED_SUBSTITUTERS)}",
            f"trusted-public-keys = {' '.join(FIXED_TRUSTED_KEYS)}",
            f"http-proxy = {proxy}",
            f"https-proxy = {proxy}",
        )
    )
    return {
        "HOME": str(output / "home"),
        "XDG_CONFIG_HOME": str(output / "home/.config"),
        "XDG_CACHE_HOME": str(output / "cache"),
        "NIX_REMOTE": f"local?root={store}",
        "NIX_STORE_DIR": str(store / "store"),
        "NIX_STATE_DIR": str(store / "state"),
        "NIX_PATH": "",
        "NIX_USER_CONF_FILES": "/dev/null",
        "NIX_CONFIG": config,
        "http_proxy": proxy,
        "https_proxy": proxy,
        "HTTP_PROXY": proxy,
        "HTTPS_PROXY": proxy,
        "NO_PROXY": "",
        "PATH": os.environ.get("PATH", "/run/current-system/sw/bin"),
        "LANG": "C",
    }


def _validate_worktree(worktree_value: str, snapshot_root: pathlib.Path) -> pathlib.Path:
    worktree = _path(worktree_value, "worktree")
    if not snapshot_root.is_dir() or snapshot_root.is_symlink():
        raise CheckRunnerError("snapshot root must be an existing directory")
    if not worktree.is_dir() or worktree.is_symlink():
        raise CheckRunnerError("worktree must be an existing directory")
    _under(worktree, snapshot_root, "worktree")
    # The systemd unit also carries ReadOnlyPaths for the snapshot root.  This
    # check rejects an operator accidentally pointing at a durable state tree.
    if worktree == snapshot_root:
        raise CheckRunnerError("worktree must be below the snapshot root")
    current = worktree
    while True:
        info = os.lstat(current)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise CheckRunnerError("worktree has a symlinked or non-directory ancestor")
        if current == snapshot_root:
            break
        current = current.parent
    return worktree


def run_check(
    command: Sequence[str],
    *,
    worktree: str,
    snapshot_root: str,
    store_root: str,
    output_root: str,
    proxy: str,
    max_jobs: int,
    build_cores: int,
    snapshot_fd: int | None = None,
) -> int:
    if not command or len(command) > 64:
        raise CheckRunnerError("check command is empty or too large")
    if any(
        not isinstance(argument, str)
        or len(argument.encode("utf-8")) > 8192
        or "\x00" in argument
        for argument in command
    ):
        raise CheckRunnerError("check command argument is malformed")
    snapshot = _path(snapshot_root, "snapshot root")
    _validate_worktree(worktree, snapshot)
    environment = check_environment(
        store_root=store_root,
        output_root=output_root,
        proxy=proxy,
        max_jobs=max_jobs,
        build_cores=build_cores,
    )
    output = pathlib.Path(output_root)
    pathlib.Path(environment["HOME"]).mkdir(mode=0o700, parents=True, exist_ok=True)
    pathlib.Path(environment["XDG_CONFIG_HOME"]).mkdir(mode=0o700, parents=True, exist_ok=True)
    pathlib.Path(environment["XDG_CACHE_HOME"]).mkdir(mode=0o700, parents=True, exist_ok=True)
    environment["GC_CHECK_WORKTREE"] = worktree
    environment["GC_CHECK_OUTPUT_ROOT"] = str(output)
    environment["GC_CHECK_UNPRIVILEGED_LOCAL_STORE"] = environment["NIX_REMOTE"]
    if snapshot_fd is not None:
        if snapshot_fd < 3:
            raise CheckRunnerError("snapshot fd overlaps standard descriptors")
        os.fstat(snapshot_fd)
        os.set_inheritable(snapshot_fd, True)
        environment["GC_CHECK_WORKTREE_FD"] = str(snapshot_fd)
    try:
        completed = subprocess.run(
            list(command),
            cwd=worktree,
            env=environment,
            close_fds=True,
            pass_fds=(snapshot_fd,) if snapshot_fd is not None else (),
            check=False,
        )
    except OSError as error:
        raise CheckRunnerError("approved check could not be started") from error
    finally:
        if snapshot_fd is not None:
            os.set_inheritable(snapshot_fd, False)
    return completed.returncode


def serve(args: argparse.Namespace) -> int:
    if args.max_heavy_checks < 1:
        raise CheckRunnerError("max-heavy-checks must be positive")
    store = _private_directory(args.store_root, "local Nix store")
    output = _private_directory(args.output_root, "check output root")
    _validate_proxy(args.proxy)
    stop_proxy = threading.Event()
    proxy_thread = threading.Thread(
        target=serve_local_proxy,
        kwargs={
            "socket_path": args.egress_socket,
            "auth_token": os.environ.get(args.auth_token_env, ""),
            "listen_port": 3128,
            "stop": stop_proxy,
        },
        daemon=True,
    )
    proxy_thread.start()
    if args.socket is None:
        try:
            while True:
                time.sleep(60)
        finally:
            stop_proxy.set()
            proxy_thread.join(timeout=1)
    socket_path = _path(args.socket, "check socket")
    if os.path.lexists(socket_path):
        if not stat.S_ISSOCK(os.lstat(socket_path).st_mode):
            raise CheckRunnerError("check socket path is occupied")
        socket_path.unlink()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(socket_path))
    os.chmod(socket_path, 0o660)
    listener.listen(8)
    listener.settimeout(0.5)
    stop = False

    def stop_server(_signum: int, _frame: object) -> None:
        nonlocal stop
        stop = True

    previous = signal.signal(signal.SIGTERM, stop_server)
    try:
        while not stop:
            try:
                client, _address = listener.accept()
            except socket.timeout:
                continue
            try:
                payload = client.recv(MAX_REQUEST_BYTES + 1)
                if len(payload) > MAX_REQUEST_BYTES:
                    raise CheckRunnerError("check request exceeds the size limit")
                request = json.loads(payload)
                if not isinstance(request, dict):
                    raise CheckRunnerError("check request is not an object")
                command = request.get("command")
                worktree = request.get("worktree")
                if not isinstance(command, list) or not all(
                    isinstance(value, str) for value in command
                ) or not isinstance(worktree, str):
                    raise CheckRunnerError("check request shape is malformed")
                return_code = run_check(
                    command,
                    worktree=worktree,
                    snapshot_root=args.snapshot_root,
                    store_root=str(store),
                    output_root=str(output),
                    proxy=args.proxy,
                    max_jobs=args.max_jobs,
                    build_cores=args.build_cores,
                )
                response = {"ok": return_code == 0, "returncode": return_code}
            except (CheckRunnerError, OSError, json.JSONDecodeError) as error:
                response = {"ok": False, "error": str(error)[:512]}
            try:
                client.sendall(json.dumps(response, sort_keys=True).encode() + b"\n")
            finally:
                client.close()
    finally:
        signal.signal(signal.SIGTERM, previous)
        listener.close()
        socket_path.unlink(missing_ok=True)
        stop_proxy.set()
        proxy_thread.join(timeout=1)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    run_parser = subparsers.add_parser("run")
    run_parser.add_argument("--worktree", required=True)
    run_parser.add_argument("--snapshot-root", required=True)
    run_parser.add_argument("--store-root", required=True)
    run_parser.add_argument("--output-root", required=True)
    run_parser.add_argument("--proxy", required=True)
    run_parser.add_argument("--max-jobs", type=int, default=1)
    run_parser.add_argument("--build-cores", type=int, default=2)
    run_parser.add_argument("--snapshot-fd", type=int)
    run_parser.add_argument("argv", nargs=argparse.REMAINDER)

    server_parser = subparsers.add_parser("server")
    server_parser.add_argument("--store-root", required=True)
    server_parser.add_argument("--output-root", required=True)
    server_parser.add_argument("--proxy", required=True)
    server_parser.add_argument("--egress-socket", required=True)
    server_parser.add_argument("--auth-token-env", default="GC_FDPROXY_AUTH")
    server_parser.add_argument("--snapshot-root", default="/var/lib/gascity-contributor/state/worktrees")
    server_parser.add_argument("--socket")
    server_parser.add_argument("--max-jobs", type=int, default=1)
    server_parser.add_argument("--build-cores", type=int, default=2)
    server_parser.add_argument("--max-heavy-checks", type=int, default=1)
    args = parser.parse_args(argv)
    if args.command == "run":
        command = list(args.argv)
        if command[:1] == ["--"]:
            command = command[1:]
        return run_check(
            command,
            worktree=args.worktree,
            snapshot_root=args.snapshot_root,
            store_root=args.store_root,
            output_root=args.output_root,
            proxy=args.proxy,
            max_jobs=args.max_jobs,
            build_cores=args.build_cores,
            snapshot_fd=args.snapshot_fd,
        )
    return serve(args)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CheckRunnerError, OSError) as error:
        print(f"check rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
