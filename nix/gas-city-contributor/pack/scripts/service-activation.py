#!/usr/bin/env python3
"""Preflight, readiness, durable continuation, and active-run GC roots."""

from __future__ import annotations

import argparse
import array
import errno
import fcntl
import grp
import ipaddress
import json
import os
import pathlib
import re
import selectors
import socket
import stat
import struct
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass


PROFILE_SETTINGS = {
    "review-sol": {
        "model": "gpt-5.6-sol",
        "effort": "xhigh",
        "context": "long_context",
    },
    "review-luna": {
        "model": "gpt-5.6-luna",
        "effort": "max",
        "context": "long_context",
    },
    "code-luna": {
        "model": "gpt-5.6-luna",
        "effort": "max",
        "context": "default",
    },
}
FALLBACK_FAILURE_CODES = frozenset({"unsupported", "unavailable"})
FAILURE_CODES = frozenset(
    {
        "authentication",
        "network",
        "quota",
        "malformed",
        "unsupported",
        "unavailable",
        "closed",
        "unknown",
    }
)
STATUS_KEYS = frozenset(
    {
        "generation",
        "state_schema",
        "ready",
        "effective_profiles",
        "error_code",
    }
)
CONTEXT_REQUIRED_KEYS = frozenset(
    {
        "run_id",
        "bead_id",
        "generation",
        "state_schema",
        "open_work",
        "summary",
        "branch",
        "commits",
        "worktree",
        "review_state",
        "retry_counters",
        "next_action",
    }
)
GC_ROOT_NAMES = (
    "package",
    "city",
    "pack",
    "profiles",
    "instructions",
)
MANAGED_ASSET_NAMES = ("city", "pack", "copilot", "buildbuddy")
NIX_STORE_ROOT = pathlib.Path("/nix/store")
NIX_STORE_OBJECT_PATTERN = re.compile(
    r"^[0123456789abcdfghijklmnpqrsvwxyz]{32}-(?!\.\.?$)[A-Za-z0-9+._?=-]+$"
)
IDENTIFIER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")


class ActivationError(RuntimeError):
    """Raised for a closed, actionable activation or continuation failure."""


class StaleGeneration(ActivationError):
    """Raised when state or readiness belongs to a different generation."""


class RootLifecycleError(ActivationError):
    """Raised when active-run GC-root cleanup violates lifecycle ownership."""


class BoundaryError(ActivationError):
    """Raised when an activation or egress boundary is unsafe."""


FDPROXY_PROTOCOL = "fdproxy/1"
DEFAULT_ALLOWED_PORT = 443
PRIVATE_NETWORKS = (
    ipaddress.ip_network("0.0.0.0/8"),
    ipaddress.ip_network("10.0.0.0/8"),
    ipaddress.ip_network("100.64.0.0/10"),
    ipaddress.ip_network("127.0.0.0/8"),
    ipaddress.ip_network("169.254.0.0/16"),
    ipaddress.ip_network("172.16.0.0/12"),
    ipaddress.ip_network("192.0.0.0/24"),
    ipaddress.ip_network("192.0.2.0/24"),
    ipaddress.ip_network("192.168.0.0/16"),
    ipaddress.ip_network("198.18.0.0/15"),
    ipaddress.ip_network("198.51.100.0/24"),
    ipaddress.ip_network("203.0.113.0/24"),
    ipaddress.ip_network("224.0.0.0/4"),
    ipaddress.ip_network("240.0.0.0/4"),
    ipaddress.ip_network("::/128"),
    ipaddress.ip_network("::1/128"),
    ipaddress.ip_network("fc00::/7"),
    ipaddress.ip_network("fe80::/10"),
    ipaddress.ip_network("ff00::/8"),
    ipaddress.ip_network("2001:db8::/32"),
)
FORBIDDEN_PROJECTION_ROOTS = tuple(
    pathlib.Path(value)
    for value in (
        "/root",
        "/etc/shadow",
        "/etc/gshadow",
        "/etc/ssh",
        "/etc/nixos",
        "/tmp",
        "/proc",
        "/sys",
        "/dev",
        "/var/cache",
        "/var/run",
        "/var/lib",
        "/run",
    )
)
NIXOS_PROJECTION_ROOT = pathlib.Path("/etc/nixos")
AGENT_RELAY_CHUNK_BYTES = 64 * 1024
AGENT_RELAY_MAX_ATTACHMENTS = 4
AGENT_RELAY_METADATA_BYTES = 16 * 1024
AGENT_RELAY_RESPONSE_BYTES = 8 * 1024
GC_ROOT_METADATA_SCHEMA = 1
TERMINAL_RECORD_SCHEMA = 1
TERMINAL_RECORD_KEYS = frozenset(
    {
        "schema",
        "run_id",
        "bead_id",
        "generation",
        "state_schema",
        "terminal_status",
    }
)
TERMINAL_WORKFLOW_STATES = frozenset(
    {"closed", "complete", "completed", "failed", "succeeded", "terminal"}
)
MAX_TERMINAL_RECORD_BYTES = 16 * 1024
MAX_BEAD_STATE_BYTES = 1024 * 1024


@dataclass(frozen=True)
class ProbeResult:
    profile: str
    ok: bool
    model: str | None = None
    context: str | None = None
    effort: str | None = None
    error_code: str | None = None
    error: str | None = None


def _absolute_normalized_path(value: str, label: str) -> pathlib.Path:
    if not value or "\x00" in value:
        raise BoundaryError(f"{label} is empty or contains NUL")
    path = pathlib.Path(value)
    if not path.is_absolute() or any(part == ".." for part in path.parts):
        raise BoundaryError(f"{label} must be absolute and normalized")
    if os.path.normpath(value) != value:
        raise BoundaryError(f"{label} is not canonical")
    return path


def _check_ancestor_chain(path: pathlib.Path, label: str) -> None:
    """Reject symlinked or writable ancestors before following a path."""

    ancestors = list(path.parents)
    for ancestor in reversed(ancestors):
        try:
            info = os.lstat(ancestor)
        except OSError as error:
            raise BoundaryError(f"{label} ancestor is unavailable: {ancestor}") from error
        if stat.S_ISLNK(info.st_mode):
            raise BoundaryError(f"{label} has a symlinked ancestor: {ancestor}")
        if not stat.S_ISDIR(info.st_mode):
            raise BoundaryError(f"{label} ancestor is not a directory: {ancestor}")
        if info.st_mode & 0o022:
            raise BoundaryError(f"{label} ancestor is writable by group or other")


def _check_store_ancestor_chain(path: pathlib.Path, label: str) -> None:
    """Reject foreign ancestors while allowing the Nix store's standard mode."""

    for ancestor in reversed(path.parents):
        try:
            info = os.lstat(ancestor)
        except OSError as error:
            raise BoundaryError(f"{label} ancestor is unavailable: {ancestor}") from error
        if stat.S_ISLNK(info.st_mode):
            raise BoundaryError(f"{label} has a symlinked ancestor: {ancestor}")
        if not stat.S_ISDIR(info.st_mode):
            raise BoundaryError(f"{label} ancestor is not a directory: {ancestor}")
        if ancestor != NIX_STORE_ROOT and info.st_mode & 0o022:
            raise BoundaryError(f"{label} ancestor is writable by group or other")


def _validate_store_object_component(path: pathlib.Path, label: str) -> None:
    """Require the first component below /nix/store to have Nix's store shape."""

    relative = path.relative_to(NIX_STORE_ROOT)
    if not relative.parts or NIX_STORE_OBJECT_PATTERN.fullmatch(relative.parts[0]):
        return
    raise BoundaryError(f"{label} must name a valid Nix store object")


def _validated_store_path(
    value: str | os.PathLike[str],
    label: str,
    *,
    expected_basename: str | None = None,
    require_directory: bool = False,
    require_existing: bool = False,
) -> pathlib.Path:
    """Validate an immutable path below /nix/store."""

    try:
        raw_value = os.fspath(value)
    except TypeError as error:
        raise BoundaryError(f"{label} is not a path") from error
    if not isinstance(raw_value, str):
        raise BoundaryError(f"{label} is not a text path")
    path = _absolute_normalized_path(raw_value, label)
    try:
        path.relative_to(NIX_STORE_ROOT)
    except ValueError as error:
        raise BoundaryError(f"{label} must be below /nix/store") from error
    if path == NIX_STORE_ROOT:
        raise BoundaryError(f"{label} must name an immutable store asset")
    _validate_store_object_component(path, label)
    if expected_basename is not None and path.name != expected_basename:
        raise BoundaryError(
            f"{label} must have basename {expected_basename}"
        )
    if require_existing or path.exists():
        _check_store_ancestor_chain(path, label)
    try:
        info = os.lstat(path)
    except FileNotFoundError as error:
        if require_existing:
            raise BoundaryError(f"{label} is unavailable") from error
        return path
    except OSError as error:
        raise BoundaryError(f"{label} is unavailable") from error
    if stat.S_ISLNK(info.st_mode):
        raise BoundaryError(f"{label} must not be a symlink")
    if info.st_mode & 0o022:
        raise BoundaryError(f"{label} must not be writable by group or other")
    if require_directory and not stat.S_ISDIR(info.st_mode):
        raise BoundaryError(f"{label} must be a directory")
    return path


def validate_credential_source(
    path_value: str,
    *,
    label: str = "credential",
    allow_service_owner: bool = False,
) -> pathlib.Path:
    """Validate a root-owned, non-link, private regular credential file."""

    path = _absolute_normalized_path(path_value, label)
    if path == pathlib.Path("/nix/store") or pathlib.Path("/nix/store") in path.parents:
        raise BoundaryError(f"{label} must not come from /nix/store")
    _check_ancestor_chain(path, label)
    try:
        info = os.lstat(path)
    except OSError as error:
        raise BoundaryError(f"{label} is unavailable") from error
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise BoundaryError(f"{label} must be a regular non-symlink file")
    if info.st_uid != 0 and not (allow_service_owner and info.st_uid == os.geteuid()):
        raise BoundaryError(f"{label} must be root-owned")
    if info.st_mode & 0o022 or info.st_mode & 0o111:
        raise BoundaryError(f"{label} has unsafe ownership or mode")
    if info.st_mode & 0o007:
        raise BoundaryError(f"{label} must not be world-readable")
    return path


def validate_host_projection(path_value: str, *, label: str = "host projection") -> pathlib.Path:
    """Validate a root-owned regular file or directory for read-only binding."""

    path = _absolute_normalized_path(path_value, label)
    if path == pathlib.Path("/") or pathlib.Path("/home") in path.parents or path == pathlib.Path("/home"):
        raise BoundaryError(f"{label} is broader than the declared projection boundary")
    if path == NIXOS_PROJECTION_ROOT:
        raise BoundaryError(f"{label} is broader than the safe host projection boundary")
    if any(
        path == root or root in path.parents
        for root in FORBIDDEN_PROJECTION_ROOTS
        if root != NIXOS_PROJECTION_ROOT
    ):
        raise BoundaryError(f"{label} is broader than the safe host projection boundary")
    _check_ancestor_chain(path, label)
    try:
        info = os.lstat(path)
    except OSError as error:
        raise BoundaryError(f"{label} is unavailable") from error
    if NIXOS_PROJECTION_ROOT in path.parents and not stat.S_ISREG(info.st_mode):
        raise BoundaryError(f"{label} below /etc/nixos must be a regular file")
    if stat.S_ISLNK(info.st_mode) or not (
        stat.S_ISREG(info.st_mode) or stat.S_ISDIR(info.st_mode)
    ):
        raise BoundaryError(f"{label} must be a regular file or directory")
    if info.st_uid != 0 or info.st_mode & 0o022:
        raise BoundaryError(f"{label} must be root-owned and not group/world writable")
    return path


def _mount_has_project_quota(path: pathlib.Path) -> bool:
    """Read mount metadata without changing quota state or invoking a daemon."""

    if os.environ.get("GC_PROJECT_QUOTA_SUPPORTED") == "1":
        return True
    try:
        mountinfo = pathlib.Path("/proc/self/mountinfo").read_text(encoding="utf-8")
    except OSError:
        return False
    candidate = str(path)
    best_mount = ""
    best_options = ""
    for line in mountinfo.splitlines():
        fields = line.split(" - ", 1)
        if len(fields) != 2:
            continue
        pre, post = fields
        pre_fields = pre.split()
        if len(pre_fields) < 6:
            continue
        mountpoint = pre_fields[4].replace("\\040", " ").replace("\\011", "\t")
        if candidate != mountpoint and not candidate.startswith(mountpoint.rstrip("/") + "/"):
            continue
        if len(mountpoint) >= len(best_mount):
            best_mount = mountpoint
            best_options = ",".join((pre_fields[5], post))
    return any(
        option in best_options.split(",")
        for option in ("prjquota", "pquota", "project_quota")
    )


def require_project_quota(path_value: str) -> pathlib.Path:
    path = _absolute_normalized_path(path_value, "project-quota path")
    if not path.exists() or not path.is_dir():
        raise BoundaryError("project-quota path must be an existing directory")
    if not _mount_has_project_quota(path):
        raise BoundaryError("project-quota support is required for contributor readiness")
    return path


def check_free_space(path_value: str, reserve_bytes: int) -> int:
    path = _absolute_normalized_path(path_value, "free-space path")
    if reserve_bytes < 0:
        raise BoundaryError("free-space reserve must not be negative")
    try:
        usage = os.statvfs(path)
    except OSError as error:
        raise BoundaryError("free-space path is unavailable") from error
    available = usage.f_bavail * usage.f_frsize
    if available < reserve_bytes:
        raise BoundaryError("free-space-reserve")
    return available


def publish_reserve_breach(
    status_path: str | os.PathLike[str],
    *,
    generation: str,
    state_schema: str,
) -> pathlib.Path:
    """Atomically block all readiness-gated work after reserve exhaustion."""

    if not generation or not state_schema:
        raise BoundaryError("free-space reserve breach lacks generation metadata")
    return write_status(
        status_path,
        _blocked_status(generation, state_schema, "free-space-reserve"),
    )


def monitor_free_space_once(
    *,
    path: str,
    reserve_bytes: int,
    status_path: str,
    generation: str,
    state_schema: str,
) -> int:
    """Publish the blocking status before returning a failed monitor result."""

    try:
        return check_free_space(path, reserve_bytes)
    except BoundaryError as error:
        if str(error) == "free-space-reserve":
            publish_reserve_breach(
                status_path,
                generation=generation,
                state_schema=state_schema,
            )
        raise


def _same_file(left: os.stat_result, right: os.stat_result) -> bool:
    return left.st_dev == right.st_dev and left.st_ino == right.st_ino


def _managed_asset_directory_flags() -> int:
    return (
        os.O_RDONLY
        | os.O_CLOEXEC
        | getattr(os, "O_DIRECTORY", 0)
        | getattr(os, "O_NOFOLLOW", 0)
    )


def _validate_managed_destination_info(info: os.stat_result) -> None:
    if not stat.S_ISDIR(info.st_mode):
        raise BoundaryError("managed asset destination is not a directory")
    # The production materializer is the privileged (+) ExecStartPre and
    # therefore requires root ownership.  An unprivileged package/fixture
    # invocation cannot create a root-owned directory, so it validates the
    # equivalent owner boundary for that non-privileged execution.
    effective_uid = os.geteuid()
    required_uid = 0 if effective_uid == 0 else effective_uid
    if info.st_uid != required_uid:
        raise BoundaryError("managed asset destination must be root-owned")
    if info.st_mode & 0o077 or info.st_mode & 0o700 != 0o700:
        raise BoundaryError("managed asset destination must be private")


def _validate_managed_parent_binding(
    parent: pathlib.Path,
    parent_fd: int,
    *,
    expected: os.stat_result | None = None,
) -> None:
    try:
        path_info = os.lstat(parent)
        fd_info = os.fstat(parent_fd)
    except OSError as error:
        raise BoundaryError("managed asset destination parent is unavailable") from error
    if not stat.S_ISDIR(path_info.st_mode) or path_info.st_mode & 0o022:
        raise BoundaryError(
            "managed asset destination parent is not a private directory"
        )
    if not _same_file(path_info, fd_info) or (
        expected is not None and not _same_file(path_info, expected)
    ):
        raise BoundaryError("managed asset destination parent was replaced")


def _validate_managed_destination_binding(
    destination_name: str,
    parent_fd: int,
    destination_fd: int,
    *,
    expected: os.stat_result | None = None,
) -> None:
    try:
        path_info = os.lstat(destination_name, dir_fd=parent_fd)
        fd_info = os.fstat(destination_fd)
    except OSError as error:
        raise BoundaryError("managed asset destination was replaced") from error
    if (
        not stat.S_ISDIR(path_info.st_mode)
        or not _same_file(path_info, fd_info)
        or (expected is not None and not _same_file(path_info, expected))
    ):
        raise BoundaryError("managed asset destination was replaced")


def _open_managed_asset_directory(destination: pathlib.Path) -> tuple[int, int]:
    """Open and validate the destination while retaining its parent binding."""

    if destination == pathlib.Path("/") or not destination.name:
        raise BoundaryError("managed asset destination must name a directory")
    parent = destination.parent
    _check_ancestor_chain(parent, "managed asset destination")
    try:
        expected_parent = os.lstat(parent)
    except OSError as error:
        raise BoundaryError("managed asset destination parent is unavailable") from error
    flags = _managed_asset_directory_flags()
    try:
        parent_fd = os.open(parent, flags)
    except OSError as error:
        raise BoundaryError("managed asset destination parent is unavailable") from error

    destination_fd = -1
    try:
        _validate_managed_parent_binding(
            parent,
            parent_fd,
            expected=expected_parent,
        )
        try:
            expected_destination = os.lstat(
                destination.name,
                dir_fd=parent_fd,
            )
        except FileNotFoundError:
            expected_destination = None
        except OSError as error:
            raise BoundaryError(
                "managed asset destination is unavailable"
            ) from error
        try:
            destination_fd = os.open(destination.name, flags, dir_fd=parent_fd)
        except FileNotFoundError:
            try:
                os.mkdir(destination.name, mode=0o700, dir_fd=parent_fd)
            except FileExistsError:
                pass
            except OSError as error:
                raise BoundaryError(
                    "managed asset destination could not be created"
                ) from error
            try:
                destination_fd = os.open(
                    destination.name,
                    flags,
                    dir_fd=parent_fd,
                )
            except OSError as error:
                raise BoundaryError(
                    "managed asset destination is unavailable"
                ) from error
        except OSError as error:
            raise BoundaryError("managed asset destination is unavailable") from error
        info = os.fstat(destination_fd)
        _validate_managed_destination_info(info)
        _validate_managed_destination_binding(
            destination.name,
            parent_fd,
            destination_fd,
            expected=expected_destination,
        )
        return parent_fd, destination_fd
    except BaseException:
        if destination_fd >= 0:
            try:
                os.close(destination_fd)
            except OSError:
                pass
        try:
            os.close(parent_fd)
        except OSError:
            pass
        raise


class _ManagedAssetMutationError(BoundaryError):
    def __init__(self, message: str, *, changed: bool) -> None:
        super().__init__(message)
        self.changed = changed


def _exception_detail(error: BaseException) -> str:
    cause = error.__cause__
    if cause is None:
        return str(error)
    return f"{error} (cause: {cause})"


class _ManagedAssetDurabilityError(BoundaryError):
    def __init__(
        self,
        mutation_error: BoundaryError,
        durability_error: BoundaryError,
    ) -> None:
        self.mutation_error = mutation_error
        self.durability_error = durability_error
        super().__init__(
            "managed asset rotation failed after a mutation and directory "
            "durability could not be established: "
            f"mutation={_exception_detail(mutation_error)}; "
            f"durability={_exception_detail(durability_error)}"
        )


def _managed_asset_needs_rotation(
    destination_fd: int,
    destination: pathlib.Path,
    source_path: pathlib.Path,
    name: str,
) -> bool:
    target = destination / name
    try:
        info = os.lstat(name, dir_fd=destination_fd)
    except FileNotFoundError:
        return True
    except OSError as error:
        raise BoundaryError(f"managed asset target is unavailable: {target}") from error
    if not stat.S_ISLNK(info.st_mode):
        raise BoundaryError(f"durable managed asset would be replaced: {target}")
    try:
        current_value = os.readlink(name, dir_fd=destination_fd)
    except OSError as error:
        raise BoundaryError(f"managed asset target is unreadable: {target}") from error
    if current_value == str(source_path):
        return False
    try:
        _validated_store_path(
            current_value,
            f"existing managed asset target {name}",
            expected_basename=name,
            require_directory=True,
        )
    except BoundaryError as error:
        raise BoundaryError(f"durable managed asset would be replaced: {target}") from error
    return True


def _replace_managed_link_atomically(
    destination_fd: int,
    destination: pathlib.Path,
    source_path: pathlib.Path,
    name: str,
) -> None:
    target = destination / name
    for attempt in range(32):
        temporary = (
            f".{name}.{os.getpid()}.{time.time_ns()}.{attempt}.tmp"
        )
        try:
            os.symlink(
                str(source_path),
                temporary,
                target_is_directory=True,
                dir_fd=destination_fd,
            )
        except FileExistsError:
            continue
        except OSError as error:
            raise BoundaryError(
                f"managed asset temporary link could not be created: {temporary}"
            ) from error
        try:
            os.replace(
                temporary,
                name,
                src_dir_fd=destination_fd,
                dst_dir_fd=destination_fd,
            )
        except OSError as error:
            try:
                os.unlink(temporary, dir_fd=destination_fd)
            except FileNotFoundError:
                pass
            except OSError as cleanup_error:
                raise _ManagedAssetMutationError(
                    f"managed asset temporary link could not be cleaned up: "
                    f"{destination / temporary}",
                    changed=True,
                ) from cleanup_error
            raise _ManagedAssetMutationError(
                f"managed asset link rotation failed: {target}",
                changed=True,
            ) from error
        try:
            os.unlink(temporary, dir_fd=destination_fd)
        except FileNotFoundError:
            pass
        except OSError as error:
            raise _ManagedAssetMutationError(
                f"managed asset temporary link could not be cleaned up: {destination / temporary}",
                changed=True,
            ) from error
        return
    raise BoundaryError("managed asset temporary link name is unavailable")


def _fsync_managed_directory(destination_fd: int) -> None:
    try:
        os.fsync(destination_fd)
    except OSError as error:
        raise BoundaryError("managed asset directory could not be synced") from error


def materialize_assets(source_value: str, destination_value: str) -> pathlib.Path:
    """Materialize immutable managed links and safely rotate package generations."""

    source = _validated_store_path(
        source_value,
        "managed asset source",
        require_directory=True,
        require_existing=True,
    )
    destination = _absolute_normalized_path(
        destination_value,
        "managed asset destination",
    )
    assets = {}
    for name in MANAGED_ASSET_NAMES:
        source_path = source / name
        assets[name] = _validated_store_path(
            source_path,
            f"managed asset source {name}",
            expected_basename=name,
            require_directory=True,
            require_existing=True,
        )
    parent_fd, destination_fd = _open_managed_asset_directory(destination)
    try:
        # Repair any directory metadata left unsynced by a killed or failed
        # invocation before inspecting targets or attempting another rotation.
        _fsync_managed_directory(destination_fd)

        pending = []
        for name, source_path in assets.items():
            if _managed_asset_needs_rotation(
                destination_fd,
                destination,
                source_path,
                name,
            ):
                pending.append((name, source_path))

        changed = False
        try:
            for name, source_path in pending:
                # Re-check immediately before the rename so a durable
                # non-symlink state observed after the preflight is still
                # fail-closed.
                if _managed_asset_needs_rotation(
                    destination_fd,
                    destination,
                    source_path,
                    name,
                ):
                    _replace_managed_link_atomically(
                        destination_fd,
                        destination,
                        source_path,
                        name,
                    )
                    changed = True
        except BoundaryError as error:
            if changed or getattr(error, "changed", False):
                try:
                    _fsync_managed_directory(destination_fd)
                except BoundaryError as durability_error:
                    raise _ManagedAssetDurabilityError(
                        error,
                        durability_error,
                    ) from error
            raise

        if changed:
            _fsync_managed_directory(destination_fd)
        _validate_managed_destination_info(os.fstat(destination_fd))
        _validate_managed_parent_binding(destination.parent, parent_fd)
        _validate_managed_destination_binding(
            destination.name,
            parent_fd,
            destination_fd,
        )
        return destination
    finally:
        try:
            os.close(destination_fd)
        except OSError:
            pass
        try:
            os.close(parent_fd)
        except OSError:
            pass


def _read_bounded_line(stream: socket.socket, limit: int) -> bytes:
    data = bytearray()
    while b"\n" not in data:
        chunk = stream.recv(min(4096, limit - len(data)))
        if not chunk:
            raise BoundaryError("sidecar channel closed before a complete frame")
        data.extend(chunk)
        if len(data) >= limit:
            raise BoundaryError("sidecar frame exceeds the size limit")
    line, remainder = bytes(data).split(b"\n", 1)
    if remainder:
        raise BoundaryError("sidecar channel pipelined multiple frames")
    return line


def _open_private_listener(path_value: str, *, socket_group: str | None = None) -> socket.socket:
    path = _absolute_normalized_path(path_value, "sidecar socket")
    path.parent.mkdir(mode=0o750, parents=True, exist_ok=True)
    parent_info = os.lstat(path.parent)
    if (
        stat.S_ISLNK(parent_info.st_mode)
        or not stat.S_ISDIR(parent_info.st_mode)
        or parent_info.st_uid != os.geteuid()
        or parent_info.st_mode & 0o022
    ):
        raise BoundaryError("sidecar socket directory is not server-owned and non-writable")
    if os.path.lexists(path):
        info = os.lstat(path)
        if not stat.S_ISSOCK(info.st_mode):
            raise BoundaryError("sidecar socket path is occupied by a non-socket")
        path.unlink()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(path))
    os.chmod(path, 0o660)
    if socket_group is not None:
        try:
            group_id = grp.getgrnam(socket_group).gr_gid
            os.chown(path, -1, group_id)
        except (KeyError, OSError) as error:
            listener.close()
            pathlib.Path(path).unlink(missing_ok=True)
            raise BoundaryError("sidecar socket group is unavailable") from error
    listener.listen(32)
    listener.settimeout(0.5)
    return listener


def run_fdproxy_sidecar(
    *,
    egress_socket: str,
    fdproxy_script: str,
    listen: str,
    command: Sequence[str],
    server_uid: int | None = None,
) -> int:
    """Connect one sidecar to egress, then pass only that channel to fdproxy."""

    socket_path = _absolute_normalized_path(egress_socket, "egress socket")
    proxy_path = _absolute_normalized_path(fdproxy_script, "fdproxy script")
    if not proxy_path.is_file() or proxy_path.is_symlink():
        raise BoundaryError("fdproxy script is unavailable or is a symlink")
    if not command:
        raise BoundaryError("fdproxy sidecar command must not be empty")

    channel_fd = -1
    deadline = time.monotonic() + 15.0
    while True:
        channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            channel.settimeout(2.0)
            channel.connect(str(socket_path))
            if server_uid is not None and _peer_uid(channel) != server_uid:
                raise BoundaryError("egress server uid is unauthorized")
            channel_fd = channel.detach()
            break
        except OSError as error:
            channel.close()
            if error.errno not in {errno.ENOENT, errno.ECONNREFUSED} or time.monotonic() >= deadline:
                raise BoundaryError("egress channel is unavailable") from error
            time.sleep(0.05)

    os.set_inheritable(channel_fd, False)
    proxy_command = [
        sys.executable,
        str(proxy_path),
        "--channel-fd",
        str(channel_fd),
        "--listen",
        listen,
        "--",
        *command,
    ]
    try:
        child = subprocess.Popen(
            proxy_command,
            close_fds=True,
            pass_fds=(channel_fd,),
            env=dict(os.environ),
        )
    finally:
        os.close(channel_fd)
        channel_fd = -1
    return child.wait()


def _close_relay_descriptors(descriptors: list[int]) -> None:
    while descriptors:
        descriptor = descriptors.pop()
        try:
            os.close(descriptor)
        except OSError:
            pass


def _extract_relay_descriptors(
    ancillary: Sequence[tuple[int, int, bytes]],
    *,
    max_descriptors: int = AGENT_RELAY_MAX_ATTACHMENTS,
) -> list[int]:
    """Validate and make received relay descriptors close-on-exec."""

    descriptors: list[int] = []
    malformed = False
    item_size = array.array("i").itemsize
    for level, kind, data in ancillary:
        if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS or not data:
            malformed = True
            continue
        complete = len(data) - (len(data) % item_size)
        if complete != len(data):
            malformed = True
        values = array.array("i")
        if complete:
            values.frombytes(data[:complete])
            descriptors.extend(int(value) for value in values)
    if len(descriptors) > max_descriptors:
        malformed = True
    if malformed:
        _close_relay_descriptors(descriptors)
        raise BoundaryError("agent relay ancillary data is malformed or unauthorized")
    try:
        for descriptor in descriptors:
            flags = fcntl.fcntl(descriptor, fcntl.F_GETFD)
            fcntl.fcntl(descriptor, fcntl.F_SETFD, flags | fcntl.FD_CLOEXEC)
    except OSError as error:
        _close_relay_descriptors(descriptors)
        raise BoundaryError("agent relay attachment fd is invalid") from error
    return descriptors


def _recv_relay_message(
    source: socket.socket,
    *,
    allow_descriptors: bool,
    max_descriptors: int,
) -> tuple[bytes, list[int]]:
    descriptors: list[int] = []
    try:
        data, ancillary, flags, _address = source.recvmsg(
            AGENT_RELAY_CHUNK_BYTES,
            socket.CMSG_SPACE(array.array("i").itemsize * AGENT_RELAY_MAX_ATTACHMENTS),
            getattr(socket, "MSG_CMSG_CLOEXEC", 0),
        )
        descriptors = _extract_relay_descriptors(
            ancillary,
            max_descriptors=max_descriptors,
        )
        if flags & getattr(socket, "MSG_CTRUNC", 0):
            raise BoundaryError("agent relay ancillary data was truncated")
        if descriptors and not allow_descriptors:
            raise BoundaryError("agent relay received unauthorized descriptors")
        if not data and descriptors:
            raise BoundaryError("agent relay descriptors were sent without a frame")
        return data, descriptors
    except BaseException:
        _close_relay_descriptors(descriptors)
        raise


def _send_relay_message(
    destination: socket.socket,
    data: bytes,
    descriptors: list[int],
) -> None:
    try:
        ancillary = []
        if descriptors:
            rights = array.array("i", descriptors).tobytes()
            ancillary = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, rights)]
        offset = 0
        while offset < len(data):
            sent = destination.sendmsg([data[offset:]], ancillary)
            if sent <= 0:
                raise BoundaryError("agent relay did not forward a frame")
            offset += sent
            ancillary = []
    finally:
        _close_relay_descriptors(descriptors)


def _track_relay_frame(
    data: bytes,
    *,
    frame_bytes: int,
    complete: bool,
    limit: int,
) -> tuple[int, bool]:
    if complete:
        return frame_bytes, complete
    newline = data.find(b"\n")
    frame_bytes += len(data) if newline < 0 else newline + 1
    if frame_bytes > limit:
        raise BoundaryError("agent relay frame exceeds the size limit")
    return frame_bytes, newline >= 0


def _relay_agent_connection(
    client: socket.socket,
    *,
    private_socket: str,
    allowed_uid: int,
) -> None:
    upstream: socket.socket | None = None
    selector: selectors.BaseSelector | None = None
    try:
        if _peer_uid(client) != allowed_uid:
            raise BoundaryError("agent relay client uid is unauthorized")
        upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        upstream.connect(private_socket)
        selector = selectors.DefaultSelector()
        selector.register(client, selectors.EVENT_READ, (upstream, True))
        selector.register(upstream, selectors.EVENT_READ, (client, False))
        client_frame_bytes = 0
        client_frame_complete = False
        client_attachment_count = 0
        upstream_frame_bytes = 0
        upstream_frame_complete = False
        while selector.get_map():
            for key, _mask in selector.select():
                source = key.fileobj
                destination, client_to_upstream = key.data
                data, descriptors = _recv_relay_message(
                    source,
                    allow_descriptors=client_to_upstream and not client_frame_complete,
                    max_descriptors=(
                        AGENT_RELAY_MAX_ATTACHMENTS - client_attachment_count
                        if client_to_upstream and not client_frame_complete
                        else 0
                    ),
                )
                try:
                    if not data:
                        _close_relay_descriptors(descriptors)
                        return
                    if client_to_upstream:
                        client_frame_bytes, client_frame_complete = _track_relay_frame(
                            data,
                            frame_bytes=client_frame_bytes,
                            complete=client_frame_complete,
                            limit=AGENT_RELAY_METADATA_BYTES,
                        )
                        client_attachment_count += len(descriptors)
                    else:
                        upstream_frame_bytes, upstream_frame_complete = _track_relay_frame(
                            data,
                            frame_bytes=upstream_frame_bytes,
                            complete=upstream_frame_complete,
                            limit=AGENT_RELAY_RESPONSE_BYTES,
                        )
                    _send_relay_message(destination, data, descriptors)
                except BaseException:
                    _close_relay_descriptors(descriptors)
                    raise
    except (BoundaryError, OSError):
        pass
    finally:
        if selector is not None:
            selector.close()
        if upstream is not None:
            upstream.close()
        client.close()


def serve_agent_relay(
    *,
    public_socket: str,
    private_socket: str,
    socket_group: str,
    allowed_uid: int,
) -> None:
    """Expose only the launcher wire protocol, without inheriting its token."""

    listener = _open_private_listener(public_socket, socket_group=socket_group)

    def relay(client: socket.socket) -> None:
        _relay_agent_connection(
            client,
            private_socket=private_socket,
            allowed_uid=allowed_uid,
        )

    try:
        while True:
            try:
                client, _address = listener.accept()
            except socket.timeout:
                continue
            threading.Thread(target=relay, args=(client,), daemon=True).start()
    finally:
        listener.close()
        pathlib.Path(public_socket).unlink(missing_ok=True)


def _peer_uid(connection: socket.socket) -> int:
    credential_size = struct.calcsize("3i")
    try:
        credentials = connection.getsockopt(
            socket.SOL_SOCKET,
            socket.SO_PEERCRED,
            credential_size,
        )
        if not isinstance(credentials, bytes) or len(credentials) != credential_size:
            raise BoundaryError("peer credentials are truncated or malformed")
        _pid, uid, _gid = struct.unpack("3i", credentials)
    except (OSError, struct.error) as error:
        raise BoundaryError("peer credentials are truncated or malformed") from error
    if uid < 0:
        raise BoundaryError("peer credentials contain an invalid uid")
    return uid


def _domain_matches(host: str, allowed_domains: Sequence[str]) -> bool:
    normalized = host.rstrip(".").lower()
    if not normalized or len(normalized) > 253:
        return False
    try:
        ipaddress.ip_address(normalized)
    except ValueError:
        pass
    else:
        return False
    return any(
        normalized == domain.lower()
        or (domain.startswith("*.") and normalized.endswith(domain[1:].lower()))
        for domain in allowed_domains
    )


def _public_addresses(host: str, port: int) -> list[tuple[int, int, int, str, tuple[object, ...]]]:
    try:
        resolved = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    except OSError as error:
        raise BoundaryError("egress DNS resolution failed") from error
    addresses: list[tuple[int, int, int, str, tuple[object, ...]]] = []
    seen: set[tuple[int, str]] = set()
    for family, kind, protocol, _canonname, sockaddr in resolved:
        address = ipaddress.ip_address(str(sockaddr[0]))
        if any(address in network for network in PRIVATE_NETWORKS) or (
            not address.is_global
        ):
            raise BoundaryError("egress DNS result is not a permitted public address")
        key = (family, str(address))
        if key not in seen:
            seen.add(key)
            addresses.append((family, kind, protocol, "", sockaddr))
    if not addresses:
        raise BoundaryError("egress DNS returned no usable address")
    return addresses


def _connect_allowlisted(
    host: str,
    port: int,
    *,
    allowed_domains: Sequence[str],
) -> socket.socket:
    if port != DEFAULT_ALLOWED_PORT or not _domain_matches(host, allowed_domains):
        raise BoundaryError("egress destination is not allowlisted")
    candidates = _public_addresses(host, port)
    last_error: OSError | None = None
    for family, kind, protocol, _canonname, sockaddr in candidates:
        upstream = socket.socket(family, kind, protocol)
        upstream.settimeout(10.0)
        try:
            upstream.connect(sockaddr)
            peer = ipaddress.ip_address(str(upstream.getpeername()[0]))
            if any(peer in network for network in PRIVATE_NETWORKS) or not peer.is_global:
                raise BoundaryError("connected egress peer failed address validation")
            upstream.settimeout(None)
            os.set_inheritable(upstream.fileno(), False)
            return upstream
        except (BoundaryError, OSError) as error:
            last_error = error if isinstance(error, OSError) else OSError(str(error))
            upstream.close()
    raise BoundaryError("allowlisted egress connection failed") from last_error


def _send_fdproxy_response(
    connection: socket.socket,
    response: Mapping[str, object],
    descriptor: int | None = None,
) -> None:
    payload = json.dumps(dict(response), sort_keys=True, separators=(",", ":")).encode() + b"\n"
    if descriptor is None:
        connection.sendall(payload)
        return
    descriptors = array.array("i", [descriptor])
    connection.sendmsg(
        [payload],
        [(socket.SOL_SOCKET, socket.SCM_RIGHTS, descriptors.tobytes())],
    )


def serve_egress_peer(
    *,
    socket_path: str,
    socket_group: str,
    auth_token: str,
    allowed_domains: Sequence[str],
    allowed_uids: Sequence[int],
) -> None:
    """Serve authenticated fdproxy/1 requests over persistent channels."""

    if not auth_token or len(auth_token.encode()) > 512:
        raise BoundaryError("egress authentication token is malformed")
    listener = _open_private_listener(socket_path, socket_group=socket_group)
    allowed_uid_set = set(allowed_uids)

    def serve_one(connection: socket.socket) -> None:
        try:
            if _peer_uid(connection) not in allowed_uid_set:
                return
            while True:
                request: object = {}
                try:
                    request = json.loads(_read_bounded_line(connection, 8192))
                    if (
                        not isinstance(request, dict)
                        or set(request)
                        != {
                            "version",
                            "operation",
                            "request_id",
                            "auth",
                            "host",
                            "port",
                        }
                        or request.get("version") != FDPROXY_PROTOCOL
                        or request.get("operation") != "connect"
                        or request.get("auth") != auth_token
                        or not isinstance(request.get("request_id"), str)
                        or not isinstance(request.get("host"), str)
                        or type(request.get("port")) is not int
                    ):
                        raise BoundaryError("egress request is malformed or unauthorized")
                except BoundaryError as error:
                    if str(error) == "sidecar channel closed before a complete frame":
                        return
                    return
                except (OSError, json.JSONDecodeError):
                    return

                request_id = request["request_id"]
                upstream: socket.socket | None = None
                try:
                    upstream = _connect_allowlisted(
                        request["host"],
                        request["port"],
                        allowed_domains=allowed_domains,
                    )
                    _send_fdproxy_response(
                        connection,
                        {
                            "version": FDPROXY_PROTOCOL,
                            "request_id": request_id,
                            "ok": True,
                        },
                        upstream.fileno(),
                    )
                except (BoundaryError, OSError):
                    try:
                        _send_fdproxy_response(
                            connection,
                            {
                                "version": FDPROXY_PROTOCOL,
                                "request_id": request_id,
                                "ok": False,
                            },
                        )
                    except OSError:
                        return
                finally:
                    if upstream is not None:
                        upstream.close()
        finally:
            connection.close()

    try:
        while True:
            try:
                connection, _address = listener.accept()
            except socket.timeout:
                continue
            thread = threading.Thread(target=serve_one, args=(connection,), daemon=True)
            thread.start()
    finally:
        listener.close()
        pathlib.Path(socket_path).unlink(missing_ok=True)


def _identifier(value: str, label: str) -> str:
    if not IDENTIFIER_PATTERN.fullmatch(value) or ".." in value:
        raise ActivationError(f"{label} is malformed")
    return value


def classify_failure(value: object) -> str:
    """Map a probe failure into the closed fallback classification."""

    if isinstance(value, Mapping):
        code = value.get("error_code")
        detail = value.get("error", "")
        if isinstance(code, str) and code in FAILURE_CODES:
            detail_code = classify_failure(detail) if detail else None
            if code in FALLBACK_FAILURE_CODES and detail_code in {
                "authentication",
                "network",
                "quota",
                "malformed",
                "unknown",
            }:
                return detail_code
            return code
        value = f"{code or ''} {detail or ''}"
    text = str(value).strip().lower()
    if not text:
        return "unknown"
    if "\x00" in text:
        return "malformed"
    if any(
        marker in text
        for marker in (
            "authentication",
            "unauthorized",
            "invalid token",
            "token expired",
            "credential",
            "401",
            "403",
        )
    ):
        return "authentication"
    if any(
        marker in text
        for marker in (
            "network",
            "connection refused",
            "connection reset",
            "dns",
            "timed out",
            "timeout",
            "proxy",
            "tls",
        )
    ):
        return "network"
    if any(
        marker in text
        for marker in ("quota", "rate limit", "rate_limit", "429", "capacity exceeded")
    ):
        return "quota"
    if any(
        marker in text
        for marker in (
            "malformed",
            "invalid json",
            "invalid json-rpc",
            "protocol",
            "parse error",
            "content-length",
        )
    ):
        return "malformed"
    if any(marker in text for marker in ("closed", "eof", "end of file")):
        return "closed"
    if any(
        marker in text
        for marker in (
            "not supported",
            "unsupported",
            "unknown model",
            "model not found",
        )
    ):
        return "unsupported"
    if any(marker in text for marker in ("model unavailable", "temporarily unavailable", "unavailable")):
        return "unavailable"
    return "unknown"


def parse_probe(profile: str, value: object) -> ProbeResult:
    if profile not in PROFILE_SETTINGS:
        raise ActivationError(f"unknown profile: {profile}")
    if isinstance(value, ProbeResult):
        if value.profile != profile:
            return ProbeResult(profile=profile, ok=False, error_code="malformed")
        if value.ok:
            expected = PROFILE_SETTINGS[profile]
            if (
                value.model != expected["model"]
                or value.context != expected["context"]
                or value.effort != expected["effort"]
            ):
                return ProbeResult(profile=profile, ok=False, error_code="malformed")
        return value
    if not isinstance(value, Mapping):
        return ProbeResult(profile=profile, ok=False, error_code="malformed")
    if value.get("profile") != profile:
        return ProbeResult(profile=profile, ok=False, error_code="malformed")
    if value.get("ok") is True:
        expected = PROFILE_SETTINGS[profile]
        if (
            value.get("model") != expected["model"]
            or value.get("context") != expected["context"]
            or value.get("effort") != expected["effort"]
        ):
            return ProbeResult(profile=profile, ok=False, error_code="malformed")
        return ProbeResult(
            profile=profile,
            ok=True,
            model=expected["model"],
            context=expected["context"],
            effort=expected["effort"],
        )
    error_code = classify_failure(value)
    return ProbeResult(
        profile=profile,
        ok=False,
        error_code=error_code,
        error=str(value.get("error", ""))[:512],
    )


def _blocked_status(generation: str, state_schema: str, error_code: str) -> dict[str, object]:
    return {
        "generation": generation,
        "state_schema": state_schema,
        "ready": False,
        "effective_profiles": {},
        "error_code": error_code,
    }


def _ready_status(
    generation: str,
    state_schema: str,
    *,
    review_profile: str,
) -> dict[str, object]:
    return {
        "generation": generation,
        "state_schema": state_schema,
        "ready": True,
        "effective_profiles": {
            "coding": "code-luna",
            "review": review_profile,
        },
        "error_code": None,
    }


def select_profiles(
    probe: Callable[[str], ProbeResult | Mapping[str, object]],
    *,
    generation: str,
    state_schema: str,
) -> dict[str, object]:
    """Probe code first, then Sol review, and use only the closed fallback."""

    code = parse_probe("code-luna", probe("code-luna"))
    if not code.ok:
        return _blocked_status(
            generation,
            state_schema,
            f"code-luna-{code.error_code or 'unknown'}",
        )
    sol = parse_probe("review-sol", probe("review-sol"))
    if sol.ok:
        return _ready_status(generation, state_schema, review_profile="review-sol")
    if sol.error_code not in FALLBACK_FAILURE_CODES:
        return _blocked_status(
            generation,
            state_schema,
            f"review-sol-{sol.error_code or 'unknown'}",
        )
    luna = parse_probe("review-luna", probe("review-luna"))
    if not luna.ok:
        return _blocked_status(
            generation,
            state_schema,
            f"review-luna-{luna.error_code or 'unknown'}",
        )
    return _ready_status(generation, state_schema, review_profile="review-luna")


def _validate_status_values(status: Mapping[str, object]) -> None:
    if not isinstance(status["generation"], str) or not status["generation"]:
        raise ActivationError("readiness status has no generation")
    if not isinstance(status["state_schema"], str) or not status["state_schema"]:
        raise ActivationError("readiness status has no state schema")
    if not isinstance(status["ready"], bool):
        raise ActivationError("readiness status has no boolean readiness")
    effective = status["effective_profiles"]
    if not isinstance(effective, dict):
        raise ActivationError("readiness effective profiles are malformed")
    if status["ready"]:
        if (
            set(effective) != {"coding", "review"}
            or effective.get("coding") != "code-luna"
            or effective.get("review") not in {"review-sol", "review-luna"}
        ):
            raise ActivationError("ready profile selection is malformed")
        if status["error_code"] is not None:
            raise ActivationError("ready readiness status has an error")
    elif effective or not isinstance(status["error_code"], str) or not status["error_code"]:
        raise ActivationError("blocked readiness status is malformed")


def write_status(path: str | os.PathLike[str], status: Mapping[str, object]) -> pathlib.Path:
    status_path = pathlib.Path(path)
    if not status_path.is_absolute() or any(part == ".." for part in status_path.parts):
        raise ActivationError("status path must be absolute and normalized")
    if set(status) != STATUS_KEYS:
        raise ActivationError("readiness status has an unauthorized shape")
    _validate_status_values(status)
    status_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = status_path.with_name(f".{status_path.name}.{os.getpid()}.tmp")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC
    descriptor = os.open(temporary, flags, 0o640)
    try:
        encoded = json.dumps(dict(status), sort_keys=True, separators=(",", ":")).encode(
            "utf-8"
        )
        os.write(descriptor, encoded + b"\n")
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    os.replace(temporary, status_path)
    os.chmod(status_path, 0o640)
    return status_path


def read_status(
    path: str | os.PathLike[str],
    *,
    generation: str,
    state_schema: str,
) -> dict[str, object]:
    try:
        with pathlib.Path(path).open("r", encoding="utf-8") as stream:
            status = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise StaleGeneration("readiness status is unreadable") from error
    if not isinstance(status, dict) or set(status) != STATUS_KEYS:
        raise StaleGeneration("readiness status shape is stale")
    if status["generation"] != generation or status["state_schema"] != state_schema:
        raise StaleGeneration("readiness status belongs to another generation")
    try:
        _validate_status_values(status)
    except ActivationError as error:
        raise StaleGeneration("readiness status values are malformed") from error
    return status


def require_ready(
    path: str | os.PathLike[str],
    *,
    generation: str,
    state_schema: str,
    profile: str,
) -> dict[str, object]:
    status = read_status(path, generation=generation, state_schema=state_schema)
    if status["ready"] is not True:
        raise ActivationError(str(status.get("error_code") or "not-ready"))
    effective = status["effective_profiles"]
    if not isinstance(effective, dict) or profile not in effective.values():
        raise ActivationError(f"profile {profile} is not ready")
    return status


def validate_run_context(
    context: object,
    *,
    generation: str,
    state_schema: str,
) -> dict[str, object]:
    if not isinstance(context, dict) or not CONTEXT_REQUIRED_KEYS.issubset(context):
        raise ActivationError("bead-owned context is incomplete")
    if (
        not isinstance(context["generation"], str)
        or not isinstance(context["state_schema"], str)
        or context["generation"] != generation
        or context["state_schema"] != state_schema
    ):
        raise StaleGeneration("bead-owned context belongs to another generation")
    if not isinstance(context["run_id"], str) or not isinstance(context["bead_id"], str):
        raise ActivationError("bead-owned identity is malformed")
    _identifier(context["run_id"], "run id")
    _identifier(context["bead_id"], "bead id")
    for field in (
        "open_work",
        "summary",
        "branch",
        "worktree",
        "review_state",
        "next_action",
    ):
        if not isinstance(context[field], str) or not context[field]:
            raise ActivationError(f"bead-owned {field} is malformed")
    worktree = pathlib.Path(context["worktree"])
    if not worktree.is_absolute() or any(part == ".." for part in worktree.parts):
        raise ActivationError("bead-owned worktree is malformed")
    counters = context["retry_counters"]
    if not isinstance(counters, dict) or any(
        not isinstance(key, str) or type(value) is not int or value < 0
        for key, value in counters.items()
    ):
        raise ActivationError("bead-owned retry counters are malformed")
    if not isinstance(context["commits"], list) or any(
        not isinstance(commit, str) or not commit for commit in context["commits"]
    ):
        raise ActivationError("bead-owned commits are malformed")
    return dict(context)


def reconstruct_prompt(
    context: object,
    *,
    generation: str,
    state_schema: str,
) -> str:
    durable = validate_run_context(
        context,
        generation=generation,
        state_schema=state_schema,
    )
    return "\n".join(
        (
            "Continue the assigned bead from durable state.",
            "Start a fresh ACP conversation; do not resume an old ACP session.",
            f"Run: {durable['run_id']}",
            f"Bead: {durable['bead_id']}",
            f"Open work: {durable['open_work']}",
            f"Summary: {durable['summary']}",
            f"Branch: {durable['branch']}",
            f"Commits: {json.dumps(durable['commits'], sort_keys=True)}",
            f"Assigned worktree: {durable['worktree']}",
            f"Review state: {durable['review_state']}",
            f"Retry counters: {json.dumps(durable['retry_counters'], sort_keys=True)}",
            f"Next action: {durable['next_action']}",
        )
    )


def increment_retry_counter(
    context_path: str | os.PathLike[str],
    *,
    counter: str,
    generation: str,
    state_schema: str,
) -> dict[str, object]:
    if not counter or not re.fullmatch(r"[a-z][a-z0-9_-]{0,63}", counter):
        raise ActivationError("retry counter name is malformed")
    path = pathlib.Path(context_path)
    if not path.is_absolute() or any(part == ".." for part in path.parts):
        raise ActivationError("bead context path must be absolute and normalized")
    if path.is_symlink():
        raise ActivationError("bead context path must not be a symlink")
    lock_path = path.with_name(f".{path.name}.lock")
    descriptor = os.open(
        lock_path,
        os.O_RDWR
        | os.O_CREAT
        | os.O_CLOEXEC
        | getattr(os, "O_NOFOLLOW", 0),
        0o600,
    )
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        try:
            with path.open("r", encoding="utf-8") as stream:
                context = json.load(stream)
        except (OSError, json.JSONDecodeError) as error:
            raise ActivationError("bead-owned context is unreadable") from error
        durable = validate_run_context(
            context,
            generation=generation,
            state_schema=state_schema,
        )
        counters = dict(durable["retry_counters"])
        counters[counter] = int(counters.get(counter, 0)) + 1
        durable["retry_counters"] = counters
        temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(durable, stream, sort_keys=True, separators=(",", ":"))
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        return durable
    finally:
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        os.close(descriptor)


def _terminal_root_path(
    root_directory: str | os.PathLike[str],
) -> pathlib.Path:
    root = _absolute_normalized_path(str(root_directory), "GC-root directory")
    return root.parent / "terminal"


def _validate_terminal_root(
    root_directory: str | os.PathLike[str],
    *,
    create: bool,
) -> pathlib.Path:
    root = _absolute_normalized_path(str(root_directory), "terminal state root")
    if create:
        try:
            root.mkdir(mode=0o750, parents=False, exist_ok=True)
        except OSError as error:
            raise RootLifecycleError("terminal state root is unavailable") from error
    try:
        info = os.lstat(root)
    except OSError as error:
        raise RootLifecycleError("terminal state root is unavailable") from error
    if (
        stat.S_ISLNK(info.st_mode)
        or not stat.S_ISDIR(info.st_mode)
        or (create and info.st_uid != os.geteuid())
        or info.st_mode & 0o027
    ):
        raise RootLifecycleError("terminal state root has unsafe ownership or mode")
    return root


def _terminal_state_path(
    terminal_root: str | os.PathLike[str],
    run_id: str,
) -> pathlib.Path:
    _identifier(run_id, "run id")
    root = _validate_terminal_root(terminal_root, create=False)
    return root / f"{run_id}.json"


def _terminal_state_root_from_path(
    state_path: str | os.PathLike[str],
    *,
    run_id: str,
) -> pathlib.Path:
    path = pathlib.Path(state_path)
    if (
        not path.is_absolute()
        or any(part == ".." for part in path.parts)
        or os.path.normpath(str(path)) != str(path)
        or path.name != f"{run_id}.json"
    ):
        raise RootLifecycleError("terminal workflow state path is malformed")
    return path.parent


def _read_json_file(
    path: pathlib.Path,
    *,
    label: str,
    limit: int,
    require_unwritable: bool = True,
) -> object:
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY
            | os.O_CLOEXEC
            | getattr(os, "O_NOFOLLOW", 0),
        )
    except OSError as error:
        raise RootLifecycleError(f"{label} is unreadable") from error
    try:
        info = os.fstat(descriptor)
        if (
            not stat.S_ISREG(info.st_mode)
            or (require_unwritable and info.st_mode & 0o022)
        ):
            raise RootLifecycleError(f"{label} has unsafe ownership or mode")
        payload = os.read(descriptor, limit + 1)
    except OSError as error:
        raise RootLifecycleError(f"{label} is unreadable") from error
    finally:
        os.close(descriptor)
    if len(payload) > limit:
        raise RootLifecycleError(f"{label} exceeds the size limit")
    try:
        return json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RootLifecycleError(f"{label} is malformed") from error


def _read_terminal_record(
    path: pathlib.Path,
    *,
    run_id: str,
    bead_id: str | None,
    generation: str,
    state_schema: str,
) -> dict[str, object]:
    value = _read_json_file(
        path,
        label="terminal workflow state",
        limit=MAX_TERMINAL_RECORD_BYTES,
    )
    if (
        not isinstance(value, dict)
        or set(value) != TERMINAL_RECORD_KEYS
        or value.get("schema") != TERMINAL_RECORD_SCHEMA
        or value.get("run_id") != run_id
        or (bead_id is not None and value.get("bead_id") != bead_id)
        or value.get("generation") != generation
        or value.get("state_schema") != state_schema
        or not isinstance(value.get("terminal_status"), str)
        or value.get("terminal_status") not in TERMINAL_WORKFLOW_STATES
    ):
        raise RootLifecycleError("terminal workflow state is stale or forged")
    return value


def _read_cancel_marker(
    cancellation_root: str | os.PathLike[str] | None,
    *,
    run_id: str,
) -> bool:
    if cancellation_root is None:
        return False
    root = _absolute_normalized_path(
        str(cancellation_root),
        "cancellation root",
    )
    if not root.exists():
        return False
    try:
        info = os.lstat(root)
    except OSError as error:
        raise RootLifecycleError("cancellation root is unavailable") from error
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
        raise RootLifecycleError("cancellation root is unsafe")
    marker = root / f"{run_id}.json"
    if not marker.exists():
        return False
    value = _read_json_file(
        marker,
        label="cancellation marker",
        limit=MAX_TERMINAL_RECORD_BYTES,
    )
    if (
        not isinstance(value, dict)
        or set(value) != {"schema", "run_id", "reason", "cancelled"}
        or value.get("schema") != 1
        or value.get("run_id") != run_id
        or value.get("cancelled") is not True
        or not isinstance(value.get("reason"), str)
    ):
        raise RootLifecycleError("cancellation marker is stale or forged")
    return True


def _read_authoritative_bead(
    bd_path: str,
    *,
    bead_id: str,
) -> dict[str, object]:
    if not bd_path or "\x00" in bd_path:
        raise RootLifecycleError("bd executable is malformed")
    command = [bd_path, "show", bead_id, "--json"]
    environment = dict(os.environ)
    environment.update({"LANG": "C", "LC_ALL": "C"})
    for name in (
        "COPILOT_GITHUB_TOKEN",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "DISCORD_TOKEN",
        "BUILD_BUDDY_API_KEY",
    ):
        environment.pop(name, None)
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=10,
            env=environment,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RootLifecycleError("authoritative root bead is unreadable") from error
    if completed.returncode != 0:
        raise RootLifecycleError("authoritative root bead query failed")
    if len(completed.stdout.encode("utf-8")) > MAX_BEAD_STATE_BYTES:
        raise RootLifecycleError("authoritative root bead is too large")
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise RootLifecycleError("authoritative root bead is malformed") from error
    if isinstance(value, list):
        if len(value) != 1:
            raise RootLifecycleError("authoritative root bead is ambiguous")
        value = value[0]
    if not isinstance(value, dict):
        raise RootLifecycleError("authoritative root bead is malformed")
    if value.get("id", value.get("bead_id")) != bead_id:
        raise RootLifecycleError("authoritative root bead identity does not match")
    metadata = value.get("metadata", {})
    if metadata is None:
        metadata = {}
    if not isinstance(metadata, dict):
        raise RootLifecycleError("authoritative root bead metadata is malformed")
    value["metadata"] = metadata
    return value


def _bead_retention_reason(
    bead: Mapping[str, object],
    *,
    cancellation: bool,
) -> str | None:
    if cancellation:
        return "cancelled"
    metadata = bead["metadata"]
    assert isinstance(metadata, dict)
    for container in (bead, metadata):
        if container.get("open_pr") is True:
            return "open-pr"
        for key in ("pull_request_url", "pr_url"):
            value = container.get(key)
            if isinstance(value, str) and value.strip():
                return "open-pr"
        state = container.get("pull_request_state")
        if isinstance(state, str) and state.lower() in {"open", "draft"}:
            return "open-pr"
    return None


def record_terminal_state(
    terminal_root: str | os.PathLike[str],
    *,
    run_id: str,
    bead_id: str,
    generation: str,
    state_schema: str,
    bd_path: str = "bd",
    cancellation_root: str | os.PathLike[str] | None = None,
) -> dict[str, object]:
    """Record terminal state only after the durable root bead is terminal."""

    _identifier(run_id, "run id")
    _identifier(bead_id, "bead id")
    if not generation or not state_schema:
        raise RootLifecycleError("terminal state generation and schema are required")
    root = _validate_terminal_root(terminal_root, create=True)
    bead = _read_authoritative_bead(bd_path, bead_id=bead_id)
    metadata = bead["metadata"]
    assert isinstance(metadata, dict)
    for key, expected in (
        ("run_id", run_id),
        ("generation", generation),
        ("state_schema", state_schema),
    ):
        if key in metadata and metadata[key] != expected:
            raise RootLifecycleError("authoritative root bead metadata is stale")
    status = bead.get("status")
    if not isinstance(status, str):
        raise RootLifecycleError("authoritative root bead status is malformed")
    status = status.strip().lower()
    open_pr_intent = (
        os.environ.get("GC_PUBLISH_ENABLED") == "1"
        and os.environ.get("GC_PUBLISH_OPEN_PR") == "1"
    )
    retention_reason = (
        "open-pr"
        if open_pr_intent
        else _bead_retention_reason(
            bead,
            cancellation=_read_cancel_marker(
                cancellation_root,
                run_id=run_id,
            ),
        )
    )
    if retention_reason is not None:
        return {
            "recorded": False,
            "retained": True,
            "reason": retention_reason,
            "run_id": run_id,
        }
    if status not in TERMINAL_WORKFLOW_STATES:
        return {
            "recorded": False,
            "retained": True,
            "reason": "nonterminal",
            "run_id": run_id,
            "status": status,
        }
    record = {
        "schema": TERMINAL_RECORD_SCHEMA,
        "run_id": run_id,
        "bead_id": bead_id,
        "generation": generation,
        "state_schema": state_schema,
        "terminal_status": status,
    }
    target = root / f"{run_id}.json"
    lock_path = root / ".lock"
    try:
        lock_fd = os.open(
            lock_path,
            os.O_RDWR
            | os.O_CREAT
            | os.O_CLOEXEC
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
    except OSError as error:
        raise RootLifecycleError("terminal state lock is unavailable") from error
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX)
        if target.exists() or target.is_symlink():
            if target.is_symlink():
                raise RootLifecycleError("terminal workflow state is an unsafe symlink")
            existing = _read_terminal_record(
                target,
                run_id=run_id,
                bead_id=bead_id,
                generation=generation,
                state_schema=state_schema,
            )
            if existing != record:
                raise RootLifecycleError("terminal workflow state is stale or forged")
            return {
                "recorded": True,
                "retained": False,
                "idempotent": True,
                "run_id": run_id,
                "terminal_status": status,
            }
        temporary = root / f".{run_id}.json.tmp"
        if temporary.exists() or temporary.is_symlink():
            if temporary.is_symlink():
                raise RootLifecycleError("terminal state temporary file is unsafe")
            temporary.unlink()
        descriptor = os.open(
            temporary,
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | os.O_CLOEXEC
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        try:
            encoded = (
                json.dumps(record, sort_keys=True, separators=(",", ":"))
                + "\n"
            ).encode("utf-8")
            offset = 0
            while offset < len(encoded):
                offset += os.write(descriptor, encoded[offset:])
            os.fsync(descriptor)
            os.fchmod(descriptor, 0o640)
        finally:
            os.close(descriptor)
        os.replace(temporary, target)
        directory_fd = os.open(
            root,
            os.O_RDONLY
            | os.O_CLOEXEC
            | getattr(os, "O_DIRECTORY", 0)
            | getattr(os, "O_NOFOLLOW", 0),
        )
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
        return {
            "recorded": True,
            "retained": False,
            "idempotent": False,
            "run_id": run_id,
            "terminal_status": status,
        }
    finally:
        try:
            temporary.unlink(missing_ok=True)
        except UnboundLocalError:
            pass
        fcntl.flock(lock_fd, fcntl.LOCK_UN)
        os.close(lock_fd)


class ActiveRunGCRoots:
    """Create and remove only this run's immutable Nix GC-root symlinks."""

    def __init__(
        self,
        directory: pathlib.Path,
        names: Sequence[str],
        *,
        bead_id: str | None = None,
        generation: str | None = None,
        state_schema: str | None = None,
        terminal_state_root: pathlib.Path | None = None,
        allowed_prefixes: Sequence[str] = ("/nix/store/",),
    ):
        self.directory = directory
        self.names = tuple(names)
        self.bead_id = bead_id
        self.generation = generation
        self.state_schema = state_schema
        self.terminal_state_root = terminal_state_root
        self.allowed_prefixes = tuple(allowed_prefixes)
        self.terminal = False

    @staticmethod
    def _validate_root_directory(path: pathlib.Path) -> pathlib.Path:
        if not path.is_absolute() or any(part == ".." for part in path.parts):
            raise RootLifecycleError("GC-root directory must be absolute and normalized")
        if path.exists() and (path.is_symlink() or not path.is_dir()):
            raise RootLifecycleError("GC-root directory is not a directory")
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = os.lstat(path)
        if (
            stat.S_ISLNK(info.st_mode)
            or not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.geteuid()
            or info.st_mode & 0o077
        ):
            raise RootLifecycleError("GC-root directory is not service-owned")
        return path

    @staticmethod
    def _validate_existing_directory(path: pathlib.Path) -> pathlib.Path:
        if not path.is_absolute() or any(part == ".." for part in path.parts):
            raise RootLifecycleError("GC-root directory must be absolute and normalized")
        try:
            info = os.lstat(path)
        except OSError as error:
            raise RootLifecycleError("active-run GC-root directory is unavailable") from error
        if (
            stat.S_ISLNK(info.st_mode)
            or not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.geteuid()
            or info.st_mode & 0o077
        ):
            raise RootLifecycleError("active-run GC-root directory is not service-owned")
        return path

    @staticmethod
    def _target(
        value: str | os.PathLike[str],
        *,
        name: str,
        allowed_prefixes: Sequence[str],
    ) -> pathlib.Path:
        if not isinstance(value, (str, os.PathLike)):
            raise RootLifecycleError(f"GC target is malformed for {name}")
        target = pathlib.Path(value)
        target_text = str(target)
        if (
            not target.is_absolute()
            or any(part == ".." for part in target.parts)
            or os.path.normpath(target_text) != target_text
        ):
            raise RootLifecycleError(f"GC target is malformed: {target}")
        try:
            approved = any(
                os.path.commonpath((target_text, str(prefix))) == str(prefix).rstrip("/")
                for prefix in allowed_prefixes
            )
        except ValueError:
            approved = False
        if not approved:
            raise RootLifecycleError(f"GC target is outside the approved store: {target}")
        if not target.exists():
            raise RootLifecycleError(f"GC target does not exist: {target}")
        return target

    @staticmethod
    def _metadata_path(directory: pathlib.Path) -> pathlib.Path:
        return directory / "metadata.json"

    @classmethod
    def _read_existing(
        cls,
        directory: pathlib.Path,
        *,
        run_id: str,
        bead_id: str | None,
        generation: str,
        state_schema: str,
        terminal_state_root: pathlib.Path | None,
        allowed_prefixes: Sequence[str],
    ) -> "ActiveRunGCRoots":
        cls._validate_existing_directory(directory)
        metadata_path = cls._metadata_path(directory)
        try:
            with metadata_path.open("r", encoding="utf-8") as stream:
                metadata = json.load(stream)
        except (OSError, json.JSONDecodeError) as error:
            raise RootLifecycleError("active-run GC-root metadata is unreadable") from error
        try:
            metadata_info = os.lstat(metadata_path)
        except OSError as error:
            raise RootLifecycleError("active-run GC-root metadata is unavailable") from error
        if (
            not stat.S_ISREG(metadata_info.st_mode)
            or metadata_info.st_uid != os.geteuid()
            or metadata_info.st_mode & 0o077
        ):
            raise RootLifecycleError("active-run GC-root metadata is not service-owned")
        if (
            not isinstance(metadata, dict)
            or set(metadata)
            != {
                "schema",
                "run_id",
                "bead_id",
                "generation",
                "state_schema",
                "targets",
            }
            or metadata.get("schema") != GC_ROOT_METADATA_SCHEMA
            or metadata.get("run_id") != run_id
            or (bead_id is not None and metadata.get("bead_id") != bead_id)
            or metadata.get("generation") != generation
            or metadata.get("state_schema") != state_schema
            or not isinstance(metadata.get("targets"), dict)
            or set(metadata["targets"]) != set(GC_ROOT_NAMES)
        ):
            raise RootLifecycleError("active-run GC roots belong to an incompatible generation")
        try:
            entries = {entry.name for entry in directory.iterdir()}
        except OSError as error:
            raise RootLifecycleError("active-run GC-root directory is unreadable") from error
        if entries != set(GC_ROOT_NAMES) | {"metadata.json"}:
            raise RootLifecycleError("active-run GC-root directory has an incompatible shape")
        targets = {
            name: cls._target(
                metadata["targets"][name],
                name=name,
                allowed_prefixes=allowed_prefixes,
            )
            for name in GC_ROOT_NAMES
        }
        for name, target in targets.items():
            link = directory / name
            try:
                if not link.is_symlink() or os.readlink(link) != str(target):
                    raise RootLifecycleError(f"active-run GC root is not pinned: {link}")
            except OSError as error:
                raise RootLifecycleError(f"active-run GC root is unavailable: {link}") from error
        return cls(
            directory,
            GC_ROOT_NAMES,
            bead_id=str(metadata["bead_id"]),
            generation=generation,
            state_schema=state_schema,
            terminal_state_root=terminal_state_root,
            allowed_prefixes=allowed_prefixes,
        )

    @staticmethod
    def _write_metadata(
        path: pathlib.Path,
        *,
        run_id: str,
        bead_id: str,
        generation: str,
        state_schema: str,
        targets: Mapping[str, pathlib.Path],
    ) -> None:
        metadata = {
            "schema": GC_ROOT_METADATA_SCHEMA,
            "run_id": run_id,
            "bead_id": bead_id,
            "generation": generation,
            "state_schema": state_schema,
            "targets": {name: str(targets[name]) for name in GC_ROOT_NAMES},
        }
        temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
        try:
            with temporary.open("x", encoding="utf-8") as stream:
                json.dump(metadata, stream, sort_keys=True, separators=(",", ":"))
                stream.write("\n")
                stream.flush()
                os.fsync(stream.fileno())
            os.chmod(temporary, 0o600)
            os.replace(temporary, path)
            directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            temporary.unlink(missing_ok=True)

    @classmethod
    def create(
        cls,
        root_directory: str | os.PathLike[str],
        *,
        run_id: str,
        bead_id: str | None = None,
        generation_paths: Mapping[str, str | os.PathLike[str]],
        allowed_prefixes: Sequence[str] = ("/nix/store/",),
        generation: str | None = None,
        state_schema: str | None = None,
        terminal_state_path: str | os.PathLike[str] | None = None,
    ) -> "ActiveRunGCRoots":
        _identifier(run_id, "run id")
        bead_id = bead_id or run_id
        _identifier(bead_id, "bead id")
        if not generation or not state_schema:
            raise RootLifecycleError("GC-root generation and state schema are required")
        if set(generation_paths) != set(GC_ROOT_NAMES):
            raise RootLifecycleError("active-run GC roots have an incomplete shape")
        terminal_state_root = None
        if terminal_state_path is not None:
            terminal_state_root = _terminal_state_root_from_path(
                terminal_state_path,
                run_id=run_id,
            )
        root = cls._validate_root_directory(pathlib.Path(root_directory))
        directory = root / run_id
        lock_path = root / ".lock"
        lock_fd = os.open(
            lock_path,
            os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        created: list[pathlib.Path] = []
        created_directory = False
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_EX)
            if directory.exists():
                if directory.is_symlink() or not directory.is_dir():
                    raise RootLifecycleError("active-run GC-root path is not a directory")
                return cls._read_existing(
                    directory,
                    run_id=run_id,
                    bead_id=bead_id,
                    generation=generation,
                    state_schema=state_schema,
                    terminal_state_root=terminal_state_root,
                    allowed_prefixes=allowed_prefixes,
                )
            directory.mkdir(mode=0o700)
            created_directory = True
            directory_stat = os.lstat(directory)
            if (
                directory_stat.st_uid != os.geteuid()
                or directory_stat.st_mode & 0o077
            ):
                raise RootLifecycleError("GC-root directory is not service-owned")
            targets = {
                name: cls._target(
                    generation_paths[name],
                    name=name,
                    allowed_prefixes=allowed_prefixes,
                )
                for name in GC_ROOT_NAMES
            }
            for name in GC_ROOT_NAMES:
                link = directory / name
                os.symlink(str(targets[name]), link)
                created.append(link)
            cls._write_metadata(
                cls._metadata_path(directory),
                run_id=run_id,
                bead_id=bead_id,
                generation=generation,
                state_schema=state_schema,
                targets=targets,
            )
        except FileExistsError:
            return cls._read_existing(
                directory,
                run_id=run_id,
                bead_id=bead_id,
                generation=generation,
                state_schema=state_schema,
                terminal_state_root=terminal_state_root,
                allowed_prefixes=allowed_prefixes,
            )
        except (OSError, RootLifecycleError):
            if created_directory:
                for link in reversed(created):
                    link.unlink(missing_ok=True)
                cls._metadata_path(directory).unlink(missing_ok=True)
                directory.rmdir()
            raise
        finally:
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
            os.close(lock_fd)
        return cls(
            directory,
            GC_ROOT_NAMES,
            bead_id=bead_id,
            generation=generation,
            state_schema=state_schema,
            terminal_state_root=terminal_state_root,
            allowed_prefixes=allowed_prefixes,
        )

    def _read_terminal_state(
        self,
        state_path: str | os.PathLike[str],
        *,
        run_id: str,
        bead_id: str | None,
        generation: str,
        state_schema: str,
    ) -> dict[str, object]:
        path = pathlib.Path(state_path)
        expected_root = self.terminal_state_root
        if expected_root is None:
            expected_root = self.directory.parent.parent / "terminal"
        else:
            supplied_root = _terminal_state_root_from_path(
                state_path,
                run_id=self.directory.name,
            )
            if supplied_root != expected_root:
                raise RootLifecycleError(
                    "terminal workflow state path is not run-bound"
                )
        if not path.is_absolute() or any(part == ".." for part in path.parts):
            raise RootLifecycleError("terminal workflow state path is malformed")
        if path != expected_root / f"{run_id}.json":
            raise RootLifecycleError("terminal workflow state path is not run-bound")
        _validate_terminal_root(expected_root, create=False)
        return _read_terminal_record(
            path,
            run_id=run_id,
            bead_id=bead_id,
            generation=generation,
            state_schema=state_schema,
        )

    def cleanup(
        self,
        *,
        state_path: str | os.PathLike[str] | None = None,
        terminal: bool = False,
    ) -> None:
        if self.terminal:
            return
        if terminal or state_path is None:
            raise RootLifecycleError("durable terminal workflow state is required")
        lock_path = self.directory.parent / ".lock"
        try:
            lock_fd = os.open(
                lock_path,
                os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0),
                0o600,
            )
        except OSError as error:
            raise RootLifecycleError("active-run GC-root lock is unavailable") from error
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_EX)
            if self.directory.is_symlink():
                raise RootLifecycleError("active-run GC-root directory is an unsafe symlink")
            if not self.directory.exists():
                self._read_terminal_state(
                    state_path,
                    run_id=self.directory.name,
                    bead_id=self.bead_id,
                    generation=self.generation or "",
                    state_schema=self.state_schema or "",
                )
                self.terminal = True
                return
            self._read_terminal_state(
                state_path,
                run_id=self.directory.name,
                bead_id=self.bead_id,
                generation=self.generation or "",
                state_schema=self.state_schema or "",
            )
            self._read_existing(
                self.directory,
                run_id=self.directory.name,
                bead_id=self.bead_id,
                generation=self.generation or "",
                state_schema=self.state_schema or "",
                terminal_state_root=self.terminal_state_root,
                allowed_prefixes=self.allowed_prefixes,
            )
            for name in self.names:
                link = self.directory / name
                if link.is_symlink():
                    link.unlink()
                elif link.exists():
                    raise RootLifecycleError(f"GC-root path is not a symlink: {link}")
            self._metadata_path(self.directory).unlink()
            self.directory.rmdir()
            self.terminal = True
        finally:
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
            os.close(lock_fd)


def _probe_environment(proxy_fd: int | None = None) -> dict[str, str]:
    result: dict[str, str] = {}
    for name, value in os.environ.items():
        if name == "COPILOT_GITHUB_TOKEN" or name in {
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "PATH",
            "SSL_CERT_FILE",
            "TMPDIR",
            "XDG_RUNTIME_DIR",
            "GC_AGENT_LAUNCHER_SOCKET",
            "GC_AGENT_LAUNCHER_TOKEN",
        }:
            result[name] = value
    if proxy_fd is not None:
        if proxy_fd < 3:
            raise ActivationError("probe proxy fd must not overlap standard descriptors")
        result["GC_PROXY_FD"] = str(proxy_fd)
        os.set_inheritable(proxy_fd, True)
    return result


def run_profile_probe(
    profile_script: str,
    *,
    profile: str,
    generation: str,
    state_schema: str,
    run_id: str,
    bead_id: str,
    worktree: str,
    lease_root: str,
    runtime_root: str,
    timeout: float,
    proxy_fd: int | None = None,
) -> ProbeResult:
    tool_policy = "coding" if profile == "code-luna" else "review"
    command = [
        sys.executable,
        profile_script,
        "--profile",
        profile,
        "--tool-policy",
        tool_policy,
        "--probe",
        "--generation",
        generation,
        "--state-schema",
        state_schema,
        "--run-id",
        run_id,
        "--bead-id",
        bead_id,
        "--worktree",
        worktree,
        "--lease-root",
        lease_root,
        "--runtime-root",
        runtime_root,
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=worktree,
            env=_probe_environment(proxy_fd),
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
            pass_fds=(proxy_fd,) if proxy_fd is not None else (),
        )
    except subprocess.TimeoutExpired:
        return ProbeResult(profile=profile, ok=False, error_code="network")
    except OSError as error:
        return ProbeResult(
            profile=profile,
            ok=False,
            error_code=classify_failure(str(error)),
            error=str(error)[:512],
        )
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError:
        value = {
            "profile": profile,
            "ok": False,
            "error_code": "malformed",
            "error": completed.stderr[-512:],
        }
    return parse_probe(profile, value)


def activate(
    *,
    status_path: str | os.PathLike[str],
    generation: str,
    state_schema: str,
    probe: Callable[[str], ProbeResult | Mapping[str, object]],
) -> dict[str, object]:
    if not generation or not state_schema:
        raise ActivationError("generation and state schema are required")
    status = select_profiles(
        probe,
        generation=generation,
        state_schema=state_schema,
    )
    write_status(status_path, status)
    return status


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate_parser = subparsers.add_parser("validate-paths")
    validate_parser.add_argument("--credential", action="append", default=[])
    validate_parser.add_argument("--projection", action="append", default=[])
    validate_parser.add_argument("--project-root", required=True)
    validate_parser.add_argument("--require-project-quota", action="store_true")

    reserve_parser = subparsers.add_parser("check-free-space")
    reserve_parser.add_argument("--path", required=True)
    reserve_parser.add_argument("--reserve-bytes", type=int, required=True)

    materialize_parser = subparsers.add_parser("materialize-assets")
    materialize_parser.add_argument("--source", required=True)
    materialize_parser.add_argument("--destination", required=True)

    monitor_parser = subparsers.add_parser("free-space-monitor")
    monitor_parser.add_argument("--path", required=True)
    monitor_parser.add_argument("--reserve-bytes", type=int, required=True)
    monitor_parser.add_argument("--status-path", required=True)
    monitor_parser.add_argument("--generation", required=True)
    monitor_parser.add_argument("--state-schema", required=True)
    monitor_parser.add_argument("--interval", type=float, default=30.0)
    monitor_parser.add_argument("--once", action="store_true")

    egress_parser = subparsers.add_parser("egress-peer")
    egress_parser.add_argument("--socket", required=True)
    egress_parser.add_argument("--socket-group", required=True)
    egress_parser.add_argument("--auth-token-env", default="GC_FDPROXY_AUTH")
    egress_parser.add_argument("--allowed-domain", action="append", default=[])
    egress_parser.add_argument("--allowed-port", type=int, default=DEFAULT_ALLOWED_PORT)
    egress_parser.add_argument("--allowed-uid", action="append", type=int, default=[])

    relay_parser = subparsers.add_parser("agent-relay")
    relay_parser.add_argument("--public-socket", required=True)
    relay_parser.add_argument("--private-socket", required=True)
    relay_parser.add_argument("--socket-group", required=True)
    relay_parser.add_argument("--allowed-uid", type=int, required=True)

    fdproxy_parser = subparsers.add_parser("fdproxy-sidecar")
    fdproxy_parser.add_argument("--egress-socket", required=True)
    fdproxy_parser.add_argument("--fdproxy", required=True)
    fdproxy_parser.add_argument("--listen", default="127.0.0.1:3128")
    fdproxy_parser.add_argument("--server-uid", type=int, required=True)
    fdproxy_parser.add_argument("wrapped", nargs=argparse.REMAINDER)

    activate_parser = subparsers.add_parser("activate")
    activate_parser.add_argument("--status-path", required=True)
    activate_parser.add_argument("--generation", required=True)
    activate_parser.add_argument("--state-schema", required=True)
    activate_parser.add_argument("--profile-script", required=True)
    activate_parser.add_argument("--run-id", required=True)
    activate_parser.add_argument("--bead-id", required=True)
    activate_parser.add_argument("--worktree", required=True)
    activate_parser.add_argument("--lease-root", required=True)
    activate_parser.add_argument("--runtime-root", required=True)
    activate_parser.add_argument("--egress-socket")
    activate_parser.add_argument("--egress-server-uid", type=int)
    activate_parser.add_argument("--timeout", type=float, default=20.0)

    prompt_parser = subparsers.add_parser("reconstruct-prompt")
    prompt_parser.add_argument("--context", required=True)
    prompt_parser.add_argument("--generation", required=True)
    prompt_parser.add_argument("--state-schema", required=True)

    terminal_parser = subparsers.add_parser("write-terminal-state")
    terminal_parser.add_argument("--terminal-state-root", required=True)
    terminal_parser.add_argument("--run-id", required=True)
    terminal_parser.add_argument("--bead-id", required=True)
    terminal_parser.add_argument("--generation", required=True)
    terminal_parser.add_argument("--state-schema", required=True)
    terminal_parser.add_argument("--bd-path", default="bd")
    terminal_parser.add_argument("--cancellation-root")

    roots_parser = subparsers.add_parser("gc-root-cleanup")
    roots_parser.add_argument("--root-directory", required=True)
    roots_parser.add_argument("--run-id", required=True)
    roots_parser.add_argument("--bead-id")
    roots_parser.add_argument("--generation", required=True)
    roots_parser.add_argument("--state-schema", required=True)
    roots_parser.add_argument("--state-path", required=True)
    roots_parser.add_argument("--allowed-prefix", action="append", default=["/nix/store/"])
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    if args.command == "validate-paths":
        for index, path in enumerate(args.credential):
            validate_credential_source(path, label=f"credential[{index}]")
        for index, path in enumerate(args.projection):
            validate_host_projection(path, label=f"projection[{index}]")
        if args.require_project_quota:
            require_project_quota(args.project_root)
        return 0
    if args.command == "check-free-space":
        print(check_free_space(args.path, args.reserve_bytes))
        return 0
    if args.command == "materialize-assets":
        materialize_assets(args.source, args.destination)
        return 0
    if args.command == "free-space-monitor":
        while True:
            try:
                monitor_free_space_once(
                    path=args.path,
                    reserve_bytes=args.reserve_bytes,
                    status_path=args.status_path,
                    generation=args.generation,
                    state_schema=args.state_schema,
                )
            except BoundaryError as error:
                print(str(error), file=sys.stderr)
                return 1
            if args.once:
                return 0
            time.sleep(max(0.1, args.interval))
    if args.command == "egress-peer":
        if args.allowed_port != DEFAULT_ALLOWED_PORT:
            raise BoundaryError("only HTTPS egress on port 443 is supported")
        token = os.environ.get(args.auth_token_env, "")
        serve_egress_peer(
            socket_path=args.socket,
            socket_group=args.socket_group,
            auth_token=token,
            allowed_domains=args.allowed_domain,
            allowed_uids=args.allowed_uid,
        )
        return 0
    if args.command == "agent-relay":
        serve_agent_relay(
            public_socket=args.public_socket,
            private_socket=args.private_socket,
            socket_group=args.socket_group,
            allowed_uid=args.allowed_uid,
        )
        return 0
    if args.command == "fdproxy-sidecar":
        command = list(args.wrapped)
        if command[:1] == ["--"]:
            command = command[1:]
        return run_fdproxy_sidecar(
            egress_socket=args.egress_socket,
            fdproxy_script=args.fdproxy,
            listen=args.listen,
            command=command,
            server_uid=args.server_uid,
        )
    if args.command == "activate":
        def probe(profile: str) -> ProbeResult:
            proxy_channel: socket.socket | None = None
            try:
                if args.egress_socket:
                    proxy_channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                    proxy_channel.connect(args.egress_socket)
                    if (
                        args.egress_server_uid is not None
                        and _peer_uid(proxy_channel) != args.egress_server_uid
                    ):
                        raise ActivationError("egress server uid is unauthorized")
                return run_profile_probe(
                    args.profile_script,
                    profile=profile,
                    generation=args.generation,
                    state_schema=args.state_schema,
                    run_id=args.run_id,
                    bead_id=args.bead_id,
                    worktree=args.worktree,
                    lease_root=args.lease_root,
                    runtime_root=args.runtime_root,
                    timeout=args.timeout,
                    proxy_fd=proxy_channel.fileno() if proxy_channel is not None else None,
                )
            finally:
                if proxy_channel is not None:
                    proxy_channel.close()

        status = activate(
            status_path=args.status_path,
            generation=args.generation,
            state_schema=args.state_schema,
            probe=probe,
        )
        json.dump(status, sys.stdout, separators=(",", ":"))
        sys.stdout.write("\n")
        return 0 if status["ready"] is True else 1
    if args.command == "reconstruct-prompt":
        try:
            with pathlib.Path(args.context).open("r", encoding="utf-8") as stream:
                context = json.load(stream)
        except (OSError, json.JSONDecodeError) as error:
            raise ActivationError("context is unreadable") from error
        print(
            reconstruct_prompt(
                context,
                generation=args.generation,
                state_schema=args.state_schema,
            )
        )
        return 0
    if args.command == "write-terminal-state":
        result = record_terminal_state(
            args.terminal_state_root,
            run_id=args.run_id,
            bead_id=args.bead_id,
            generation=args.generation,
            state_schema=args.state_schema,
            bd_path=args.bd_path,
            cancellation_root=args.cancellation_root,
        )
        json.dump(result, sys.stdout, sort_keys=True, separators=(",", ":"))
        sys.stdout.write("\n")
        return 0
    if args.command == "gc-root-cleanup":
        _identifier(args.run_id, "run id")
        root_directory = pathlib.Path(args.root_directory)
        if not root_directory.is_absolute() or any(
            part == ".." for part in root_directory.parts
        ):
            raise RootLifecycleError("GC-root directory must be absolute and normalized")
        directory = root_directory / args.run_id
        if directory.exists() and directory.is_symlink():
            raise RootLifecycleError("active-run GC-root path is an unsafe symlink")
        if directory.exists():
            roots = ActiveRunGCRoots._read_existing(
                directory,
                run_id=args.run_id,
                bead_id=args.bead_id,
                generation=args.generation,
                state_schema=args.state_schema,
                terminal_state_root=_terminal_state_root_from_path(
                    args.state_path,
                    run_id=args.run_id,
                ),
                allowed_prefixes=tuple(args.allowed_prefix),
            )
        else:
            roots = ActiveRunGCRoots(
                directory,
                GC_ROOT_NAMES,
                bead_id=args.bead_id,
                generation=args.generation,
                state_schema=args.state_schema,
                terminal_state_root=_terminal_state_root_from_path(
                    args.state_path,
                    run_id=args.run_id,
                ),
                allowed_prefixes=tuple(args.allowed_prefix),
            )
        roots.cleanup(state_path=args.state_path)
        return 0
    raise ActivationError("unknown activation command")


if __name__ == "__main__":
    raise SystemExit(main())
