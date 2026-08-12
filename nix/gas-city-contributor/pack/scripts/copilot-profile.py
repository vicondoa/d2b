#!/usr/bin/env python3
"""Authority and ACP stdio proxy for the three pinned Copilot profiles."""

from __future__ import annotations

import argparse
import array
import errno
import hashlib
import hmac
import json
import os
import pathlib
import re
import select
import signal
import socket
import shutil
import struct
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Mapping, Sequence


PROFILE_SETTINGS = {
    "review-sol": {
        "model": "gpt-5.6-sol",
        "contextTier": "long_context",
    },
    "review-luna": {
        "model": "gpt-5.6-luna",
        "contextTier": "long_context",
    },
    "code-luna": {
        "model": "gpt-5.6-luna",
        "contextTier": "default",
    },
}
PROFILE_EFFORT = {
    "review-sol": "xhigh",
    "review-luna": "max",
    "code-luna": "max",
}
SANDBOX_WORKSPACE = "/workspace"
ACTIVE_MODEL_KEYS = (
    "currentModelId",
    "current_model_id",
    "effectiveModel",
    "effective_model",
)
PROFILE_NAMES = frozenset(PROFILE_SETTINGS)
TOOL_POLICIES = {
    "review": "view,search",
    "planning": "view,search,apply_patch",
    "coding": "bash,view,search,apply_patch",
}
ALLOWED_ENV_NAMES = frozenset(
    {
        "ALL_PROXY",
        "GC_AGENT_FD",
        "GC_BEAD_ID",
        "GC_CITY_GENERATION",
        "GC_CONTROL_FD",
        "GC_FDPROXY_FD",
        "GC_FDPROXY_AUTH",
        "GC_PROXY_FD",
        "GC_STATE_SCHEMA",
        "GC_RUN_ID",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "LC_LANG",
        "LC_MESSAGES",
        "NO_PROXY",
        "PATH",
        "SSL_CERT_FILE",
        "TERM",
        "TMPDIR",
        "XDG_RUNTIME_DIR",
    }
)
IDENTIFIER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")


class ProfileError(RuntimeError):
    """Raised when the immutable profile authority cannot be established."""


def _identifier(value: str, label: str) -> str:
    if not IDENTIFIER_PATTERN.fullmatch(value) or ".." in value:
        raise ProfileError(f"{label} is malformed")
    return value


def profile_root() -> pathlib.Path:
    configured = os.environ.get("GC_PROFILE_ROOT")
    if configured:
        return pathlib.Path(configured).resolve()
    configured_root = os.environ.get("GC_CONTRIBUTOR_ROOT")
    if configured_root:
        return (pathlib.Path(configured_root) / "copilot").resolve()
    return (pathlib.Path(__file__).resolve().parents[2] / "copilot").resolve()


def settings_path(profile: str, root: str | os.PathLike[str] | None = None) -> pathlib.Path:
    if profile not in PROFILE_NAMES:
        raise ProfileError(f"unknown Copilot profile: {profile}")
    path = pathlib.Path(root or profile_root()) / profile / "settings.json"
    if not path.is_absolute() or any(part == ".." for part in path.parts):
        raise ProfileError("profile settings path is not absolute and normalized")
    return path


def load_profile(profile: str, root: str | os.PathLike[str] | None = None) -> dict[str, str]:
    path = settings_path(profile, root)
    try:
        with path.open("r", encoding="utf-8") as stream:
            settings = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise ProfileError(f"cannot read immutable settings for {profile}") from error
    expected = PROFILE_SETTINGS[profile]
    if settings != expected:
        raise ProfileError(f"settings authority mismatch for {profile}")
    return dict(expected)


def scrub_environment(source: Mapping[str, str] | None = None) -> dict[str, str]:
    source_environment = os.environ if source is None else source
    result: dict[str, str] = {}
    for name, value in source_environment.items():
        if name == "COPILOT_GITHUB_TOKEN" or name in ALLOWED_ENV_NAMES:
            result[name] = value
    return result


def child_argv(
    profile: str,
    *,
    tool_policy: str,
    root: str | os.PathLike[str] | None = None,
) -> list[str]:
    settings = load_profile(profile, root)
    if tool_policy not in TOOL_POLICIES:
        raise ProfileError(f"unknown tool policy: {tool_policy}")
    return [
        "--acp",
        "--model",
        settings["model"],
        "--context",
        settings["contextTier"],
        "--effort",
        PROFILE_EFFORT[profile],
        "--no-custom-instructions",
        "--no-auto-update",
        "--disable-builtin-mcps",
        "--no-remote",
        "--no-remote-export",
        "--secret-env-vars",
        "COPILOT_GITHUB_TOKEN",
        "--available-tools",
        TOOL_POLICIES[tool_policy],
        "--deny-tool",
        "shell(gh)",
        "--deny-tool",
        "shell(gh *)",
        "--deny-tool",
        "shell(git push)",
        "--deny-tool",
        "shell(git push *)",
        "--deny-tool",
        "shell(discord)",
        "--deny-tool",
        "shell(discord *)",
    ]


def _configured_path(
    value: str | None,
    *,
    environment_name: str,
    fallback: str | None,
    label: str,
) -> str:
    candidate = value or os.environ.get(environment_name) or fallback
    if not candidate:
        raise ProfileError(f"{label} is not configured")
    path = pathlib.Path(candidate)
    if not path.is_absolute():
        found = shutil.which(candidate)
        if not found:
            raise ProfileError(f"{label} is not available: {candidate}")
        path = pathlib.Path(found)
    if not path.exists():
        raise ProfileError(f"{label} does not exist: {path}")
    return str(path.resolve())


def _effective_profile(
    profile: str,
    *,
    tool_policy: str,
    args: argparse.Namespace,
) -> str:
    if profile != "review-sol" or tool_policy not in {"review", "planning"}:
        return profile
    if args.probe:
        return profile
    status_value = args.readiness_status or os.environ.get("GC_READINESS_STATUS")
    if not status_value:
        return profile
    metadata = _metadata(args)
    status_path = pathlib.Path(status_value)
    if not status_path.is_absolute() or any(part == ".." for part in status_path.parts):
        raise ProfileError("readiness status path is not absolute and normalized")
    try:
        with status_path.open("r", encoding="utf-8") as stream:
            status = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise ProfileError("readiness status is unreadable or malformed") from error
    if not isinstance(status, dict) or set(status) != {
        "generation",
        "state_schema",
        "ready",
        "effective_profiles",
        "error_code",
    }:
        raise ProfileError("readiness status is not an object")
    if (
        status.get("generation") != metadata["generation"]
        or status.get("state_schema") != metadata["state_schema"]
        or status.get("ready") is not True
        or status.get("error_code") is not None
    ):
        raise ProfileError("readiness status is stale or not ready")
    effective = status.get("effective_profiles")
    selected = effective.get("review") if isinstance(effective, dict) else None
    if (
        not isinstance(effective, dict)
        or set(effective) != {"coding", "review"}
        or effective.get("coding") != "code-luna"
        or selected not in {"review-sol", "review-luna"}
    ):
        raise ProfileError("readiness has no valid review profile")
    return str(selected)


def _metadata(args: argparse.Namespace) -> dict[str, str]:
    run_id = args.run_id or os.environ.get("GC_RUN_ID")
    bead_id = args.bead_id or os.environ.get("GC_BEAD_ID")
    generation = args.generation or os.environ.get("GC_CITY_GENERATION")
    if not run_id or not bead_id or not generation:
        raise ProfileError("run, bead, and city generation metadata are required")
    return {
        "run_id": _identifier(run_id, "run id"),
        "bead_id": _identifier(bead_id, "bead id"),
        "generation": _identifier(generation, "city generation"),
        "state_schema": _identifier(
            args.state_schema or os.environ.get("GC_STATE_SCHEMA", "1"),
            "state schema",
        ),
    }


def _environment_fd(name: str) -> int | None:
    value = os.environ.get(name)
    if value is None:
        return None
    try:
        return int(value, 10)
    except ValueError as error:
        raise ProfileError(f"{name} is not an integer") from error


LAUNCHER_PROTOCOL = "gascity-agent/1"
CHECK_PROTOCOL = "gascity-check/1"
MAX_LAUNCH_METADATA_BYTES = 16 * 1024
MAX_LAUNCH_RESPONSE_BYTES = 8 * 1024
LAUNCHER_FD_NAMES = ("proxy", "progress", "control")


def _absolute_path(value: str | os.PathLike[str], label: str) -> str:
    path = pathlib.Path(value)
    if not path.is_absolute() or any(part == ".." for part in path.parts):
        raise ProfileError(f"{label} must be an absolute normalized path")
    return str(path)


def _configured_server_uid(environment_name: str, label: str) -> int | None:
    value = os.environ.get(environment_name)
    if value is None:
        return None
    try:
        uid = int(value, 10)
    except ValueError as error:
        raise ProfileError(f"{label} is malformed") from error
    if uid < 0:
        raise ProfileError(f"{label} is malformed")
    return uid


def _check_server_uid(
    channel: socket.socket,
    *,
    environment_name: str,
    label: str,
) -> None:
    expected = _configured_server_uid(environment_name, label)
    if expected is None:
        return
    try:
        raw = channel.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
        _pid, uid, _gid = struct.unpack("3i", raw)
    except OSError as error:
        raise ProfileError(f"{label} credentials are unavailable") from error
    if uid != expected:
        raise ProfileError(f"{label} identity is unauthorized")


def _launcher_socket_path(args: argparse.Namespace) -> pathlib.Path:
    configured = args.launcher_socket or os.environ.get("GC_AGENT_LAUNCHER_SOCKET")
    if not configured:
        raise ProfileError("agent launcher socket is not configured")
    path = pathlib.Path(_absolute_path(configured, "agent launcher socket"))
    if path == pathlib.Path("/"):
        raise ProfileError("agent launcher socket is malformed")
    return path


def _check_bind_auth(
    auth_token: str,
    *,
    run_id: str,
    bead_id: str,
    worktree: str,
) -> str:
    if not auth_token:
        raise ProfileError("check authentication is not configured")
    message = "\0".join((run_id, bead_id, worktree)).encode("utf-8")
    return hmac.new(
        auth_token.encode("utf-8"),
        message,
        hashlib.sha256,
    ).hexdigest()


def _open_check_channel(
    *,
    profile: str,
    tool_policy: str,
    identity: Mapping[str, str],
    worktree: pathlib.Path,
) -> int | None:
    if tool_policy != "coding":
        return None
    socket_value = os.environ.get("GC_CHECK_SOCKET")
    auth_token = os.environ.get("GC_CHECK_AUTH")
    if not socket_value and not auth_token:
        return None
    if not socket_value or not auth_token:
        raise ProfileError("check socket and authentication must be configured together")
    socket_path = pathlib.Path(_absolute_path(socket_value, "check socket"))
    channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        channel.settimeout(5.0)
        channel.connect(str(socket_path))
        _check_server_uid(
            channel,
            environment_name="GC_CHECK_SERVER_UID",
            label="check server",
        )
        channel.sendall(
            json.dumps(
                {
                    "protocol": CHECK_PROTOCOL,
                    "operation": "bind",
                    "run_id": identity["run_id"],
                    "bead_id": identity["bead_id"],
                    "worktree": str(worktree),
                    "auth": _check_bind_auth(
                        auth_token,
                        run_id=identity["run_id"],
                        bead_id=identity["bead_id"],
                        worktree=str(worktree),
                    ),
                },
                separators=(",", ":"),
            ).encode("utf-8")
            + b"\n"
        )
        response = _read_socket_line(channel, limit=MAX_LAUNCH_RESPONSE_BYTES)
        if (
            response.get("protocol") != CHECK_PROTOCOL
            or type(response.get("ok")) is not bool
        ):
            raise ProfileError("check bind response is malformed")
        if response["ok"] is not True:
            error = response.get("error")
            raise ProfileError(
                str(error) if isinstance(error, str) and error else "check bind was rejected"
            )
        channel.settimeout(None)
        return channel.detach()
    except (OSError, ProfileError):
        channel.close()
        raise


def _launch_metadata(
    args: argparse.Namespace,
    *,
    profile: str,
    tool_policy: str,
) -> tuple[dict[str, object], list[int]]:
    if profile not in PROFILE_NAMES:
        raise ProfileError(f"unknown Copilot profile: {profile}")
    if tool_policy not in TOOL_POLICIES:
        raise ProfileError(f"unknown tool policy: {tool_policy}")
    identity = _metadata(args)
    worktree = pathlib.Path(
        _absolute_path(
            args.worktree or os.environ.get("GC_WORKTREE", os.getcwd()),
            "assigned worktree",
        )
    )
    metadata: dict[str, object] = {
        "protocol": LAUNCHER_PROTOCOL,
        "operation": "launch",
        "profile": profile,
        "tool_policy": tool_policy,
        **identity,
        "worktree": str(worktree),
        "fds": [],
    }
    state_root = args.state_root or os.environ.get("GC_STATE_ROOT")
    if state_root:
        metadata["state_root"] = _absolute_path(state_root, "state root")
    terminal_state_root = os.environ.get("GC_TERMINAL_STATE_ROOT")
    terminal_state_path = args.terminal_state_path
    if terminal_state_path is None and terminal_state_root:
        terminal_state_path = os.path.join(terminal_state_root, f"{identity['run_id']}.json")
    if terminal_state_path:
        metadata["terminal_state_path"] = _absolute_path(
            terminal_state_path,
            "terminal workflow state",
        )
    if args.require_ready or os.environ.get("GC_REQUIRE_READINESS") == "1":
        metadata["require_ready"] = True
    token = os.environ.get("GC_AGENT_LAUNCHER_TOKEN")
    if token:
        if len(token.encode("utf-8")) > 512 or any(
            ord(character) < 0x21 or ord(character) > 0x7E for character in token
        ):
            raise ProfileError("agent launcher authentication token is malformed")
        metadata["auth"] = token

    descriptors: list[int] = []
    names: list[str] = []
    check_fd = _open_check_channel(
        profile=profile,
        tool_policy=tool_policy,
        identity=identity,
        worktree=worktree,
    )
    if check_fd is not None:
        descriptors.append(check_fd)
        names.append("check")
    for name, environment_name in (
        ("proxy", "GC_PROXY_FD"),
        ("progress", "GC_AGENT_FD"),
        ("control", "GC_CONTROL_FD"),
    ):
        descriptor = args.proxy_fd if name == "proxy" else None
        if name == "progress":
            descriptor = args.progress_fd
        elif name == "control":
            descriptor = args.control_fd
        if descriptor is None:
            descriptor = _environment_fd(environment_name)
        if descriptor is None:
            continue
        if descriptor < 3:
            raise ProfileError(f"{name} fd must not overlap stdio")
        if descriptor in descriptors:
            raise ProfileError("launcher attachment fds must be distinct")
        descriptors.append(descriptor)
        names.append(name)
    metadata["fds"] = names
    encoded = json.dumps(metadata, separators=(",", ":")).encode("utf-8")
    if len(encoded) + 1 > MAX_LAUNCH_METADATA_BYTES:
        raise ProfileError("launcher metadata exceeds the size limit")
    return metadata, descriptors


def _read_socket_line(
    channel: socket.socket,
    *,
    limit: int,
) -> dict[str, object]:
    data = bytearray()
    while b"\n" not in data:
        if len(data) >= limit:
            raise ProfileError("agent launcher response exceeds the size limit")
        # Read one byte at a time so an immediately available ACP response
        # remains in the socket for the stdio proxy after the acknowledgement.
        chunk = channel.recv(1)
        if not chunk:
            raise ProfileError("agent launcher closed before acknowledging the launch")
        data.extend(chunk)
        if len(data) > limit:
            raise ProfileError("agent launcher response exceeds the size limit")
    line, remainder = bytes(data).split(b"\n", 1)
    if remainder:
        raise ProfileError("agent launcher sent data before the launch acknowledgement")
    try:
        value = json.loads(line)
    except json.JSONDecodeError as error:
        raise ProfileError("agent launcher acknowledgement is malformed") from error
    if not isinstance(value, dict):
        raise ProfileError("agent launcher acknowledgement is not an object")
    return value


def _connect_launcher(
    args: argparse.Namespace,
    *,
    profile: str,
    tool_policy: str,
) -> socket.socket:
    metadata, descriptors = _launch_metadata(
        args,
        profile=profile,
        tool_policy=tool_policy,
    )
    encoded = json.dumps(metadata, separators=(",", ":")).encode("utf-8") + b"\n"
    channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        channel.connect(str(_launcher_socket_path(args)))
        _check_server_uid(
            channel,
            environment_name="GC_AGENT_SERVER_UID",
            label="agent server",
        )
        if descriptors:
            sent = channel.sendmsg(
                [encoded],
                [
                    (
                        socket.SOL_SOCKET,
                        socket.SCM_RIGHTS,
                        array.array("i", descriptors).tobytes(),
                    )
                ],
            )
            if sent != len(encoded):
                raise ProfileError("agent launcher metadata was only partially sent")
        else:
            channel.sendall(encoded)
        for descriptor in descriptors:
            try:
                os.close(descriptor)
            except OSError:
                pass
        descriptors.clear()
        response = _read_socket_line(channel, limit=MAX_LAUNCH_RESPONSE_BYTES)
        if response.get("protocol") != LAUNCHER_PROTOCOL:
            raise ProfileError("agent launcher protocol version mismatch")
        if response.get("ok") is not True:
            error = response.get("error")
            raise ProfileError(
                str(error) if isinstance(error, str) and error else "agent launcher rejected the launch"
            )
        return channel
    except (OSError, ProfileError):
        for descriptor in descriptors:
            try:
                os.close(descriptor)
            except OSError:
                pass
        channel.close()
        raise


def _sandbox_session_message(message: bytes) -> bytes:
    try:
        value = json.loads(message)
    except json.JSONDecodeError:
        return message
    if not isinstance(value, dict) or value.get("method") != "session/new":
        return message
    params = value.get("params")
    if not isinstance(params, dict):
        return message
    rewritten = dict(value)
    rewritten_params = dict(params)
    rewritten_params["cwd"] = SANDBOX_WORKSPACE
    rewritten["params"] = rewritten_params
    return _frame(rewritten)


def _proxy_stdio(channel: socket.socket) -> int:
    """Proxy the caller's stdio to the authenticated launcher connection."""

    selector = select.poll()
    stdin_open = True
    socket_open = True
    stdin_buffer = bytearray()
    exit_code = 0
    channel_fd = channel.fileno()
    selector.register(0, select.POLLIN | select.POLLHUP | select.POLLERR)
    selector.register(channel_fd, select.POLLIN | select.POLLHUP | select.POLLERR)

    def consume_stdin(data: bytes) -> None:
        stdin_buffer.extend(data)
        while True:
            newline = stdin_buffer.find(b"\n")
            if newline < 0:
                return
            line = bytes(stdin_buffer[: newline + 1])
            del stdin_buffer[: newline + 1]
            channel.sendall(_sandbox_session_message(line))

    def close_stdin() -> None:
        nonlocal exit_code, stdin_open
        if not stdin_open:
            return
        if stdin_buffer:
            stdin_buffer.clear()
            print(
                "gascity-copilot-profile: rejected unterminated ACP frame "
                "at stdin EOF/HUP",
                file=sys.stderr,
            )
            exit_code = 1
        stdin_open = False
        try:
            channel.shutdown(socket.SHUT_WR)
        except OSError:
            pass
        selector.unregister(0)

    try:
        while socket_open:
            for descriptor, events in selector.poll(100):
                if descriptor == 0:
                    if not stdin_open:
                        continue
                    stdin_eof = False
                    if events & (select.POLLIN | select.POLLHUP):
                        while True:
                            try:
                                data = os.read(0, 64 * 1024)
                            except OSError as error:
                                if error.errno in {errno.EBADF, errno.EIO}:
                                    data = b""
                                else:
                                    raise
                            if not data:
                                stdin_eof = True
                                break
                            consume_stdin(data)
                            if not events & select.POLLHUP:
                                break
                    if stdin_eof or events & (
                        select.POLLHUP | select.POLLERR | select.POLLNVAL
                    ):
                        close_stdin()
                elif descriptor == channel_fd:
                    if events & (select.POLLIN | select.POLLHUP):
                        while True:
                            data = channel.recv(64 * 1024)
                            if not data:
                                socket_open = False
                                break
                            offset = 0
                            while offset < len(data):
                                offset += os.write(1, data[offset:])
                            if not events & select.POLLHUP:
                                break
                    if events & (select.POLLHUP | select.POLLERR):
                        socket_open = False
    except (BrokenPipeError, ConnectionError):
        return exit_code
    finally:
        channel.close()
    return exit_code


def _launcher_argv(
    args: argparse.Namespace,
    *,
    profile: str,
    settings: pathlib.Path,
    root: str | os.PathLike[str] | None,
    launcher: str,
    copilot: str,
    tool_policy: str,
) -> list[str]:
    metadata = _metadata(args)
    worktree = pathlib.Path(args.worktree or os.environ.get("GC_WORKTREE", os.getcwd()))
    lease_root = args.lease_root or os.environ.get("GC_LEASE_ROOT")
    runtime_root = args.runtime_root or os.environ.get("GC_RUNTIME_ROOT")
    sandbox_script = args.sandbox_script or str(
        pathlib.Path(__file__).resolve().with_name("agent-sandbox.py")
    )
    fdproxy_script = args.fdproxy_script or str(
        pathlib.Path(__file__).resolve().with_name("fdproxy.py")
    )
    if not lease_root or not runtime_root:
        raise ProfileError("lease and runtime roots are required")
    command = [
        sys.executable,
        launcher,
        "--profile",
        profile,
        "--tool-policy",
        tool_policy,
        "--settings",
        str(settings),
        "--copilot",
        copilot,
        "--run-id",
        metadata["run_id"],
        "--bead-id",
        metadata["bead_id"],
        "--generation",
        metadata["generation"],
        "--state-schema",
        metadata["state_schema"],
        "--worktree",
        str(worktree),
        "--lease-root",
        lease_root,
        "--runtime-root",
        runtime_root,
        "--sandbox-script",
        sandbox_script,
        "--fdproxy-script",
        fdproxy_script,
        "--proxy-port",
        str(args.proxy_port),
    ]
    if args.state_root or os.environ.get("GC_STATE_ROOT"):
        command.extend(["--state-root", args.state_root or os.environ["GC_STATE_ROOT"]])
    terminal_state_path = args.terminal_state_path
    terminal_state_root = os.environ.get("GC_TERMINAL_STATE_ROOT")
    if terminal_state_path is None and terminal_state_root:
        terminal_state_path = os.path.join(
            terminal_state_root,
            f"{metadata['run_id']}.json",
        )
    if terminal_state_path:
        command.extend(["--terminal-state-path", terminal_state_path])
    if args.readiness_status or os.environ.get("GC_READINESS_STATUS"):
        command.extend(
            [
                "--readiness-status",
                args.readiness_status or os.environ["GC_READINESS_STATUS"],
            ]
        )
    if args.require_ready or os.environ.get("GC_REQUIRE_READINESS") == "1":
        command.append("--require-ready")
    if args.fixture_direct:
        command.append("--allow-unsafe-fixture")
    if args.probe:
        command.append("--probe")
    runtime_paths = list(args.runtime_path)
    for variable in ("GC_RUNTIME_PATH", "GC_RUNTIME_PATHS"):
        configured = os.environ.get(variable)
        if configured:
            runtime_paths.extend(
                value for value in configured.split(os.pathsep) if value
            )
    for path in runtime_paths:
        command.extend(["--runtime-path", path])
    for path in args.approved_wrapper:
        command.extend(["--approved-wrapper", path])
    for option, value, environment_name in (
        ("--proxy-fd", args.proxy_fd, "GC_PROXY_FD"),
        ("--progress-fd", args.progress_fd, "GC_AGENT_FD"),
        ("--control-fd", args.control_fd, "GC_CONTROL_FD"),
    ):
        value = value if value is not None else _environment_fd(environment_name)
        if value is not None:
            command.extend([option, str(value)])
    if args.max_agents is not None:
        command.extend(["--max-agents", str(args.max_agents)])
    if args.max_active_runs is not None:
        command.extend(["--max-active-runs", str(args.max_active_runs)])
    if args.bwrap_path:
        command.extend(["--bwrap-path", args.bwrap_path])
    if args.sandbox_python:
        command.extend(["--sandbox-python", args.sandbox_python])
    child_arguments = child_argv(profile, tool_policy=tool_policy, root=root)
    if args.fixture_direct:
        child_arguments.append(f"--fixture-direct-cwd={worktree}")
    command.extend(["--", *child_arguments])
    return command


def build_launch_argv(
    profile: str,
    *,
    tool_policy: str,
    root: str | os.PathLike[str] | None = None,
    launcher: str | None = None,
    copilot: str | None = None,
    args: argparse.Namespace | None = None,
) -> list[str]:
    launch_args = args or _default_namespace(profile, tool_policy)
    if not launch_args.fixture_direct or os.environ.get("GC_TEST_MODE") != "1":
        raise ProfileError(
            "local launcher construction is restricted to explicit GC_TEST_MODE fixtures"
        )
    effective_profile = _effective_profile(
        profile,
        tool_policy=tool_policy,
        args=launch_args,
    )
    load_profile(effective_profile, root)
    settings = settings_path(effective_profile, root)
    launcher_path = _configured_path(
        launcher,
        environment_name="GC_AGENT_LAUNCHER",
        fallback=str(pathlib.Path(__file__).resolve().with_name("agent-launcher.py")),
        label="agent launcher",
    )
    copilot_path = _configured_path(
        copilot,
        environment_name="GC_COPILOT_BIN",
        fallback=shutil.which("copilot"),
        label="Copilot executable",
    )
    return _launcher_argv(
        launch_args,
        profile=effective_profile,
        settings=settings,
        launcher=launcher_path,
        copilot=copilot_path,
        tool_policy=tool_policy,
        root=root,
    )


def _default_namespace(profile: str, tool_policy: str) -> argparse.Namespace:
    return argparse.Namespace(
        profile=profile,
        tool_policy=tool_policy,
        run_id=None,
        bead_id=None,
        generation=None,
        state_schema=None,
        worktree=None,
        state_root=None,
        terminal_state_path=None,
        lease_root=None,
        runtime_root=None,
        sandbox_script=None,
        fdproxy_script=None,
        proxy_port=3128,
        launcher_socket=None,
        proxy_fd=None,
        progress_fd=None,
        control_fd=None,
        readiness_status=None,
        require_ready=False,
        probe=False,
        fixture_direct=False,
        runtime_path=[],
        approved_wrapper=[],
        max_agents=None,
        max_active_runs=None,
        bwrap_path=None,
        sandbox_python=None,
    )


def _frame(value: Mapping[str, object]) -> bytes:
    """Encode one ACP JSON-RPC message as newline-delimited JSON."""

    encoded = json.dumps(value, separators=(",", ":")).encode("utf-8")
    if b"\n" in encoded or b"\r" in encoded:
        raise ProfileError("ACP message contains an unescaped line break")
    return encoded + b"\n"


class ACPClosed(ProfileError):
    """Raised when the ACP server closes stdout before a response arrives."""


class _ACPReader:
    def __init__(self, fd: int):
        self.fd = fd
        self.buffer = bytearray()

    def read(self, timeout: float) -> dict[str, object]:
        deadline = time.monotonic() + timeout
        while True:
            newline = self.buffer.find(b"\n")
            if newline >= 0:
                line = bytes(self.buffer[:newline])
                del self.buffer[: newline + 1]
                if not line:
                    raise ProfileError("ACP response contains an empty line")
                if line.endswith(b"\r"):
                    line = line[:-1]
                try:
                    value = json.loads(line)
                except json.JSONDecodeError as error:
                    raise ProfileError("ACP response JSON is malformed") from error
                if not isinstance(value, dict):
                    raise ProfileError("ACP response is not an object")
                return value
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ProfileError("ACP probe timed out")
            ready, _writes, _errors = select.select([self.fd], [], [], remaining)
            if not ready:
                raise ProfileError("ACP probe timed out")
            data = os.read(self.fd, 64 * 1024)
            if not data:
                raise ACPClosed("ACP process closed its stdout")
            self.buffer.extend(data)
            if len(self.buffer) > 64 * 1024:
                raise ProfileError("ACP response line exceeds the size limit")


def _response_for(
    reader: _ACPReader,
    request_id: int,
    timeout: float,
    *,
    observations: list[object],
) -> dict[str, object]:
    while True:
        response = reader.read(timeout)
        observations.append(response)
        if "id" not in response:
            if "result" in response or "error" in response:
                raise ProfileError("ACP response is malformed: missing response id")
            continue
        response_id = response["id"]
        if type(response_id) is not int:
            raise ProfileError("ACP response is malformed: invalid response id")
        if response_id != request_id:
            raise ProfileError("ACP response is malformed: unexpected response id")
        if "error" in response:
            raise ProfileError(f"ACP request failed: {response['error']}")
        if not isinstance(response.get("result"), dict):
            raise ProfileError("ACP response is malformed: result is not an object")
        return response


def _find_values(value: object, keys: Sequence[str]) -> list[str]:
    found: list[str] = []
    if isinstance(value, dict):
        for key in keys:
            candidate = value.get(key)
            if isinstance(candidate, str) and candidate:
                found.append(candidate)
        for nested in value.values():
            found.extend(_find_values(nested, keys))
    elif isinstance(value, list):
        for nested in value:
            found.extend(_find_values(nested, keys))
    return found


def _find_value(value: object, keys: Sequence[str]) -> str | None:
    values = _find_values(value, keys)
    return values[0] if values else None


def _active_model_observations(value: object) -> tuple[bool, bool, list[str]]:
    present = False
    invalid = False
    values: list[str] = []
    if isinstance(value, dict):
        for key in ACTIVE_MODEL_KEYS:
            if key not in value:
                continue
            present = True
            candidate = value[key]
            if isinstance(candidate, str) and candidate:
                values.append(candidate)
            else:
                invalid = True
        for nested in value.values():
            nested_present, nested_invalid, nested_values = _active_model_observations(
                nested
            )
            present = present or nested_present
            invalid = invalid or nested_invalid
            values.extend(nested_values)
    elif isinstance(value, list):
        for nested in value:
            nested_present, nested_invalid, nested_values = _active_model_observations(
                nested
            )
            present = present or nested_present
            invalid = invalid or nested_invalid
            values.extend(nested_values)
    return present, invalid, values


def _validated_probe_responses(
    response_values: Sequence[object],
) -> dict[int, dict[str, object]] | None:
    responses: dict[int, dict[str, object]] = {}
    expected_ids = {1, 2, 3}
    for value in response_values:
        if not isinstance(value, dict):
            return None
        if "id" not in value:
            if "result" in value or "error" in value:
                return None
            continue
        response_id = value["id"]
        if type(response_id) is not int or response_id not in expected_ids:
            return None
        if response_id in responses:
            return None
        responses[response_id] = value
    if set(responses) != expected_ids:
        return None
    for response in responses.values():
        if "error" in response or not isinstance(response.get("result"), dict):
            return None
    session_result = responses[2]["result"]
    if not _find_value(session_result, ("sessionId", "session_id")):
        return None
    return responses


def _probe_result(profile: str, response_values: Sequence[object]) -> dict[str, object]:
    expected = PROFILE_SETTINGS[profile]
    reported_models: list[str] = []
    reported_contexts: list[str] = []
    active_model_present = False
    active_model_invalid = False
    for value in response_values:
        present, invalid, models = _active_model_observations(value)
        active_model_present = active_model_present or present
        active_model_invalid = active_model_invalid or invalid
        reported_models.extend(models)
        reported_contexts.extend(
            _find_values(value, ("contextTier", "context_tier"))
        )
    reported_model = reported_models[0] if reported_models else None
    reported_context = reported_contexts[0] if reported_contexts else None
    if (
        _validated_probe_responses(response_values) is None
        or active_model_invalid
        or (active_model_present and set(reported_models) != {expected["model"]})
    ):
        return {
            "ok": False,
            "profile": profile,
            "error_code": "malformed",
            "reported_model": reported_model,
            "reported_context": reported_context,
        }
    return {
        "ok": True,
        "profile": profile,
        "model": expected["model"],
        "context": expected["contextTier"],
        "effort": PROFILE_EFFORT[profile],
    }


def _session_cwd(args: argparse.Namespace) -> str:
    if args.fixture_direct:
        return args.worktree or os.getcwd()
    return SANDBOX_WORKSPACE


def _probe_error_code(text: str) -> str:
    lowered = text.lower()
    if any(marker in lowered for marker in ("closed", "eof", "end of file")):
        return "closed"
    if any(marker in lowered for marker in ("authentication", "unauthorized", "invalid token", "401", "403")):
        return "authentication"
    if any(marker in lowered for marker in ("network", "connection", "timeout", "dns", "proxy", "tls")):
        return "network"
    if any(marker in lowered for marker in ("quota", "rate limit", "429")):
        return "quota"
    if any(marker in lowered for marker in ("malformed", "invalid json", "protocol", "parse")):
        return "malformed"
    if any(marker in lowered for marker in ("unsupported", "not supported", "unknown model", "model not found")):
        return "unsupported"
    if "unavailable" in lowered:
        return "unavailable"
    return "unknown"


def _probe_exchange(
    reader: _ACPReader,
    send_message: Callable[[bytes], None],
    *,
    profile: str,
    args: argparse.Namespace,
    timeout: float,
) -> dict[str, object]:
    values: list[object] = []
    initialize = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {
                "name": "d2b-gascity-preflight",
                "version": "1",
            },
        },
    }
    send_message(_frame(initialize))
    _response_for(reader, 1, timeout, observations=values)
    new_session = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {
            "cwd": _session_cwd(args),
            "mcpServers": [],
        },
    }
    send_message(_frame(new_session))
    session_response = _response_for(reader, 2, timeout, observations=values)
    session_result = session_response["result"]
    session_id = _find_value(session_result, ("sessionId", "session_id"))
    if not session_id:
        raise ProfileError("ACP session/new response is malformed: no session id")
    prompt = {
        "jsonrpc": "2.0",
        "id": 3,
        "method": "session/prompt",
        "params": {
            "sessionId": session_id,
            "prompt": [
                {
                    "type": "text",
                    "text": "Preflight diagnostic. Report the active model and context tier, then stop.",
                }
            ],
        },
    }
    send_message(_frame(prompt))
    _response_for(reader, 3, timeout, observations=values)
    return _probe_result(profile, values)


def _drain_channel(channel: socket.socket, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ProfileError("agent launcher did not close after probe EOF")
        ready, _writes, _errors = select.select([channel], [], [], remaining)
        if not ready:
            raise ProfileError("agent launcher did not close after probe EOF")
        if not channel.recv(64 * 1024):
            return


def _run_socket_probe(
    profile: str,
    *,
    tool_policy: str,
    args: argparse.Namespace,
    timeout: float,
) -> dict[str, object]:
    try:
        channel = _connect_launcher(args, profile=profile, tool_policy=tool_policy)
    except (OSError, ProfileError) as error:
        text = str(error)
        return {
            "ok": False,
            "profile": profile,
            "error_code": _probe_error_code(text),
            "error": text,
        }
    try:
        result = _probe_exchange(
            _ACPReader(channel.fileno()),
            channel.sendall,
            profile=profile,
            args=args,
            timeout=timeout,
        )
        channel.shutdown(socket.SHUT_WR)
        _drain_channel(channel, timeout)
        return result
    except (OSError, ProfileError) as error:
        text = str(error)
        try:
            channel.shutdown(socket.SHUT_WR)
        except OSError:
            pass
        return {
            "ok": False,
            "profile": profile,
            "error_code": _probe_error_code(text),
            "error": text,
        }
    finally:
        channel.close()


def _run_direct_probe(
    profile: str,
    *,
    tool_policy: str,
    args: argparse.Namespace,
    timeout: float,
) -> dict[str, object]:
    if os.environ.get("GC_TEST_MODE") != "1":
        raise ProfileError("direct ACP probe mode requires GC_TEST_MODE=1")
    command = build_launch_argv(
        profile,
        tool_policy=tool_policy,
        launcher=args.launcher,
        copilot=args.copilot,
        args=args,
    )
    environment = scrub_environment()
    environment["GC_TEST_MODE"] = "1"
    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=args.worktree or os.getcwd(),
        env=environment,
        start_new_session=True,
    )
    if process.stdin is None or process.stdout is None or process.stderr is None:
        process.kill()
        process.wait()
        raise ProfileError("probe process pipes were not created")
    stderr_buffer = bytearray()

    def drain_stderr() -> None:
        while True:
            data = process.stderr.read(64 * 1024)
            if not data:
                return
            stderr_buffer.extend(data)
            if len(stderr_buffer) > 8192:
                del stderr_buffer[:-8192]

    stderr_thread = threading.Thread(target=drain_stderr, name="acp-probe-stderr")
    stderr_thread.start()
    try:
        result = _probe_exchange(
            _ACPReader(process.stdout.fileno()),
            lambda message: (process.stdin.write(message), process.stdin.flush()),
            profile=profile,
            args=args,
            timeout=timeout,
        )
        process.stdin.close()
        process.wait(timeout=timeout)
        if process.returncode != 0:
            detail = bytes(stderr_buffer).decode("utf-8", errors="replace")
            return {
                "ok": False,
                "profile": profile,
                "error_code": _probe_error_code(detail),
                "error": detail[-2048:],
            }
        return result
    except (OSError, ProfileError) as error:
        try:
            process.stdin.close()
        except OSError:
            pass
        try:
            process.wait(timeout=1.0)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        text = f"{error}\n{bytes(stderr_buffer).decode('utf-8', errors='replace')}".strip()
        return {
            "ok": False,
            "profile": profile,
            "error_code": _probe_error_code(text),
            "error": text,
        }
    finally:
        stderr_thread.join(timeout=1.0)
        process.stdout.close()
        process.stderr.close()


def run_probe(
    profile: str,
    *,
    tool_policy: str,
    args: argparse.Namespace,
    timeout: float = 15.0,
) -> dict[str, object]:
    if args.fixture_direct:
        return _run_direct_probe(
            profile,
            tool_policy=tool_policy,
            args=args,
            timeout=timeout,
        )
    return _run_socket_probe(
        profile,
        tool_policy=tool_policy,
        args=args,
        timeout=timeout,
    )


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", required=True, choices=sorted(PROFILE_NAMES))
    parser.add_argument("--tool-policy", choices=sorted(TOOL_POLICIES), default="review")
    parser.add_argument("--launcher")
    parser.add_argument("--copilot")
    parser.add_argument("--run-id")
    parser.add_argument("--bead-id")
    parser.add_argument("--generation")
    parser.add_argument("--state-schema")
    parser.add_argument("--worktree")
    parser.add_argument("--state-root")
    parser.add_argument("--terminal-state-path")
    parser.add_argument("--launcher-socket")
    parser.add_argument("--lease-root")
    parser.add_argument("--runtime-root")
    parser.add_argument("--sandbox-script")
    parser.add_argument("--fdproxy-script")
    parser.add_argument("--readiness-status")
    parser.add_argument("--require-ready", action="store_true")
    parser.add_argument(
        "--fixture-direct",
        "--allow-unsafe-fixture",
        dest="fixture_direct",
        action="store_true",
    )
    parser.add_argument("--runtime-path", action="append", default=[])
    parser.add_argument("--approved-wrapper", action="append", default=[])
    parser.add_argument("--proxy-fd", type=int)
    parser.add_argument("--progress-fd", type=int)
    parser.add_argument("--control-fd", type=int)
    parser.add_argument("--max-agents", type=int)
    parser.add_argument("--max-active-runs", type=int)
    parser.add_argument("--bwrap-path")
    parser.add_argument("--sandbox-python")
    parser.add_argument("--proxy-port", type=int, default=3128)
    parser.add_argument("--probe", action="store_true")
    parser.add_argument("--describe", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    settings = load_profile(args.profile)
    if args.describe:
        json.dump(
            {
                "profile": args.profile,
                "settings": settings,
                "child_argv": child_argv(args.profile, tool_policy=args.tool_policy),
            },
            sys.stdout,
            separators=(",", ":"),
        )
        sys.stdout.write("\n")
        return 0
    if args.probe:
        result = run_probe(
            args.profile,
            tool_policy=args.tool_policy,
            args=args,
        )
        json.dump(result, sys.stdout, separators=(",", ":"))
        sys.stdout.write("\n")
        return 0 if result.get("ok") is True else 1

    if args.fixture_direct:
        command = build_launch_argv(
            args.profile,
            tool_policy=args.tool_policy,
            launcher=args.launcher,
            copilot=args.copilot,
            args=args,
        )
        environment = scrub_environment()
        environment["GC_TEST_MODE"] = "1"
        os.execve(command[0], command, environment)
        return 0
    effective_profile = _effective_profile(
        args.profile,
        tool_policy=args.tool_policy,
        args=args,
    )
    channel = _connect_launcher(
        args,
        profile=effective_profile,
        tool_policy=args.tool_policy,
    )
    return _proxy_stdio(channel)


if __name__ == "__main__":
    raise SystemExit(main())
