#!/usr/bin/env python3
"""Run approved checks against a read-only worktree and a local Nix store."""

from __future__ import annotations

import argparse
import array
import hashlib
import hmac
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
from collections.abc import Mapping, Sequence


MAX_REQUEST_BYTES = 64 * 1024
CHECK_PROTOCOL = "gascity-check/1"
CHECK_IDENTIFIER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")
DEFAULT_APPROVED_CHECK = "build-artifact-valid=.gc/scripts/checks/build-artifact-valid.sh"
MAX_CHECK_TIMEOUT_SECONDS = 86400.0
CHECK_TERM_GRACE_SECONDS = 2.0
CHECK_KILL_GRACE_SECONDS = 1.0
CHECK_CLIENT_TIMEOUT_SECONDS = (
    MAX_CHECK_TIMEOUT_SECONDS
    + CHECK_TERM_GRACE_SECONDS
    + CHECK_KILL_GRACE_SECONDS
    + 1.0
)
LOCAL_PROXY = re.compile(r"^http://127\.0\.0\.1:[0-9]{1,5}$")
FIXED_SUBSTITUTERS = ("https://cache.nixos.org",)
FIXED_TRUSTED_KEYS = (
    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY=",
)
FDPROXY_PROTOCOL = "fdproxy/1"


class CheckRunnerError(RuntimeError):
    """Raised when a local check would leave the contributor boundary."""

    error_code = "check_error"


class MalformedRequestError(CheckRunnerError):
    error_code = "malformed_request"


class UnauthorizedRequestError(CheckRunnerError):
    error_code = "unauthorized"


class UnknownCheckError(CheckRunnerError):
    error_code = "unknown_check"


class CheckTimeoutError(CheckRunnerError):
    error_code = "timeout"


class ServiceStoppingError(CheckRunnerError):
    error_code = "service_stopping"


def _validate_identifier(value: object, label: str) -> str:
    if not isinstance(value, str) or not CHECK_IDENTIFIER.fullmatch(value) or ".." in value:
        raise MalformedRequestError(f"{label} is malformed")
    return value


def bind_auth_token(
    auth_token: str,
    *,
    run_id: str,
    bead_id: str,
    worktree: str,
) -> str:
    if not isinstance(auth_token, str) or not auth_token:
        raise CheckRunnerError("check authentication is not configured")
    message = "\0".join((run_id, bead_id, worktree)).encode("utf-8")
    return hmac.new(
        auth_token.encode("utf-8"),
        message,
        hashlib.sha256,
    ).hexdigest()


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


def _approved_checks(values: Sequence[str]) -> dict[str, str]:
    checks: dict[str, str] = {}
    for value in values:
        if not isinstance(value, str) or "=" not in value:
            raise CheckRunnerError("approved check must be NAME=relative-path")
        name, relative = value.split("=", 1)
        name = _validate_identifier(name, "approved check name")
        if (
            not relative
            or pathlib.PurePosixPath(relative).is_absolute()
            or "\x00" in relative
            or os.path.normpath(relative) != relative
            or any(part in {"", ".."} for part in pathlib.PurePosixPath(relative).parts)
        ):
            raise CheckRunnerError(
                f"approved check path is not a normalized relative path: {name}"
            )
        if name in checks:
            raise CheckRunnerError(f"approved check is configured more than once: {name}")
        checks[name] = relative
    if not checks:
        raise CheckRunnerError("at least one approved check is required")
    return checks


def _approved_command(
    check_name: str,
    *,
    worktree: pathlib.Path,
    approved_checks: Mapping[str, str],
) -> list[str]:
    name = _validate_identifier(check_name, "check name")
    relative = approved_checks.get(name)
    if relative is None:
        raise UnknownCheckError(f"check is not approved: {name}")
    candidate = worktree / relative
    try:
        resolved = candidate.resolve(strict=True)
    except OSError as error:
        raise CheckRunnerError(f"approved check is unavailable: {name}") from error
    _under(resolved, worktree.resolve(), f"approved check {name}")
    if candidate.is_symlink() or not resolved.is_file():
        raise CheckRunnerError(f"approved check is not a regular file: {name}")
    if not os.access(resolved, os.X_OK):
        raise CheckRunnerError(f"approved check is not executable: {name}")
    return [str(resolved)]


class ActiveCheckProcesses:
    """Track check process groups so service shutdown can terminate them."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._processes: set[subprocess.Popen[bytes]] = set()

    def add(self, process: subprocess.Popen[bytes]) -> None:
        with self._lock:
            self._processes.add(process)

    def remove(self, process: subprocess.Popen[bytes]) -> None:
        with self._lock:
            self._processes.discard(process)

    def terminate_all(self, *, term_grace: float, kill_grace: float) -> None:
        with self._lock:
            processes = tuple(self._processes)
        for process in processes:
            _terminate_process_group(
                process,
                term_grace=term_grace,
                kill_grace=kill_grace,
            )


def _signal_process_group(process: subprocess.Popen[bytes], signum: int) -> None:
    try:
        os.killpg(process.pid, signum)
    except ProcessLookupError:
        pass


def _terminate_process_group(
    process: subprocess.Popen[bytes],
    *,
    term_grace: float,
    kill_grace: float,
) -> None:
    _signal_process_group(process, signal.SIGTERM)
    try:
        process.wait(timeout=term_grace)
    except subprocess.TimeoutExpired:
        pass
    else:
        return
    _signal_process_group(process, signal.SIGKILL)
    try:
        process.wait(timeout=kill_grace)
    except subprocess.TimeoutExpired:
        try:
            process.kill()
        except ProcessLookupError:
            pass
        process.wait()


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
    timeout_seconds: float = 300.0,
    term_grace: float = 2.0,
    kill_grace: float = 1.0,
    processes: ActiveCheckProcesses | None = None,
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
    if timeout_seconds <= 0 or term_grace <= 0 or kill_grace <= 0:
        raise CheckRunnerError("check timeout and termination grace periods must be positive")
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
    process: subprocess.Popen[bytes] | None = None
    try:
        try:
            process = subprocess.Popen(
                list(command),
                cwd=worktree,
                env=environment,
                close_fds=True,
                pass_fds=(snapshot_fd,) if snapshot_fd is not None else (),
                start_new_session=True,
            )
        except OSError as error:
            raise CheckRunnerError("approved check could not be started") from error
        if processes is not None:
            processes.add(process)
        try:
            try:
                return process.wait(timeout=timeout_seconds)
            except subprocess.TimeoutExpired as error:
                _terminate_process_group(
                    process,
                    term_grace=term_grace,
                    kill_grace=kill_grace,
                )
                raise CheckTimeoutError(
                    f"approved check timed out after {timeout_seconds:g} seconds"
                ) from error
        finally:
            if processes is not None:
                processes.remove(process)
    finally:
        if snapshot_fd is not None:
            os.set_inheritable(snapshot_fd, False)


def _receive_frame(client: socket.socket) -> object:
    payload = bytearray()
    while True:
        chunk = client.recv(min(4096, MAX_REQUEST_BYTES + 1 - len(payload)))
        if not chunk:
            raise CheckRunnerError("check channel closed")
        newline = chunk.find(b"\n")
        if newline >= 0:
            payload.extend(chunk[:newline])
            if chunk[newline + 1 :]:
                raise MalformedRequestError("check channel pipelined multiple frames")
            break
        payload.extend(chunk)
        if len(payload) > MAX_REQUEST_BYTES:
            raise MalformedRequestError("check request exceeds the size limit")
    try:
        return json.loads(bytes(payload))
    except json.JSONDecodeError as error:
        raise MalformedRequestError("check request JSON is malformed") from error


def _send_response(client: socket.socket, response: Mapping[str, object]) -> None:
    client.sendall(
        json.dumps(response, sort_keys=True, separators=(",", ":")).encode("utf-8")
        + b"\n"
    )


def _request_response(error: CheckRunnerError) -> dict[str, object]:
    return {
        "protocol": CHECK_PROTOCOL,
        "ok": False,
        "error_code": error.error_code,
        "error": str(error)[:512],
    }


def _peer_uid(client: socket.socket) -> int:
    try:
        raw = client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
        if len(raw) < 8:
            raise OSError("peer credentials are truncated")
        return int.from_bytes(raw[4:8], byteorder=sys.byteorder, signed=True)
    except OSError as error:
        raise UnauthorizedRequestError("check peer credentials are unavailable") from error


def _bind_request(
    request: object,
    *,
    auth_token: str,
    snapshot_root: pathlib.Path,
) -> tuple[str, str, pathlib.Path]:
    if not isinstance(request, dict):
        raise MalformedRequestError("check bind request is not an object")
    required = {"protocol", "operation", "run_id", "bead_id", "worktree", "auth"}
    if set(request) != required:
        raise MalformedRequestError("check bind request shape is malformed")
    if request.get("protocol") != CHECK_PROTOCOL or request.get("operation") != "bind":
        raise MalformedRequestError("check bind protocol or operation is unsupported")
    run_id = _validate_identifier(request.get("run_id"), "run id")
    bead_id = _validate_identifier(request.get("bead_id"), "bead id")
    worktree_value = request.get("worktree")
    if not isinstance(worktree_value, str):
        raise MalformedRequestError("worktree is malformed")
    supplied_auth = request.get("auth")
    if not isinstance(supplied_auth, str):
        raise MalformedRequestError("check authentication is malformed")
    expected_auth = bind_auth_token(
        auth_token,
        run_id=run_id,
        bead_id=bead_id,
        worktree=worktree_value,
    )
    if not hmac.compare_digest(supplied_auth, expected_auth):
        raise UnauthorizedRequestError("check authentication failed")
    worktree = _validate_worktree(worktree_value, snapshot_root)
    return run_id, bead_id, worktree


def _run_request(request: object) -> str:
    if not isinstance(request, dict):
        raise MalformedRequestError("check run request is not an object")
    required = {"protocol", "operation", "request_id", "check"}
    if set(request) != required:
        raise MalformedRequestError("check run request shape is malformed")
    if request.get("protocol") != CHECK_PROTOCOL or request.get("operation") != "run":
        raise MalformedRequestError("check run protocol or operation is unsupported")
    _validate_identifier(request.get("request_id"), "request id")
    return _validate_identifier(request.get("check"), "check name")


def _serve_connection(
    client: socket.socket,
    *,
    args: argparse.Namespace,
    snapshot_root: pathlib.Path,
    store: pathlib.Path,
    output: pathlib.Path,
    approved_checks: Mapping[str, str],
    slots: threading.BoundedSemaphore,
    processes: ActiveCheckProcesses,
    stop_event: threading.Event,
    allowed_uids: set[int],
    check_auth: str,
) -> None:
    client.settimeout(5.0)
    if _peer_uid(client) not in allowed_uids:
        raise UnauthorizedRequestError("check peer identity is not authorized")
    run_id: str | None = None
    bead_id: str | None = None
    worktree: pathlib.Path | None = None
    while not stop_event.is_set():
        request = _receive_frame(client)
        if worktree is None:
            run_id, bead_id, worktree = _bind_request(
                request,
                auth_token=check_auth,
                snapshot_root=snapshot_root,
            )
            client.settimeout(None)
            _send_response(
                client,
                {
                    "protocol": CHECK_PROTOCOL,
                    "ok": True,
                    "operation": "bind",
                    "run_id": run_id,
                    "bead_id": bead_id,
                },
            )
            continue
        check_name = _run_request(request)
        while not stop_event.is_set():
            if slots.acquire(timeout=0.1):
                break
        else:
            raise ServiceStoppingError("check service is stopping")
        try:
            command = _approved_command(
                check_name,
                worktree=worktree,
                approved_checks=approved_checks,
            )
            return_code = run_check(
                command,
                worktree=str(worktree),
                snapshot_root=str(snapshot_root),
                store_root=str(store),
                output_root=str(output),
                proxy=args.proxy,
                max_jobs=args.max_jobs,
                build_cores=args.build_cores,
                timeout_seconds=args.timeout_seconds,
                term_grace=args.term_grace,
                kill_grace=args.kill_grace,
                processes=processes,
            )
            response = {
                "protocol": CHECK_PROTOCOL,
                "ok": return_code == 0,
                "returncode": return_code,
            }
        except CheckRunnerError as error:
            response = _request_response(error)
        except (OSError, ValueError) as error:
            response = _request_response(CheckRunnerError(str(error)))
        finally:
            slots.release()
        _send_response(client, response)


def serve(args: argparse.Namespace) -> int:
    if args.max_heavy_checks < 1:
        raise CheckRunnerError("max-heavy-checks must be positive")
    if args.listen_port < 1024 or args.listen_port > 65535:
        raise CheckRunnerError("check proxy port is outside the unprivileged range")
    if not args.allowed_uid:
        raise CheckRunnerError("at least one allowed check caller uid is required")
    if args.socket is None:
        raise CheckRunnerError("check socket is required")
    store = _private_directory(args.store_root, "local Nix store")
    output = _private_directory(args.output_root, "check output root")
    snapshot_root = _path(args.snapshot_root, "snapshot root")
    _validate_proxy(args.proxy)
    approved_checks = _approved_checks(
        args.approved_check or [DEFAULT_APPROVED_CHECK]
    )
    check_auth = os.environ.get(args.check_auth_token_env, "")
    if not check_auth:
        raise CheckRunnerError("check authentication token is not configured")
    stop_proxy = threading.Event()
    proxy_thread = threading.Thread(
        target=serve_local_proxy,
        kwargs={
            "socket_path": args.egress_socket,
            "auth_token": os.environ.get(args.auth_token_env, ""),
            "listen_port": args.listen_port,
            "stop": stop_proxy,
        },
        daemon=True,
    )
    proxy_thread.start()
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
    stop_event = threading.Event()
    slots = threading.BoundedSemaphore(args.max_heavy_checks)
    processes = ActiveCheckProcesses()
    workers: set[threading.Thread] = set()
    workers_lock = threading.Lock()
    previous = signal.signal(signal.SIGTERM, lambda _signum, _frame: stop_event.set())
    previous_int = signal.signal(signal.SIGINT, lambda _signum, _frame: stop_event.set())

    def worker(client: socket.socket) -> None:
        current = threading.current_thread()
        try:
            _serve_connection(
                client,
                args=args,
                snapshot_root=snapshot_root,
                store=store,
                output=output,
                approved_checks=approved_checks,
                slots=slots,
                processes=processes,
                stop_event=stop_event,
                allowed_uids=set(args.allowed_uid),
                check_auth=check_auth,
            )
        except (CheckRunnerError, OSError, json.JSONDecodeError) as error:
            try:
                _send_response(client, _request_response(
                    error if isinstance(error, CheckRunnerError) else CheckRunnerError(str(error))
                ))
            except OSError:
                pass
        finally:
            client.close()
            with workers_lock:
                workers.discard(current)

    try:
        while not stop_event.is_set():
            try:
                client, _address = listener.accept()
            except socket.timeout:
                continue
            thread = threading.Thread(target=worker, args=(client,), daemon=True)
            with workers_lock:
                workers.add(thread)
            thread.start()
    finally:
        stop_event.set()
        listener.close()
        processes.terminate_all(
            term_grace=args.term_grace,
            kill_grace=args.kill_grace,
        )
        deadline = time.monotonic() + args.term_grace + args.kill_grace + 1.0
        while True:
            with workers_lock:
                active = tuple(workers)
            if not active or time.monotonic() >= deadline:
                break
            for thread in active:
                thread.join(timeout=0.1)
        signal.signal(signal.SIGTERM, previous)
        signal.signal(signal.SIGINT, previous_int)
        socket_path.unlink(missing_ok=True)
        stop_proxy.set()
        proxy_thread.join(timeout=1)
    return 0


def request_check(*, fd: int, check_name: str) -> int:
    if fd < 3:
        raise CheckRunnerError("check fd must not overlap standard descriptors")
    _validate_identifier(check_name, "check name")
    descriptor = os.dup(fd)
    client = socket.socket(fileno=descriptor)
    try:
        client.settimeout(CHECK_CLIENT_TIMEOUT_SECONDS)
        _send_response(
            client,
            {
                "protocol": CHECK_PROTOCOL,
                "operation": "run",
                "request_id": f"request-{time.monotonic_ns()}",
                "check": check_name,
            },
        )
        response = _receive_frame(client)
        if (
            not isinstance(response, dict)
            or response.get("protocol") != CHECK_PROTOCOL
            or type(response.get("ok")) is not bool
        ):
            raise CheckRunnerError("check response is malformed")
        if response["ok"]:
            return 0
        return_code = response.get("returncode")
        if type(return_code) is int and 0 <= return_code <= 255:
            return return_code
        error_code = response.get("error_code", "check_error")
        error = response.get("error", "approved check failed")
        raise CheckRunnerError(f"{error_code}: {error}")
    finally:
        client.close()


def main(argv: list[str] | None = None) -> int:
    if argv is None and pathlib.Path(sys.argv[0]).name == "gascity-check":
        argv = ["request", *sys.argv[1:]]
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
    server_parser.add_argument("--check-auth-token-env", default="GC_CHECK_AUTH")
    server_parser.add_argument("--allowed-uid", action="append", type=int, default=[])
    server_parser.add_argument("--snapshot-root", default="/var/lib/gascity-contributor/state/worktrees")
    server_parser.add_argument("--socket")
    server_parser.add_argument("--max-jobs", type=int, default=1)
    server_parser.add_argument("--build-cores", type=int, default=2)
    server_parser.add_argument("--max-heavy-checks", type=int, default=1)
    server_parser.add_argument("--timeout-seconds", type=float, default=300.0)
    server_parser.add_argument("--term-grace", type=float, default=2.0)
    server_parser.add_argument("--kill-grace", type=float, default=1.0)
    server_parser.add_argument("--listen-port", type=int, default=3128)
    server_parser.add_argument(
        "--approved-check",
        action="append",
        default=[],
    )
    request_parser = subparsers.add_parser("request")
    request_parser.add_argument("--check", required=True)
    request_parser.add_argument("--fd", type=int)
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
    if args.command == "server":
        return serve(args)
    descriptor = args.fd
    if descriptor is None:
        raw_descriptor = os.environ.get("GC_CHECK_FD")
        if not raw_descriptor:
            raise CheckRunnerError("GC_CHECK_FD is not configured")
        try:
            descriptor = int(raw_descriptor)
        except ValueError as error:
            raise CheckRunnerError("GC_CHECK_FD is malformed") from error
    return request_check(fd=descriptor, check_name=args.check)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (CheckRunnerError, OSError) as error:
        print(f"check rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
