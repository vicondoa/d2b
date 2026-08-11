#!/usr/bin/env python3
"""Build the bubblewrap command used for one Gas City ACP child.

The launcher owns process lifetime.  This module owns only the namespace
construction, which keeps the mount and fd policy inspectable without
starting a sandbox.  The default root is an empty tmpfs; every path visible
to the child is therefore an explicit mount.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import sys
from collections.abc import Mapping, Sequence


class SandboxError(RuntimeError):
    """Raised when a requested sandbox projection is unsafe or incomplete."""


HIDDEN_ENV_NAMES = frozenset(
    {
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "BUILD_BUDDY_API_KEY",
        "DISCORD_TOKEN",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "SSH_AUTH_SOCK",
    }
)
TOOL_POLICIES = frozenset({"review", "planning", "coding"})
PLANNING_ARTIFACT_ROOTS = ("docs/plans",)
ALLOWED_GC_ENV_NAMES = frozenset(
    {
        "GC_AGENT_FD",
        "GC_BEAD_ID",
        "GC_CITY_GENERATION",
        "GC_FDPROXY_FD",
        "GC_FDPROXY_AUTH",
        "GC_PROFILE_NAME",
        "GC_RUN_ID",
        "GC_STATE_SCHEMA",
    }
)


def _absolute(path: str | os.PathLike[str], label: str) -> pathlib.Path:
    value = pathlib.Path(path)
    if not value.is_absolute():
        raise SandboxError(f"{label} must be absolute: {value}")
    if any(part == ".." for part in value.parts):
        raise SandboxError(f"{label} must not contain '..': {value}")
    return value


def _existing(path: str | os.PathLike[str], label: str) -> pathlib.Path:
    value = _absolute(path, label)
    if not value.exists():
        raise SandboxError(f"{label} does not exist: {value}")
    return value.resolve()


def _validate_worktree(worktree: str | os.PathLike[str], state_root: str | None) -> pathlib.Path:
    assigned = _existing(worktree, "assigned worktree")
    if not assigned.is_dir():
        raise SandboxError(f"assigned worktree is not a directory: {assigned}")
    if assigned == pathlib.Path("/"):
        raise SandboxError("the root directory cannot be an assigned worktree")
    if state_root is not None:
        state = _existing(state_root, "state root")
        if assigned.is_relative_to(state) or state.is_relative_to(assigned):
            raise SandboxError("state root and assigned worktree must be disjoint")
    return assigned


def _planning_artifact_roots(worktree: pathlib.Path) -> list[tuple[pathlib.Path, str]]:
    roots: list[tuple[pathlib.Path, str]] = []
    for relative in PLANNING_ARTIFACT_ROOTS:
        candidate = worktree / relative
        if not candidate.exists():
            raise SandboxError(
                f"planning artifact root does not exist: {candidate}"
            )
        resolved = candidate.resolve()
        try:
            resolved.relative_to(worktree)
        except ValueError as error:
            raise SandboxError(
                f"planning artifact root escapes the assigned worktree: {candidate}"
            ) from error
        if not resolved.is_dir():
            raise SandboxError(f"planning artifact root is not a directory: {candidate}")
        roots.append((resolved, f"/workspace/{relative}"))
    return roots


def _add_parent_dirs(arguments: list[str], destination: str, known: set[str]) -> None:
    parent = pathlib.PurePosixPath(destination).parent
    parents: list[str] = []
    while str(parent) not in {"", "/"}:
        parents.append(str(parent))
        parent = parent.parent
    for directory in reversed(parents):
        if directory not in known:
            arguments.extend(["--dir", directory])
            known.add(directory)


def _bind_read_only(
    arguments: list[str],
    source: pathlib.Path,
    destination: str,
    known_dirs: set[str],
) -> None:
    _add_parent_dirs(arguments, destination, known_dirs)
    arguments.extend(["--ro-bind", str(source), destination])


def _bind_writable(
    arguments: list[str],
    source: pathlib.Path,
    destination: str,
    known_dirs: set[str],
) -> None:
    _add_parent_dirs(arguments, destination, known_dirs)
    arguments.extend(["--bind", str(source), destination])


def _runtime_paths(
    command: Sequence[str],
    runtime_paths: Sequence[str | os.PathLike[str]],
    approved_wrappers: Sequence[str | os.PathLike[str]],
) -> list[pathlib.Path]:
    candidates: list[str | os.PathLike[str]] = list(runtime_paths)
    candidates.extend(approved_wrappers)
    if command and os.path.isabs(command[0]):
        candidates.append(command[0])
    result: list[pathlib.Path] = []
    seen: set[pathlib.Path] = set()
    for candidate in candidates:
        resolved = _existing(candidate, "approved runtime path")
        if resolved not in seen:
            result.append(resolved)
            seen.add(resolved)
    return result


def _safe_environment(environment: Mapping[str, str]) -> dict[str, str]:
    projected: dict[str, str] = {}
    for name, value in environment.items():
        if name in HIDDEN_ENV_NAMES:
            continue
        if name == "COPILOT_GITHUB_TOKEN":
            projected[name] = value
            continue
        if name in ALLOWED_GC_ENV_NAMES or name in {
            "ALL_PROXY",
            "HOME",
            "HTTPS_PROXY",
            "HTTP_PROXY",
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
        }:
            projected[name] = value
    return projected


def build_sandbox_argv(
    command: Sequence[str],
    *,
    worktree: str | os.PathLike[str],
    tool_policy: str = "coding",
    state_root: str | os.PathLike[str] | None = None,
    copilot_home: str | os.PathLike[str] | None = None,
    runtime_paths: Sequence[str | os.PathLike[str]] = (),
    approved_wrappers: Sequence[str | os.PathLike[str]] = (),
    environment: Mapping[str, str] | None = None,
    proxy_fd: int | None = None,
    progress_fd: int | None = None,
    fdproxy_path: str | os.PathLike[str] | None = None,
    python_path: str | os.PathLike[str] | None = None,
    bwrap_path: str | os.PathLike[str] | None = None,
    proxy_port: int = 3128,
) -> tuple[list[str], tuple[int, ...]]:
    """Return ``(bwrap_argv, inherited_fds)`` for one child.

    ``environment`` is intentionally not encoded into argv.  In particular,
    the Copilot token is inherited as an environment value and is never
    placed in a bwrap argument.  The launcher supplies the already-scrubbed
    environment and marks the child non-dumpable.
    """

    if not command:
        raise SandboxError("sandbox command must not be empty")
    if tool_policy not in TOOL_POLICIES:
        raise SandboxError(f"unknown sandbox tool policy: {tool_policy}")
    assigned = _validate_worktree(worktree, os.fspath(state_root) if state_root else None)
    home = None
    if copilot_home is not None:
        home = _existing(copilot_home, "Copilot home")
        if not home.is_dir():
            raise SandboxError(f"Copilot home is not a directory: {home}")

    bwrap = _existing(bwrap_path, "bubblewrap") if bwrap_path else None
    bwrap_executable = str(bwrap or shutil.which("bwrap") or "")
    if not bwrap_executable:
        raise SandboxError("bubblewrap is not available")

    python = _existing(python_path, "sandbox Python") if python_path else None
    python_executable = str(python or sys.executable)
    if not os.path.isabs(python_executable):
        raise SandboxError("sandbox Python must resolve to an absolute path")

    runtime = _runtime_paths(command, runtime_paths, approved_wrappers)
    runtime.extend(
        path
        for path in (
            pathlib.Path(python_executable).resolve(),
            _existing(fdproxy_path, "fdproxy") if fdproxy_path else None,
        )
        if path is not None and path not in runtime
    )

    arguments: list[str] = [
        bwrap_executable,
        "--die-with-parent",
        "--new-session",
        "--as-pid-1",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--cap-drop",
        "ALL",
        "--tmpfs",
        "/",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/run",
        "--tmpfs",
        "/home",
    ]
    known_dirs = {"/proc", "/dev", "/tmp", "/run", "/home"}

    for path in runtime:
        _bind_read_only(arguments, path, str(path), known_dirs)

    arguments.extend(["--dir", "/workspace"])
    known_dirs.add("/workspace")
    if tool_policy == "coding":
        _bind_writable(arguments, assigned, "/workspace", known_dirs)
    else:
        _bind_read_only(arguments, assigned, "/workspace", known_dirs)
        if tool_policy == "planning":
            for source, destination in _planning_artifact_roots(assigned):
                _bind_writable(arguments, source, destination, known_dirs)
    if home is not None:
        arguments.extend(["--dir", "/home/copilot"])
        known_dirs.add("/home/copilot")
        _bind_writable(arguments, home, "/home/copilot", known_dirs)
    else:
        arguments.extend(["--dir", "/home/copilot"])
        known_dirs.add("/home/copilot")

    # Common host state and service-control locations do not reappear through
    # an inherited root mount.
    for hidden in ("/var/lib", "/var/run", "/etc/gascity", "/srv", "/opt"):
        _add_parent_dirs(arguments, hidden, known_dirs)
        arguments.extend(["--tmpfs", hidden])

    projected = _safe_environment(environment or {})
    if proxy_fd is None:
        for proxy_name in ("ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"):
            projected.pop(proxy_name, None)
        projected.pop("GC_FDPROXY_FD", None)
    if progress_fd is None:
        projected.pop("GC_AGENT_FD", None)
    projected.update(
        {
            "HOME": "/home/copilot",
            "COPILOT_HOME": "/home/copilot",
            "PWD": "/workspace",
            "TMPDIR": "/tmp",
            "XDG_RUNTIME_DIR": "/run",
            "PATH": projected.get("PATH", "/wrappers:/runtime/bin"),
        }
    )
    for name, value in sorted(projected.items()):
        if name in {"COPILOT_GITHUB_TOKEN", "GC_FDPROXY_AUTH"}:
            # Secrets remain in the inherited, scrubbed environment.  Adding
            # them with --setenv would expose them in process arguments.
            continue
        arguments.extend(["--setenv", name, value])
    for name in sorted(HIDDEN_ENV_NAMES):
        arguments.extend(["--unsetenv", name])

    if proxy_fd is not None:
        if proxy_fd < 3:
            raise SandboxError("proxy fd must not overlap stdio")
        if progress_fd == proxy_fd:
            raise SandboxError("proxy and progress fds must be distinct")
        if fdproxy_path is None:
            raise SandboxError("fdproxy path is required when proxy fd is supplied")
        arguments.extend(
            [
                "--setenv",
                "GC_FDPROXY_FD",
                str(proxy_fd),
                "--setenv",
                "HTTP_PROXY",
                f"http://127.0.0.1:{proxy_port}",
                "--setenv",
                "HTTPS_PROXY",
                f"http://127.0.0.1:{proxy_port}",
                "--setenv",
                "ALL_PROXY",
                f"http://127.0.0.1:{proxy_port}",
                "--setenv",
                "NO_PROXY",
                "127.0.0.1,localhost",
            ]
        )

    if progress_fd is not None:
        if progress_fd < 3:
            raise SandboxError("progress fd must not overlap stdio")
        arguments.extend(["--setenv", "GC_AGENT_FD", str(progress_fd)])

    inner_command = list(command)
    if proxy_fd is not None:
        inner_command = [
            python_executable,
            str(_existing(fdproxy_path, "fdproxy")),
            "--channel-fd",
            str(proxy_fd),
            "--listen",
            f"127.0.0.1:{proxy_port}",
            *(
                ["--progress-fd", str(progress_fd)]
                if progress_fd is not None
                else []
            ),
            "--",
            *inner_command,
        ]

    arguments.extend(["--chdir", "/workspace", "--", *inner_command])
    inherited = tuple(
        sorted(
            fd
            for fd in (proxy_fd, progress_fd)
            if fd is not None
        )
    )
    return arguments, inherited


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worktree", required=True)
    parser.add_argument("--tool-policy", default="coding")
    parser.add_argument("--state-root")
    parser.add_argument("--copilot-home")
    parser.add_argument("--runtime-path", action="append", default=[])
    parser.add_argument("--approved-wrapper", action="append", default=[])
    parser.add_argument("--proxy-fd", type=int)
    parser.add_argument("--progress-fd", type=int)
    parser.add_argument("--fdproxy-path")
    parser.add_argument("--python-path")
    parser.add_argument("--bwrap-path")
    parser.add_argument("--proxy-port", type=int, default=3128)
    parser.add_argument("--print-argv", action="store_true")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    command = list(args.command)
    if command[:1] == ["--"]:
        command = command[1:]
    argv, inherited = build_sandbox_argv(
        command,
        worktree=args.worktree,
        tool_policy=args.tool_policy,
        state_root=args.state_root,
        copilot_home=args.copilot_home,
        runtime_paths=args.runtime_path,
        approved_wrappers=args.approved_wrapper,
        environment=dict(os.environ),
        proxy_fd=args.proxy_fd,
        progress_fd=args.progress_fd,
        fdproxy_path=args.fdproxy_path,
        python_path=args.python_path,
        bwrap_path=args.bwrap_path,
        proxy_port=args.proxy_port,
    )
    if args.print_argv:
        json.dump({"argv": argv, "inherited_fds": inherited}, sys.stdout)
        sys.stdout.write("\n")
        return 0
    projected = _safe_environment(dict(os.environ))
    projected.update(
        {
            "HOME": "/home/copilot",
            "COPILOT_HOME": "/home/copilot",
            "TMPDIR": "/tmp",
            "XDG_RUNTIME_DIR": "/run",
        }
    )
    os.execve(argv[0], argv, projected)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
