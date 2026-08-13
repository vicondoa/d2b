#!/usr/bin/env python3
"""Own one authenticated ACP child, its namespace, and its concurrency lease."""

from __future__ import annotations

import argparse
import array
import ctypes
import errno
import fcntl
import hmac
import importlib.util
import json
import os
import pathlib
import re
import selectors
import signal
import shutil
import socket
import stat
import struct
import subprocess
import sys
import threading
import time
from collections.abc import Callable, Mapping, Sequence
from dataclasses import dataclass
from types import ModuleType


class LauncherError(RuntimeError):
    """Raised when a launch cannot satisfy an ownership invariant."""


class LeaseBusy(LauncherError):
    """Raised when an agent or active-run slot is already occupied."""


class StaleReadiness(LauncherError):
    """Raised when a launcher is not authorized by the current readiness row."""


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
PROFILE_NAMES = frozenset(PROFILE_SETTINGS)
TOOL_POLICIES = {
    "review": "view,search",
    "planning": "view,search,apply_patch",
    "coding": "bash,view,search,apply_patch",
}
LAUNCHER_PROTOCOL = "gascity-agent/1"
MAX_LAUNCH_METADATA_BYTES = 16 * 1024
MAX_LAUNCH_RESPONSE_BYTES = 8 * 1024
MAX_RELAY_BUFFER_BYTES = 1024 * 1024
LAUNCHER_FD_NAMES = ("proxy", "progress", "control", "check")
GC_ROOT_NAMES = ("package", "city", "pack", "profiles", "instructions")
IDENTIFIER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")
FORBIDDEN_CHILD_FLAGS = frozenset(
    {
        "--reasoning-effort",
        "--reasoning_summary",
        "--reasoning-summary",
    }
)
PROFILE_VALUE_FLAGS = {
    "--model": "model",
    "--context": "contextTier",
    "--effort": "effort",
}
SECRET_ENV_NAMES = frozenset(
    {
        "AWS_ACCESS_KEY_ID",
        "AWS_SECRET_ACCESS_KEY",
        "BUILD_BUDDY_API_KEY",
        "DISCORD_TOKEN",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "NIX_CONFIG",
        "SSH_AUTH_SOCK",
    }
)
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


def _validate_identifier(value: str, label: str) -> str:
    if not IDENTIFIER_PATTERN.fullmatch(value) or ".." in value:
        raise LauncherError(f"{label} is malformed")
    return value


def _absolute_existing(path: str | os.PathLike[str], label: str) -> pathlib.Path:
    value = pathlib.Path(path)
    if not value.is_absolute() or any(part == ".." for part in value.parts):
        raise LauncherError(f"{label} must be an absolute normalized path")
    if not value.exists():
        raise LauncherError(f"{label} does not exist: {value}")
    return value.resolve()


def _private_directory(path: str | os.PathLike[str], label: str) -> pathlib.Path:
    value = pathlib.Path(path)
    if not value.is_absolute() or any(part == ".." for part in value.parts):
        raise LauncherError(f"{label} must be an absolute normalized path")
    if value.is_symlink():
        raise LauncherError(f"{label} must not be a symlink")
    value.mkdir(mode=0o700, parents=True, exist_ok=True)
    stat_result = value.stat()
    if stat_result.st_uid != os.geteuid() or stat_result.st_mode & 0o077:
        raise LauncherError(
            f"{label} is not a private service-owned directory "
            f"(owner={stat_result.st_uid}, expected={os.geteuid()}, "
            f"mode={stat.S_IMODE(stat_result.st_mode):04o})"
        )
    return value


def _parse_positive(value: str | int, label: str) -> int:
    try:
        number = int(value)
    except (TypeError, ValueError) as error:
        raise LauncherError(f"{label} must be a positive integer") from error
    if number < 1:
        raise LauncherError(f"{label} must be a positive integer")
    return number


def _load_activation_module(path: str | os.PathLike[str]) -> ModuleType:
    activation_path = _absolute_existing(path, "service activation script")
    specification = importlib.util.spec_from_file_location(
        "gascity_service_activation",
        activation_path,
    )
    if specification is None or specification.loader is None:
        raise LauncherError("service activation module could not be loaded")
    module = importlib.util.module_from_spec(specification)
    sys.modules[specification.name] = module
    specification.loader.exec_module(module)
    return module


def _create_active_run_roots(
    args: argparse.Namespace,
    *,
    run_id: str,
    bead_id: str,
    generation: str,
    state_schema: str,
    root_bead_id: str | None = None,
    terminal_state_path: str | None = None,
):
    if not args.gc_root_directory or run_id == "readiness":
        return None
    required = {
        "activation_script": args.activation_script,
        "package_path": args.package_path,
        "city_path": args.city_path,
        "pack_path": args.pack_path,
        "profiles_path": args.profiles_path,
        "instructions_path": args.instructions_path,
    }
    if any(value is None for value in required.values()):
        raise LauncherError("active-run GC-root configuration is incomplete")
    activation = _load_activation_module(args.activation_script)
    try:
        root_class = activation.ActiveRunGCRoots
        root_names = activation.GC_ROOT_NAMES
    except AttributeError as error:
        raise LauncherError("service activation lacks active-run GC-root support") from error
    if tuple(root_names) != GC_ROOT_NAMES:
        raise LauncherError("active-run GC-root shape is incompatible")
    prefixes = tuple(args.gc_root_prefix or ["/nix/store/"])
    root_directory = pathlib.Path(args.gc_root_directory)
    run_directory = root_directory / run_id
    if terminal_state_path is None:
        terminal_root = root_directory.parent / "terminal"
        terminal_state = terminal_root / f"{run_id}.json"
    else:
        terminal_state = pathlib.Path(terminal_state_path)
    if (
        not run_directory.exists()
        and not run_directory.is_symlink()
        and terminal_state.exists()
    ):
        activation._terminal_state_root_from_path(
            terminal_state,
            run_id=run_id,
        )
        activation._validate_terminal_root(terminal_state.parent, create=False)
        activation._read_terminal_record(
            terminal_state,
            run_id=run_id,
            bead_id=bead_id,
            generation=generation,
            state_schema=state_schema,
        )
        return None
    return root_class.create(
        args.gc_root_directory,
        run_id=run_id,
        bead_id=bead_id,
        generation_paths={
            "package": args.package_path,
            "city": args.city_path,
            "pack": args.pack_path,
            "profiles": args.profiles_path,
            "instructions": args.instructions_path,
        },
        allowed_prefixes=prefixes,
        generation=generation,
        state_schema=state_schema,
        terminal_state_path=terminal_state,
    )


def _cleanup_active_run_roots(active_roots, terminal_state_path: str) -> None:
    try:
        active_roots.cleanup(state_path=terminal_state_path)
    except (OSError, RuntimeError) as error:
        print(
            f"active-run GC-root cleanup deferred: {error}",
            file=sys.stderr,
        )


def _validate_gc_root_configuration(args: argparse.Namespace) -> None:
    values = (
        args.gc_root_directory,
        args.activation_script,
        args.package_path,
        args.city_path,
        args.pack_path,
        args.profiles_path,
        args.instructions_path,
    )
    if any(value is not None for value in values) and not all(value is not None for value in values):
        raise LauncherError("active-run GC-root configuration is incomplete")


class _ActiveRunMembership:
    """Reference-count one active-run slot while each member holds a shared lock."""

    def __init__(
        self,
        *,
        lease_root: pathlib.Path,
        slot_path: pathlib.Path,
        slot_index: int,
        run_id: str,
        slot_fd: int,
    ):
        self.lease_root = lease_root
        self.slot_path = slot_path
        self.slot_index = slot_index
        self.run_id = run_id
        self.slot_fd = slot_fd
        self.released = False

    @staticmethod
    def _open(path: pathlib.Path) -> int:
        return os.open(
            path,
            os.O_RDWR
            | os.O_CREAT
            | os.O_CLOEXEC
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )

    @staticmethod
    def _try_exclusive(descriptor: int) -> bool:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return False
        return True

    @staticmethod
    def _unlock_close(descriptor: int) -> None:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)

    @classmethod
    def _registry_lock(cls, path: pathlib.Path) -> int:
        descriptor = cls._open(path)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX)
        except OSError:
            os.close(descriptor)
            raise
        return descriptor

    @staticmethod
    def _read_registry(path: pathlib.Path) -> dict[str, dict[str, object]]:
        if not path.exists():
            return {}
        try:
            with path.open("r", encoding="utf-8") as stream:
                value = json.load(stream)
        except (OSError, json.JSONDecodeError) as error:
            raise LauncherError("active-run registry is unreadable or malformed") from error
        if not isinstance(value, dict) or value.get("version") != 1:
            raise LauncherError("active-run registry version is unsupported")
        slots = value.get("slots")
        if not isinstance(slots, dict):
            raise LauncherError("active-run registry slots are malformed")
        result: dict[str, dict[str, object]] = {}
        for key, record in slots.items():
            if not isinstance(key, str) or not key.isdigit():
                raise LauncherError("active-run registry slot is malformed")
            if (
                not isinstance(record, dict)
                or not isinstance(record.get("run_id"), str)
                or not isinstance(record.get("refcount"), int)
                or record["refcount"] < 1
            ):
                raise LauncherError("active-run registry entry is malformed")
            _validate_identifier(str(record["run_id"]), "run id")
            result[key] = {
                "run_id": str(record["run_id"]),
                "refcount": int(record["refcount"]),
            }
        return result

    @staticmethod
    def _write_registry(
        path: pathlib.Path,
        slots: Mapping[str, Mapping[str, object]],
    ) -> None:
        temporary = path.with_name(
            f".{path.name}.{os.getpid()}.{threading.get_ident()}.tmp"
        )
        payload = {
            "version": 1,
            "slots": {
                str(key): {
                    "run_id": str(value["run_id"]),
                    "refcount": int(value["refcount"]),
                }
                for key, value in slots.items()
            },
        }
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
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                descriptor = -1
                json.dump(payload, stream, sort_keys=True, separators=(",", ":"))
                stream.write("\n")
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            if descriptor >= 0:
                os.close(descriptor)
            try:
                temporary.unlink()
            except FileNotFoundError:
                pass

    @classmethod
    def acquire(
        cls,
        lease_root: pathlib.Path,
        runs: pathlib.Path,
        *,
        run_id: str,
        max_active_runs: int,
    ) -> "_ActiveRunMembership":
        registry = lease_root / "active-runs.json"
        registry_lock = lease_root / "active-runs.registry.lock"
        lock_fd = cls._registry_lock(registry_lock)
        try:
            slots = cls._read_registry(registry)
            existing_key = next(
                (
                    key
                    for key, record in slots.items()
                    if record.get("run_id") == run_id
                ),
                None,
            )
            if existing_key is not None:
                slot_index = int(existing_key)
                slot_path = runs / f"active-run-{slot_index:04d}.lock"
                slot_fd = cls._open(slot_path)
                try:
                    if cls._try_exclusive(slot_fd):
                        fcntl.flock(slot_fd, fcntl.LOCK_SH)
                        refcount = 1
                    else:
                        fcntl.flock(
                            slot_fd,
                            fcntl.LOCK_SH | fcntl.LOCK_NB,
                        )
                        refcount = int(slots[existing_key]["refcount"]) + 1
                    slots[existing_key] = {
                        "run_id": run_id,
                        "refcount": refcount,
                    }
                    cls._write_registry(registry, slots)
                    return cls(
                        lease_root=lease_root,
                        slot_path=slot_path,
                        slot_index=slot_index,
                        run_id=run_id,
                        slot_fd=slot_fd,
                    )
                except BaseException:
                    cls._unlock_close(slot_fd)
                    raise

            for key in tuple(slots):
                slot_index = int(key)
                if slot_index >= max_active_runs:
                    continue
                slot_path = runs / f"active-run-{slot_index:04d}.lock"
                slot_fd = cls._open(slot_path)
                if cls._try_exclusive(slot_fd):
                    cls._unlock_close(slot_fd)
                    del slots[key]
                else:
                    os.close(slot_fd)

            for slot_index in range(max_active_runs):
                key = str(slot_index)
                if key in slots:
                    continue
                slot_path = runs / f"active-run-{slot_index:04d}.lock"
                slot_fd = cls._open(slot_path)
                try:
                    if not cls._try_exclusive(slot_fd):
                        os.close(slot_fd)
                        continue
                    fcntl.flock(slot_fd, fcntl.LOCK_SH)
                    slots[key] = {"run_id": run_id, "refcount": 1}
                    cls._write_registry(registry, slots)
                    return cls(
                        lease_root=lease_root,
                        slot_path=slot_path,
                        slot_index=slot_index,
                        run_id=run_id,
                        slot_fd=slot_fd,
                    )
                except BaseException:
                    cls._unlock_close(slot_fd)
                    raise
            raise LeaseBusy(f"active-run concurrency cap ({max_active_runs}) is exhausted")
        finally:
            cls._unlock_close(lock_fd)

    def release(self) -> None:
        if self.released:
            return
        self.released = True
        registry = self.lease_root / "active-runs.json"
        registry_lock = self.lease_root / "active-runs.registry.lock"
        lock_fd = self._registry_lock(registry_lock)
        try:
            self._unlock_close(self.slot_fd)
            slots = self._read_registry(registry)
            key = str(self.slot_index)
            record = slots.get(key)
            if record is None or record.get("run_id") != self.run_id:
                return
            probe_fd = self._open(self.slot_path)
            try:
                if self._try_exclusive(probe_fd):
                    self._unlock_close(probe_fd)
                    del slots[key]
                else:
                    os.close(probe_fd)
                    record["refcount"] = max(1, int(record["refcount"]) - 1)
                self._write_registry(registry, slots)
            except BaseException:
                try:
                    os.close(probe_fd)
                except OSError:
                    pass
                raise
        finally:
            self._unlock_close(lock_fd)


class ConcurrencyLease:
    """A service-owned lifetime lease for one agent, bead, and active run."""

    def __init__(
        self,
        file_descriptors: Sequence[int],
        paths: Sequence[pathlib.Path],
        active_run: _ActiveRunMembership,
    ):
        self.file_descriptors = tuple(file_descriptors)
        self.paths = tuple(paths)
        self.active_run = active_run
        self.released = False

    @classmethod
    def acquire(
        cls,
        root: str | os.PathLike[str],
        *,
        run_id: str,
        bead_id: str | None = None,
        max_agents: int,
        max_active_runs: int,
    ) -> "ConcurrencyLease":
        _validate_identifier(run_id, "run id")
        if bead_id is not None:
            _validate_identifier(bead_id, "bead id")
        max_agents = _parse_positive(max_agents, "max agents")
        max_active_runs = _parse_positive(max_active_runs, "max active runs")
        lease_root = _private_directory(root, "lease root")
        agents = lease_root / "agents"
        runs = lease_root / "active-runs"
        beads = lease_root / "bead-locks"
        for directory in (agents, runs, beads):
            _private_directory(directory, "lease directory")
        descriptors: list[int] = []
        paths: list[pathlib.Path] = []
        active_run: _ActiveRunMembership | None = None
        try:
            agent_fd, agent_path = cls._claim_slot(agents, "agent", max_agents)
            descriptors.append(agent_fd)
            paths.append(agent_path)
            active_run = _ActiveRunMembership.acquire(
                lease_root,
                runs,
                run_id=run_id,
                max_active_runs=max_active_runs,
            )
            descriptors.append(active_run.slot_fd)
            paths.append(active_run.slot_path)
            bead_path = beads / f"{bead_id or run_id}.lock"
            bead_fd = cls._open_and_lock(bead_path)
            descriptors.append(bead_fd)
            paths.append(bead_path)
            return cls(descriptors, paths, active_run)
        except (OSError, LauncherError):
            if active_run is not None:
                active_run.release()
            for descriptor in reversed(descriptors):
                if active_run is not None and descriptor == active_run.slot_fd:
                    continue
                cls._unlock_close(descriptor)
            raise

    @staticmethod
    def _open_and_lock(path: pathlib.Path) -> int:
        descriptor = os.open(
            path,
            os.O_RDWR
            | os.O_CREAT
            | os.O_CLOEXEC
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            os.close(descriptor)
            raise LeaseBusy(f"run lease is already held: {path.name}") from error
        except OSError:
            os.close(descriptor)
            raise
        return descriptor

    @classmethod
    def _claim_slot(
        cls,
        directory: pathlib.Path,
        prefix: str,
        limit: int,
    ) -> tuple[int, pathlib.Path]:
        for index in range(limit):
            path = directory / f"{prefix}-{index:04d}.lock"
            try:
                return cls._open_and_lock(path), path
            except LeaseBusy:
                continue
        raise LeaseBusy(f"{prefix} concurrency cap ({limit}) is exhausted")

    @staticmethod
    def _unlock_close(descriptor: int) -> None:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)

    def release(self) -> None:
        if self.released:
            return
        self.released = True
        try:
            self.active_run.release()
        finally:
            for descriptor in reversed(self.file_descriptors):
                if descriptor == self.active_run.slot_fd:
                    continue
                self._unlock_close(descriptor)

    def __enter__(self) -> "ConcurrencyLease":
        return self

    def __exit__(self, _exc_type, _exc_value, _traceback) -> None:
        self.release()


def _load_settings(path: str | os.PathLike[str], profile: str) -> dict[str, str]:
    if profile not in PROFILE_NAMES:
        raise LauncherError(f"unknown Copilot profile: {profile}")
    settings_path = _absolute_existing(path, "profile settings")
    try:
        with settings_path.open("r", encoding="utf-8") as stream:
            settings = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise LauncherError("profile settings are unreadable or malformed") from error
    expected = PROFILE_SETTINGS[profile]
    if settings != expected:
        raise LauncherError(f"profile settings do not match {profile}")
    return dict(expected)


def materialize_copilot_home(
    settings_path: str | os.PathLike[str],
    *,
    profile: str,
    runtime_root: str | os.PathLike[str],
    run_id: str,
    bead_id: str | None = None,
) -> pathlib.Path:
    settings = _load_settings(settings_path, profile)
    _validate_identifier(run_id, "run id")
    if bead_id is not None:
        _validate_identifier(bead_id, "bead id")
    root = _private_directory(runtime_root, "agent runtime root")
    suffix = f".{bead_id}" if bead_id is not None else ""
    home = root / f"{run_id}{suffix}.copilot-home"
    if home.exists():
        raise LauncherError(f"Copilot home already exists for run: {run_id}")
    home.mkdir(mode=0o700)
    settings_file = home / "settings.json"
    with settings_file.open("x", encoding="utf-8") as stream:
        json.dump(settings, stream, sort_keys=True, separators=(",", ":"))
        stream.write("\n")
    os.chmod(settings_file, 0o600)
    return home


def scrub_environment(
    source: Mapping[str, str] | None = None,
    *,
    profile: str,
    run_id: str,
    bead_id: str,
    generation: str | None = None,
    state_schema: str | None = None,
) -> dict[str, str]:
    if profile not in PROFILE_NAMES:
        raise LauncherError(f"unknown Copilot profile: {profile}")
    _validate_identifier(run_id, "run id")
    _validate_identifier(bead_id, "bead id")
    root_bead_id = root_bead_id or run_id
    _validate_identifier(root_bead_id, "root bead id")
    source_environment = os.environ if source is None else source
    result: dict[str, str] = {}
    for name, value in source_environment.items():
        if name in SECRET_ENV_NAMES:
            continue
        if name == "COPILOT_GITHUB_TOKEN":
            result[name] = value
        elif name in {
            "ALL_PROXY",
            "HOME",
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
        }:
            result[name] = value
        elif name in ALLOWED_GC_ENV_NAMES:
            result[name] = value
    result.update(
        {
            "GC_PROFILE_NAME": profile,
            "GC_RUN_ID": run_id,
            "GC_BEAD_ID": bead_id,
            "GC_ROOT_BEAD_ID": root_bead_id,
        }
    )
    if generation is not None:
        result["GC_CITY_GENERATION"] = generation
    if state_schema is not None:
        result["GC_STATE_SCHEMA"] = state_schema
    return result


def validate_child_arguments(
    arguments: Sequence[str],
    *,
    profile: str | None = None,
) -> list[str]:
    values = list(arguments)
    observed: dict[str, list[str]] = {
        flag: [] for flag in PROFILE_VALUE_FLAGS
    }
    index = 0
    while index < len(values):
        argument = values[index]
        flag = argument.split("=", 1)[0]
        if flag in FORBIDDEN_CHILD_FLAGS:
            raise LauncherError(
                f"unsupported Copilot child flag: {flag}"
            )
        if flag in PROFILE_VALUE_FLAGS:
            if profile is None and flag != "--effort":
                raise LauncherError(f"{flag} requires a Copilot profile")
            if "=" in argument:
                value = argument.split("=", 1)[1]
            elif index + 1 >= len(values):
                raise LauncherError(f"Copilot {flag} has no value")
            else:
                value = values[index + 1]
                index += 1
            observed[flag].append(value)
        index += 1
    if profile is not None:
        if profile not in PROFILE_SETTINGS:
            raise LauncherError(f"unknown Copilot profile: {profile}")
        expected = {
            "--model": PROFILE_SETTINGS[profile]["model"],
            "--context": PROFILE_SETTINGS[profile]["contextTier"],
            "--effort": PROFILE_EFFORT[profile],
        }
        for flag, expected_value in expected.items():
            if observed[flag] and observed[flag] != [expected_value]:
                raise LauncherError(
                    f"Copilot {flag} does not match the immutable profile"
                )
            if not observed[flag]:
                values.extend([flag, expected_value])
    return values


def _pidfd_open(pid: int) -> int:
    pidfd_open = getattr(os, "pidfd_open", None)
    if pidfd_open is None:
        raise LauncherError("launcher requires pidfd support")
    try:
        return int(pidfd_open(pid, 0))
    except OSError as error:
        if error.errno in {errno.EINVAL, errno.ENOSYS}:
            raise LauncherError("launcher requires pidfd support") from error
        raise LauncherError(f"pidfd_open({pid}) failed") from error


def _send_pid_signal(pidfd: int, pid: int, signum: int) -> None:
    pidfd_send_signal = getattr(signal, "pidfd_send_signal", None)
    if pidfd is not None and pidfd_send_signal is not None:
        try:
            pidfd_send_signal(signum, pidfd)
        except ProcessLookupError:
            return
        except OSError:
            pass
        else:
            return
    try:
        os.kill(pid, signum)
    except ProcessLookupError:
        return


def _send_group_signal(pgid: int, signum: int) -> None:
    try:
        os.killpg(pgid, signum)
    except ProcessLookupError:
        return


def _load_sandbox_module(path: str | os.PathLike[str]) -> ModuleType:
    sandbox_path = _absolute_existing(path, "sandbox helper")
    specification = importlib.util.spec_from_file_location("gascity_agent_sandbox", sandbox_path)
    if specification is None or specification.loader is None:
        raise LauncherError(f"cannot load sandbox helper: {sandbox_path}")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def _install_nondumpable_and_fd_policy(keep_fds: Sequence[int]) -> None:
    _set_nondumpable()
    keep = set(keep_fds)
    try:
        entries = os.listdir("/proc/self/fd")
    except FileNotFoundError:
        entries = [str(fd) for fd in range(3, 256)]
    for entry in entries:
        if not entry.isdigit():
            continue
        descriptor = int(entry)
        if descriptor in keep:
            continue
        try:
            os.close(descriptor)
        except OSError as error:
            if error.errno != errno.EBADF:
                raise


def _set_nondumpable() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    result = libc.prctl(4, 0, 0, 0, 0)
    if result != 0:
        error_number = ctypes.get_errno()
        raise OSError(error_number, os.strerror(error_number))


@dataclass
class Child:
    process: subprocess.Popen[bytes]
    pidfd: int
    pgid: int
    stdin_fd: int
    stdout_fd: int
    stderr_fd: int

    def close_pidfd(self) -> None:
        try:
            os.close(self.pidfd)
        except OSError:
            pass


class ChildRelay:
    """Forward ACP bytes while retaining exact ownership of the child group."""

    def __init__(
        self,
        child: Child,
        *,
        run_id: str,
        control_fd: int | None,
        term_grace: float,
        kill_grace: float,
        stop_flag: Callable[[], bool] | None = None,
    ):
        self.child = child
        self.run_id = run_id
        self.control_fd = control_fd
        self.term_grace = term_grace
        self.kill_grace = kill_grace
        self.stop_flag = stop_flag
        self.stop_requested = False
        self.stop_reason = ""
        self.stop_deadline: float | None = None
        self.group_killed = False
        self.control_buffer = bytearray()
        self._pending_input = bytearray()

    def request_stop(self, reason: str) -> None:
        if self.stop_requested:
            return
        self.stop_requested = True
        self.stop_reason = reason
        self._pending_input.clear()
        try:
            os.close(self.child.stdin_fd)
        except OSError as error:
            if error.errno != errno.EBADF:
                raise
        _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGTERM)
        _send_group_signal(self.child.pgid, signal.SIGTERM)
        self.stop_deadline = time.monotonic() + self.term_grace

    def _force_stop_if_needed(self) -> None:
        if not self.stop_requested or self.stop_deadline is None:
            return
        if self.child.process.poll() is None and time.monotonic() >= self.stop_deadline:
            _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGKILL)
            _send_group_signal(self.child.pgid, signal.SIGKILL)
            self.group_killed = True
            self.stop_deadline = time.monotonic() + self.kill_grace

    @staticmethod
    def _write_all(fd: int, data: bytes) -> None:
        offset = 0
        while offset < len(data):
            try:
                written = os.write(fd, data[offset:])
            except BrokenPipeError:
                raise
            if written == 0:
                raise BrokenPipeError
            offset += written

    def _control_message(self, message: bytes) -> None:
        self.control_buffer.extend(message)
        while b"\n" in self.control_buffer:
            line, _, remainder = self.control_buffer.partition(b"\n")
            self.control_buffer = bytearray(remainder)
            if not line:
                continue
            try:
                request = json.loads(line)
            except json.JSONDecodeError:
                self._write_control({"ok": False, "error": "malformed"})
                continue
            if not isinstance(request, dict):
                self._write_control({"ok": False, "error": "malformed"})
                continue
            if request.get("run_id") != self.run_id:
                self._write_control({"ok": False, "error": "run-mismatch"})
                continue
            if request.get("op") == "cancel":
                self.request_stop("control-cancel")
                self._write_control({"ok": True, "op": "cancel"})
            elif request.get("op") == "drain":
                self.request_stop("service-drain")
                self._write_control({"ok": True, "op": "drain"})
            else:
                self._write_control({"ok": False, "error": "operation-not-allowed"})

    def _write_control(self, value: Mapping[str, object]) -> None:
        if self.control_fd is None:
            return
        encoded = json.dumps(value, separators=(",", ":")).encode("utf-8") + b"\n"
        try:
            self._write_all(self.control_fd, encoded)
        except (BrokenPipeError, OSError):
            try:
                os.close(self.control_fd)
            except OSError:
                pass
            self.control_fd = None

    def run(self) -> int:
        selector = selectors.DefaultSelector()
        stdin_open = True
        stdout_open = True
        stderr_open = True
        child_input = self._pending_input
        for descriptor in (0, self.child.stdout_fd, self.child.stderr_fd):
            os.set_blocking(descriptor, False)
        if self.control_fd is not None:
            os.set_blocking(self.control_fd, False)
        selector.register(0, selectors.EVENT_READ, "stdin")
        selector.register(self.child.stdout_fd, selectors.EVENT_READ, "stdout")
        selector.register(self.child.stderr_fd, selectors.EVENT_READ, "stderr")
        if self.control_fd is not None:
            selector.register(self.control_fd, selectors.EVENT_READ, "control")

        try:
            while stdin_open or stdout_open or stderr_open or self.child.process.poll() is None:
                if self.stop_flag is not None and self.stop_flag():
                    self.request_stop("launcher-signal")
                if self.child.process.poll() is not None and stdin_open:
                    stdin_open = False
                    try:
                        selector.unregister(0)
                    except KeyError:
                        pass
                    try:
                        os.close(self.child.stdin_fd)
                    except OSError as error:
                        if error.errno != errno.EBADF:
                            raise
                self._force_stop_if_needed()
                if child_input and stdin_open and not self.stop_requested:
                    try:
                        selector.register(self.child.stdin_fd, selectors.EVENT_WRITE, "child-stdin")
                    except KeyError:
                        pass
                elif not child_input:
                    try:
                        selector.unregister(self.child.stdin_fd)
                    except KeyError:
                        pass
                try:
                    events = selector.select(0.1)
                except InterruptedError:
                    continue
                if not events and self.child.process.poll() is not None:
                    if stdout_open or stderr_open:
                        continue
                    break
                for key, mask in events:
                    descriptor = int(key.fd)
                    kind = key.data
                    if kind == "stdin" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(0, 64 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            stdin_open = False
                            selector.unregister(0)
                            self.request_stop("acp-stdin-eof")
                        else:
                            child_input.extend(data)
                    elif kind == "child-stdin" and mask & selectors.EVENT_WRITE:
                        try:
                            written = os.write(descriptor, child_input)
                        except BrokenPipeError:
                            child_input.clear()
                            stdin_open = False
                            selector.unregister(descriptor)
                        except BlockingIOError:
                            continue
                        else:
                            del child_input[:written]
                    elif kind == "stdout" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(descriptor, 64 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            stdout_open = False
                            selector.unregister(descriptor)
                            if self.child.process.poll() is None:
                                self.request_stop("acp-stdout-eof")
                        else:
                            try:
                                self._write_all(1, data)
                            except BrokenPipeError:
                                self.request_stop("client-stdout-closed")
                                stdout_open = False
                                selector.unregister(descriptor)
                    elif kind == "stderr" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(descriptor, 64 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            stderr_open = False
                            selector.unregister(descriptor)
                        else:
                            try:
                                self._write_all(2, data)
                            except BrokenPipeError:
                                self.request_stop("client-stderr-closed")
                                stderr_open = False
                                selector.unregister(descriptor)
                    elif kind == "control" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(descriptor, 16 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            selector.unregister(descriptor)
                            try:
                                os.close(descriptor)
                            except OSError:
                                pass
                            self.control_fd = None
                        else:
                            self._control_message(data)
            self._force_stop_if_needed()
            if self.child.process.poll() is None:
                self.request_stop("relay-finished")
                deadline = time.monotonic() + self.term_grace
                while self.child.process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.01)
                self._force_stop_if_needed()
            if self.child.process.poll() is None:
                _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGKILL)
                _send_group_signal(self.child.pgid, signal.SIGKILL)
                self.group_killed = True
                deadline = time.monotonic() + self.kill_grace
                while self.child.process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.01)
            return 0 if self.stop_requested else self.child.process.wait()
        finally:
            selector.close()
            if self.control_fd is not None:
                try:
                    os.close(self.control_fd)
                except OSError as error:
                    if error.errno != errno.EBADF:
                        raise
                self.control_fd = None
            for descriptor in (
                self.child.stdin_fd,
                self.child.stdout_fd,
                self.child.stderr_fd,
            ):
                try:
                    os.close(descriptor)
                except OSError as error:
                    if error.errno != errno.EBADF:
                        raise
            self.child.close_pidfd()


class SocketChildRelay:
    """Proxy one authenticated launcher connection to exactly one ACP child."""

    def __init__(
        self,
        child: Child,
        client: socket.socket,
        *,
        run_id: str,
        control_fd: int | None,
        term_grace: float,
        kill_grace: float,
        stop_flag: Callable[[], bool] | None = None,
    ):
        self.child = child
        self.client = client
        self.run_id = run_id
        self.control_fd = control_fd
        self.term_grace = term_grace
        self.kill_grace = kill_grace
        self.stop_flag = stop_flag
        self.stop_requested = False
        self.stop_reason = ""
        self.stop_deadline: float | None = None
        self.group_killed = False
        self.control_buffer = bytearray()
        self._lock = threading.Lock()

    def request_stop(self, reason: str) -> None:
        with self._lock:
            if self.stop_requested:
                return
            self.stop_requested = True
            self.stop_reason = reason
            try:
                os.close(self.child.stdin_fd)
            except OSError as error:
                if error.errno != errno.EBADF:
                    raise
            _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGTERM)
            _send_group_signal(self.child.pgid, signal.SIGTERM)
            self.stop_deadline = time.monotonic() + self.term_grace

    def _force_stop_if_needed(self) -> None:
        if not self.stop_requested or self.stop_deadline is None:
            return
        if self.child.process.poll() is None and time.monotonic() >= self.stop_deadline:
            _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGKILL)
            _send_group_signal(self.child.pgid, signal.SIGKILL)
            self.group_killed = True
            self.stop_deadline = time.monotonic() + self.kill_grace

    @staticmethod
    def _write_all(fd: int, data: bytes) -> None:
        offset = 0
        while offset < len(data):
            written = os.write(fd, data[offset:])
            if written <= 0:
                raise BrokenPipeError
            offset += written

    def _write_control(self, value: Mapping[str, object]) -> None:
        if self.control_fd is None:
            return
        encoded = json.dumps(value, separators=(",", ":")).encode("utf-8") + b"\n"
        try:
            self._write_all(self.control_fd, encoded)
        except (BrokenPipeError, OSError):
            try:
                os.close(self.control_fd)
            except OSError:
                pass
            self.control_fd = None

    def _control_message(self, message: bytes) -> None:
        self.control_buffer.extend(message)
        if len(self.control_buffer) > MAX_LAUNCH_RESPONSE_BYTES:
            self._write_control({"ok": False, "error": "control-message-too-large"})
            self.request_stop("control-message-too-large")
            return
        while b"\n" in self.control_buffer:
            line, _, remainder = self.control_buffer.partition(b"\n")
            self.control_buffer = bytearray(remainder)
            if not line:
                continue
            try:
                request = json.loads(line)
            except json.JSONDecodeError:
                self._write_control({"ok": False, "error": "malformed"})
                continue
            if not isinstance(request, dict):
                self._write_control({"ok": False, "error": "malformed"})
                continue
            if request.get("run_id") != self.run_id:
                self._write_control({"ok": False, "error": "run-mismatch"})
                continue
            operation = request.get("op")
            if operation == "cancel":
                self.request_stop("control-cancel")
                self._write_control({"ok": True, "op": "cancel"})
            elif operation == "drain":
                self.request_stop("service-drain")
                self._write_control({"ok": True, "op": "drain"})
            else:
                self._write_control({"ok": False, "error": "operation-not-allowed"})

    def abort(self, reason: str) -> None:
        """Clean up a child when the relay never entered its event loop."""

        self.request_stop(reason)
        deadline = time.monotonic() + self.term_grace
        while (
            self.child.process.poll() is None
            and time.monotonic() < deadline
        ):
            time.sleep(0.01)
        if self.child.process.poll() is None:
            _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGKILL)
            _send_group_signal(self.child.pgid, signal.SIGKILL)
            try:
                self.child.process.wait(timeout=self.kill_grace)
            except subprocess.TimeoutExpired:
                _send_pid_signal(
                    self.child.pidfd,
                    self.child.process.pid,
                    signal.SIGKILL,
                )
                _send_group_signal(self.child.pgid, signal.SIGKILL)
                self.child.process.wait()
        else:
            self.child.process.wait()
        for descriptor in (
            self.child.stdin_fd,
            self.child.stdout_fd,
            self.child.stderr_fd,
        ):
            try:
                os.close(descriptor)
            except OSError:
                pass
        self.child.close_pidfd()
        if self.control_fd is not None:
            try:
                os.close(self.control_fd)
            except OSError:
                pass
            self.control_fd = None
        try:
            self.client.close()
        except OSError:
            pass

    def run(self) -> int:
        selector = selectors.DefaultSelector()
        client_open = True
        child_output = bytearray()
        stderr_open = True
        child_input = bytearray()
        client_descriptor = self.client.fileno()
        for descriptor in (client_descriptor, self.child.stdout_fd, self.child.stderr_fd):
            os.set_blocking(descriptor, False)
        if self.control_fd is not None:
            os.set_blocking(self.control_fd, False)
        selector.register(client_descriptor, selectors.EVENT_READ, "client")
        selector.register(self.child.stdout_fd, selectors.EVENT_READ, "stdout")
        selector.register(self.child.stderr_fd, selectors.EVENT_READ, "stderr")
        if self.control_fd is not None:
            selector.register(self.control_fd, selectors.EVENT_READ, "control")

        def update_client_events() -> None:
            if not client_open and not child_output:
                try:
                    selector.unregister(client_descriptor)
                except KeyError:
                    pass
                return
            events = 0
            if client_open:
                events |= selectors.EVENT_READ
            if child_output:
                events |= selectors.EVENT_WRITE
            try:
                selector.modify(client_descriptor, events, "client")
            except KeyError:
                if events:
                    selector.register(client_descriptor, events, "client")

        try:
            while client_open or child_output or stderr_open or self.child.process.poll() is None:
                if self.stop_flag is not None and self.stop_flag():
                    self.request_stop("service-drain")
                if self.child.process.poll() is not None:
                    if client_open:
                        client_open = False
                    try:
                        selector.unregister(client_descriptor)
                    except KeyError:
                        pass
                self._force_stop_if_needed()
                if child_input and not self.stop_requested:
                    try:
                        selector.register(self.child.stdin_fd, selectors.EVENT_WRITE, "child-stdin")
                    except KeyError:
                        pass
                else:
                    try:
                        selector.unregister(self.child.stdin_fd)
                    except KeyError:
                        pass
                update_client_events()
                try:
                    events = selector.select(0.1)
                except InterruptedError:
                    continue
                for key, mask in events:
                    descriptor = int(key.fd)
                    kind = key.data
                    if kind == "client":
                        if mask & selectors.EVENT_READ and client_open:
                            try:
                                data = self.client.recv(64 * 1024)
                            except BlockingIOError:
                                continue
                            except (ConnectionError, OSError):
                                data = b""
                            if not data:
                                client_open = False
                                try:
                                    selector.unregister(client_descriptor)
                                except KeyError:
                                    pass
                                self.request_stop("launcher-client-eof")
                            elif len(child_input) + len(data) > MAX_RELAY_BUFFER_BYTES:
                                self.request_stop("launcher-input-buffer-limit")
                            else:
                                child_input.extend(data)
                        if mask & selectors.EVENT_WRITE and child_output:
                            try:
                                written = self.client.send(child_output)
                            except (BlockingIOError, InterruptedError):
                                continue
                            except (BrokenPipeError, ConnectionError, OSError):
                                child_output.clear()
                                client_open = False
                                self.request_stop("launcher-client-closed")
                            else:
                                del child_output[:written]
                    elif kind == "child-stdin" and mask & selectors.EVENT_WRITE:
                        try:
                            written = os.write(descriptor, child_input)
                        except (BrokenPipeError, OSError) as error:
                            if getattr(error, "errno", None) not in {errno.EAGAIN, errno.EWOULDBLOCK}:
                                child_input.clear()
                                self.request_stop("acp-stdin-closed")
                        else:
                            del child_input[:written]
                    elif kind == "stdout" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(descriptor, 64 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            try:
                                selector.unregister(descriptor)
                            except KeyError:
                                pass
                            if self.child.process.poll() is None:
                                self.request_stop("acp-stdout-eof")
                        elif len(child_output) + len(data) > MAX_RELAY_BUFFER_BYTES:
                            self.request_stop("launcher-output-buffer-limit")
                        else:
                            child_output.extend(data)
                    elif kind == "stderr" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(descriptor, 64 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            stderr_open = False
                            try:
                                selector.unregister(descriptor)
                            except KeyError:
                                pass
                        else:
                            try:
                                self._write_all(2, data)
                            except (BrokenPipeError, OSError):
                                stderr_open = False
                    elif kind == "control" and mask & selectors.EVENT_READ:
                        try:
                            data = os.read(descriptor, 16 * 1024)
                        except BlockingIOError:
                            continue
                        if not data:
                            try:
                                selector.unregister(descriptor)
                            except KeyError:
                                pass
                            try:
                                os.close(descriptor)
                            except OSError:
                                pass
                            self.control_fd = None
                        else:
                            self._control_message(data)
            self._force_stop_if_needed()
            if self.child.process.poll() is None:
                self.request_stop("relay-finished")
                deadline = time.monotonic() + self.term_grace
                while self.child.process.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.01)
                self._force_stop_if_needed()
            if self.child.process.poll() is None:
                _send_pid_signal(self.child.pidfd, self.child.process.pid, signal.SIGKILL)
                _send_group_signal(self.child.pgid, signal.SIGKILL)
                self.group_killed = True
            return 0 if self.stop_requested else self.child.process.wait()
        finally:
            selector.close()
            if self.control_fd is not None:
                try:
                    os.close(self.control_fd)
                except OSError:
                    pass
                self.control_fd = None
            for descriptor in (
                self.child.stdin_fd,
                self.child.stdout_fd,
                self.child.stderr_fd,
            ):
                try:
                    os.close(descriptor)
                except OSError:
                    pass
            self.child.close_pidfd()
            try:
                self.client.close()
            except OSError:
                pass


def _signal_flag(state: dict[str, bool], _signum: int, _frame) -> None:
    state["stop"] = True


def _close_descriptors(descriptors: Sequence[int]) -> None:
    for descriptor in descriptors:
        try:
            os.close(descriptor)
        except OSError:
            pass


def _socket_path(value: str | os.PathLike[str], label: str) -> pathlib.Path:
    path = pathlib.Path(value)
    if not path.is_absolute() or any(part == ".." for part in path.parts):
        raise LauncherError(f"{label} must be an absolute normalized path")
    if path == pathlib.Path("/"):
        raise LauncherError(f"{label} is malformed")
    return path


def _peer_credentials(client: socket.socket) -> tuple[int, int, int]:
    try:
        raw = client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
    except OSError as error:
        raise LauncherError("authenticated launcher peers are unavailable") from error
    if len(raw) != 12:
        raise LauncherError("authenticated launcher peer credentials are malformed")
    return struct.unpack("3i", raw)


def _read_auth_token(path: str | os.PathLike[str] | None) -> str | None:
    if path is None:
        return None
    token_path = pathlib.Path(path)
    if (
        not token_path.is_absolute()
        or any(part == ".." for part in token_path.parts)
        or token_path.is_symlink()
    ):
        raise LauncherError("launcher authentication token path is malformed")
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    descriptor: int | None = None
    stream = None
    try:
        descriptor = os.open(token_path, flags)
        stream = os.fdopen(descriptor, "r", encoding="utf-8")
        descriptor = None
        try:
            stat_result = os.fstat(stream.fileno())
            if (
                not stat.S_ISREG(stat_result.st_mode)
                or stat_result.st_uid != os.geteuid()
                or stat_result.st_mode & 0o077
            ):
                raise LauncherError(
                    "launcher authentication token is not a private regular file"
                )
            raw_token = stream.buffer.read(513)
            if len(raw_token) > 512:
                raise LauncherError("launcher authentication token is malformed")
            token = raw_token.decode("utf-8")
        finally:
            stream.close()
    except (OSError, UnicodeError) as error:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
        raise LauncherError("launcher authentication token is unreadable") from error
    token = token.rstrip("\n")
    if (
        not token
        or len(token.encode("utf-8")) > 512
        or any(ord(character) < 0x21 or ord(character) > 0x7E for character in token)
    ):
        raise LauncherError("launcher authentication token is malformed")
    return token


def _extract_rights(
    ancillary: Sequence[tuple[int, int, bytes]],
) -> list[int]:
    descriptors: list[int] = []
    item_size = array.array("i").itemsize
    for level, kind, data in ancillary:
        if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS:
            _close_descriptors(descriptors)
            raise LauncherError("launcher request contains unauthorized ancillary data")
        if len(data) % item_size:
            complete = len(data) - (len(data) % item_size)
            if complete:
                complete_values = array.array("i")
                complete_values.frombytes(data[:complete])
                _close_descriptors(complete_values)
            _close_descriptors(descriptors)
            raise LauncherError("launcher SCM_RIGHTS payload is malformed")
        values = array.array("i")
        values.frombytes(data)
        descriptors.extend(int(value) for value in values)
    for descriptor in descriptors:
        try:
            flags = fcntl.fcntl(descriptor, fcntl.F_GETFD)
            fcntl.fcntl(descriptor, fcntl.F_SETFD, flags | fcntl.FD_CLOEXEC)
        except OSError:
            _close_descriptors(descriptors)
            raise LauncherError("launcher attachment fd is invalid")
    return descriptors


def _receive_launch_request(
    client: socket.socket,
) -> tuple[dict[str, object], list[int]]:
    data = bytearray()
    descriptors: list[int] = []
    cmsg_space = socket.CMSG_SPACE(array.array("i").itemsize * len(LAUNCHER_FD_NAMES))
    try:
        while b"\n" not in data:
            if len(data) >= MAX_LAUNCH_METADATA_BYTES:
                raise LauncherError("launcher metadata exceeds the size limit")
            chunk, ancillary, flags, _address = client.recvmsg(
                min(4096, MAX_LAUNCH_METADATA_BYTES - len(data)),
                cmsg_space,
            )
            descriptors.extend(_extract_rights(ancillary))
            if flags & getattr(socket, "MSG_CTRUNC", 0):
                _close_descriptors(descriptors)
                raise LauncherError("launcher request ancillary data was truncated")
            if not chunk:
                raise LauncherError("launcher client closed before launch metadata")
            data.extend(chunk)
            if len(data) > MAX_LAUNCH_METADATA_BYTES:
                raise LauncherError("launcher metadata exceeds the size limit")
        line, remainder = bytes(data).split(b"\n", 1)
        if remainder:
            raise LauncherError("launcher metadata was followed by ACP bytes")
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise LauncherError("launcher metadata is malformed JSON") from error
        if not isinstance(value, dict):
            raise LauncherError("launcher metadata is not an object")
        return value, descriptors
    except Exception:
        _close_descriptors(descriptors)
        raise


def _validate_launch_metadata(
    value: object,
    *,
    client_uid: int,
    allowed_uids: set[int],
    auth_token: str | None,
) -> dict[str, object]:
    if client_uid not in allowed_uids:
        raise LauncherError("launcher client identity is not authorized")
    if not isinstance(value, dict):
        raise LauncherError("launcher metadata is not an object")
    required = {
        "protocol",
        "operation",
        "profile",
        "tool_policy",
        "run_id",
        "bead_id",
        "generation",
        "state_schema",
        "worktree",
        "fds",
    }
    optional = {
        "auth",
        "require_ready",
        "root_bead_id",
        "state_root",
        "terminal_state_path",
    }
    if set(value) - required - optional or not required.issubset(value):
        raise LauncherError("launcher metadata has an unauthorized shape")
    if value.get("protocol") != LAUNCHER_PROTOCOL or value.get("operation") != "launch":
        raise LauncherError("launcher protocol or operation is unsupported")
    profile = value.get("profile")
    if not isinstance(profile, str) or profile not in PROFILE_NAMES:
        raise LauncherError("launcher profile is malformed")
    tool_policy = value.get("tool_policy")
    if not isinstance(tool_policy, str) or tool_policy not in TOOL_POLICIES:
        raise LauncherError("launcher tool policy is malformed")
    for key in ("run_id", "bead_id", "generation", "state_schema"):
        candidate = value.get(key)
        if not isinstance(candidate, str):
            raise LauncherError(f"launcher {key} is malformed")
        _validate_identifier(candidate, key.replace("_", " "))
    root_bead_id = value.get("root_bead_id")
    if root_bead_id is not None:
        if not isinstance(root_bead_id, str):
            raise LauncherError("launcher root_bead_id is malformed")
        _validate_identifier(root_bead_id, "root bead id")
    for key in ("worktree", "state_root", "terminal_state_path"):
        candidate = value.get(key)
        if candidate is None and key in {"state_root", "terminal_state_path"}:
            continue
        if not isinstance(candidate, str):
            raise LauncherError(f"launcher {key} is malformed")
        _socket_path(candidate, f"launcher {key}")
    requested_fds = value.get("fds")
    if (
        not isinstance(requested_fds, list)
        or any(
            not isinstance(name, str) or name not in LAUNCHER_FD_NAMES
            for name in requested_fds
        )
        or len(set(requested_fds)) != len(requested_fds)
    ):
        raise LauncherError("launcher attachment metadata is malformed")
    require_ready = value.get("require_ready", False)
    if type(require_ready) is not bool:
        raise LauncherError("launcher readiness flag is malformed")
    if auth_token is not None:
        supplied = value.get("auth")
        if (
            not isinstance(supplied, str)
            or not hmac.compare_digest(supplied, auth_token)
        ):
            raise LauncherError("launcher authentication failed")
    elif "auth" in value:
        supplied = value["auth"]
        if not isinstance(supplied, str) or len(supplied.encode("utf-8")) > 512:
            raise LauncherError("launcher authentication field is malformed")
    return dict(value)


def _profile_child_arguments(
    profile: str,
    tool_policy: str,
    extra: Sequence[str] = (),
) -> list[str]:
    if profile not in PROFILE_NAMES:
        raise LauncherError(f"unknown Copilot profile: {profile}")
    tools = TOOL_POLICIES.get(tool_policy)
    if tools is None:
        raise LauncherError(f"unknown tool policy: {tool_policy}")
    values = [
        "--acp",
        "--model",
        PROFILE_SETTINGS[profile]["model"],
        "--context",
        PROFILE_SETTINGS[profile]["contextTier"],
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
        tools,
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
    values.extend(extra)
    return validate_child_arguments(values, profile=profile)


class ActiveConnections:
    """Track every accepted connection so service drain is exact and complete."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._relays: dict[int, SocketChildRelay] = {}
        self._threads: list[threading.Thread] = []
        self.draining = False

    def add(self, relay: SocketChildRelay) -> None:
        with self._lock:
            if self.draining:
                relay.request_stop("service-drain")
                raise LauncherError("launcher is draining")
            self._relays[id(relay)] = relay

    def add_thread(self, thread: threading.Thread) -> None:
        with self._lock:
            self._threads.append(thread)

    def remove_thread(self, thread: threading.Thread) -> None:
        with self._lock:
            try:
                self._threads.remove(thread)
            except ValueError:
                pass

    def remove(self, relay: SocketChildRelay) -> None:
        with self._lock:
            self._relays.pop(id(relay), None)

    def request_stop_all(self, reason: str) -> None:
        with self._lock:
            self.draining = True
            relays = tuple(self._relays.values())
        for relay in relays:
            relay.request_stop(reason)

    def join_all(self) -> None:
        while True:
            with self._lock:
                threads = tuple(self._threads)
                self._threads = [thread for thread in self._threads if thread.is_alive()]
            if not threads:
                return
            for thread in threads:
                thread.join(timeout=0.2)
            with self._lock:
                if not any(thread.is_alive() for thread in self._threads):
                    return


def _server_state_root(
    args: argparse.Namespace,
    metadata: Mapping[str, object],
) -> pathlib.Path | None:
    configured = pathlib.Path(args.state_root) if args.state_root else None
    requested_value = metadata.get("state_root")
    requested = pathlib.Path(requested_value) if isinstance(requested_value, str) else None
    if configured is not None and requested is not None:
        if configured != requested:
            raise LauncherError("assigned state root is not owned by the launcher")
    state_root = configured or requested
    if state_root is None:
        return None
    return _absolute_existing(state_root, "state root")


def _serve_client(
    client: socket.socket,
    *,
    args: argparse.Namespace,
    allowed_uids: set[int],
    auth_token: str | None,
    connections: ActiveConnections,
    drain_event: threading.Event,
) -> None:
    descriptors: list[int] = []
    relay: SocketChildRelay | None = None
    home: pathlib.Path | None = None
    lease: ConcurrencyLease | None = None
    active_roots = None
    relay_started = False
    acknowledged = False
    try:
        client.settimeout(1.0)
        _pid, client_uid, _gid = _peer_credentials(client)
        raw_metadata, descriptors = _receive_launch_request(client)
        metadata = _validate_launch_metadata(
            raw_metadata,
            client_uid=client_uid,
            allowed_uids=allowed_uids,
            auth_token=auth_token,
        )
        if drain_event.is_set():
            raise LauncherError("launcher is draining")
        names = metadata["fds"]
        assert isinstance(names, list)
        if len(descriptors) != len(names):
            raise LauncherError("launcher attachment count does not match metadata")
        attachments = dict(zip(names, descriptors, strict=True))
        run_id = str(metadata["run_id"])
        bead_id = str(metadata["bead_id"])
        root_bead_id = str(metadata.get("root_bead_id", run_id))
        generation = str(metadata["generation"])
        state_schema = str(metadata["state_schema"])
        if args.generation and generation != args.generation:
            raise LauncherError("launch generation does not match the service generation")
        if args.state_schema and state_schema != str(args.state_schema):
            raise LauncherError("launch state schema does not match the service schema")
        profile = str(metadata["profile"])
        tool_policy = str(metadata["tool_policy"])
        worktree = _absolute_existing(str(metadata["worktree"]), "assigned worktree")
        if not worktree.is_dir():
            raise LauncherError("assigned worktree is not a directory")
        state_root = _server_state_root(args, metadata)
        terminal_state_path = metadata.get("terminal_state_path")
        if terminal_state_path is not None:
            terminal_path = _socket_path(
                str(terminal_state_path),
                "terminal workflow state",
            )
            if state_root is not None:
                try:
                    terminal_path.relative_to(state_root)
                except ValueError as error:
                    raise LauncherError(
                        "terminal workflow state is outside the assigned state root"
                    ) from error
        if state_root is not None:
            try:
                worktree.relative_to(state_root)
                raise LauncherError("assigned worktree is inside state root")
            except ValueError:
                pass
            try:
                state_root.relative_to(worktree)
                raise LauncherError("state root is inside assigned worktree")
            except ValueError:
                pass
        if args.require_ready or metadata.get("require_ready") is True:
            if not args.readiness_status:
                raise StaleReadiness("readiness status is required")
            require_readiness(
                args.readiness_status,
                generation=generation,
                state_schema=state_schema,
                profile=profile,
            )
        settings_root = _absolute_existing(args.settings_root, "profile settings root")
        settings = _absolute_existing(
            settings_root / profile / "settings.json",
            "profile settings",
        )
        _load_settings(settings, profile)
        runtime_root = _private_directory(args.runtime_root, "agent runtime root")
        lease = ConcurrencyLease.acquire(
            args.lease_root,
            run_id=run_id,
            bead_id=bead_id,
            max_agents=args.max_agents,
            max_active_runs=args.max_active_runs,
        )
        active_roots = _create_active_run_roots(
            args,
            run_id=run_id,
            bead_id=root_bead_id,
            generation=generation,
            state_schema=state_schema,
            terminal_state_path=(
                str(terminal_state_path)
                if terminal_state_path is not None
                else None
            ),
        )
        home = materialize_copilot_home(
            settings,
            profile=profile,
            runtime_root=runtime_root,
            run_id=run_id,
            bead_id=bead_id,
        )
        environment = scrub_environment(
            profile=profile,
            run_id=run_id,
            bead_id=bead_id,
            generation=generation,
            state_schema=state_schema,
            root_bead_id=root_bead_id,
        )
        environment["COPILOT_HOME"] = str(home)
        proxy_fd = attachments.get("proxy")
        progress_fd = attachments.get("progress")
        control_fd = attachments.get("control")
        check_fd = attachments.get("check")
        if check_fd is not None and tool_policy != "coding":
            raise LauncherError("check channel is available only to coding launches")
        if proxy_fd is None:
            for proxy_name in ("ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"):
                environment.pop(proxy_name, None)
            environment.pop("GC_FDPROXY_FD", None)
            environment.pop("GC_FDPROXY_AUTH", None)
        if progress_fd is None:
            environment.pop("GC_AGENT_FD", None)
        child = _spawn_child(
            profile=profile,
            tool_policy=tool_policy,
            settings_path=settings,
            copilot=str(_absolute_existing(args.copilot, "Copilot executable")),
            child_arguments=_profile_child_arguments(
                profile,
                tool_policy,
                args.fixture_child_arg,
            ),
            worktree=worktree,
            state_root=state_root,
            home=home,
            environment=environment,
            args=args,
            proxy_fd=proxy_fd,
            progress_fd=progress_fd,
            check_fd=check_fd,
        )
        for descriptor in (proxy_fd, progress_fd, check_fd):
            if descriptor is not None:
                try:
                    descriptors.remove(descriptor)
                except ValueError:
                    pass
                try:
                    os.close(descriptor)
                except OSError:
                    pass
        relay = SocketChildRelay(
            child,
            client,
            run_id=run_id,
            control_fd=control_fd,
            term_grace=args.term_grace,
            kill_grace=args.kill_grace,
            stop_flag=drain_event.is_set,
        )
        client.settimeout(None)
        if control_fd is not None:
            try:
                descriptors.remove(control_fd)
            except ValueError:
                pass
        connections.add(relay)
        _write_json_line(
            client,
            {
                "protocol": LAUNCHER_PROTOCOL,
                "ok": True,
                "profile": profile,
                "run_id": run_id,
            },
        )
        acknowledged = True
        relay_started = True
        relay.run()
        if active_roots is not None and isinstance(terminal_state_path, str):
            _cleanup_active_run_roots(active_roots, terminal_state_path)
    except (OSError, LauncherError) as error:
        if not acknowledged:
            try:
                _write_json_line(
                    client,
                    {
                        "protocol": LAUNCHER_PROTOCOL,
                        "ok": False,
                        "error": str(error)[:256],
                    },
                )
            except OSError:
                pass
        if relay is not None:
            if relay_started:
                relay.request_stop("launcher-error")
            else:
                relay.abort("launcher-error")
        elif lease is not None:
            lease.release()
    finally:
        if relay is not None:
            connections.remove(relay)
        if lease is not None:
            lease.release()
        _close_descriptors(descriptors)
        if home is not None:
            try:
                if home.is_dir() and not home.is_symlink():
                    shutil.rmtree(home)
                elif home.exists() or home.is_symlink():
                    home.unlink()
            except OSError:
                pass
        try:
            client.close()
        except OSError:
            pass
        connections.remove_thread(threading.current_thread())


def _write_json_line(channel: socket.socket, value: Mapping[str, object]) -> None:
    encoded = json.dumps(value, separators=(",", ":")).encode("utf-8") + b"\n"
    if len(encoded) > MAX_LAUNCH_RESPONSE_BYTES:
        raise LauncherError("launcher response exceeds the size limit")
    channel.sendall(encoded)


def _bind_server_socket(path: pathlib.Path) -> socket.socket:
    path.parent.mkdir(mode=0o750, parents=True, exist_ok=True)
    parent_stat = path.parent.stat()
    if parent_stat.st_uid != os.geteuid() or parent_stat.st_mode & 0o022:
        raise LauncherError("launcher socket parent is not service-owned")
    if path.exists() or path.is_symlink():
        path_stat = path.lstat()
        if not stat.S_ISSOCK(path_stat.st_mode) or path_stat.st_uid != os.geteuid():
            raise LauncherError("launcher socket path is not a service-owned socket")
        path.unlink()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        listener.bind(str(path))
        os.chmod(path, 0o660)
        listener.listen(16)
        listener.settimeout(0.2)
        return listener
    except OSError:
        listener.close()
        raise


def serve_server(args: argparse.Namespace) -> int:
    _set_nondumpable()
    _validate_gc_root_configuration(args)
    if not args.socket:
        raise LauncherError("launcher server socket is required")
    if args.max_agents is None or args.max_active_runs is None:
        raise LauncherError("launcher server concurrency limits are required")
    if not args.settings_root:
        raise LauncherError("launcher server settings root is required")
    socket_path = _socket_path(args.socket, "launcher server socket")
    _absolute_existing(args.settings_root, "profile settings root")
    _absolute_existing(args.copilot, "Copilot executable")
    _private_directory(args.lease_root, "lease root")
    _private_directory(args.runtime_root, "agent runtime root")
    auth_token = _read_auth_token(args.auth_token_file)
    configured_uids = set(args.client_uid or [os.geteuid()])
    if any(uid < 0 for uid in configured_uids):
        raise LauncherError("launcher client uid is malformed")
    if args.allow_unsafe_fixture and os.environ.get("GC_TEST_MODE") != "1":
        raise LauncherError("unsafe fixture mode requires GC_TEST_MODE=1")
    if args.fixture_child_arg and not args.allow_unsafe_fixture:
        raise LauncherError("fixture child arguments require unsafe fixture mode")
    if args.fixture_child_script and not args.allow_unsafe_fixture:
        raise LauncherError("fixture child script requires unsafe fixture mode")
    args.skip_sandbox = args.allow_unsafe_fixture
    connections = ActiveConnections()
    drain_event = threading.Event()
    previous_term = signal.signal(signal.SIGTERM, lambda *_: drain_event.set())
    previous_int = signal.signal(signal.SIGINT, lambda *_: drain_event.set())
    listener = _bind_server_socket(socket_path)
    try:
        while not drain_event.is_set():
            try:
                client, _address = listener.accept()
            except socket.timeout:
                continue
            if drain_event.is_set():
                client.close()
                break
            thread = threading.Thread(
                target=_serve_client,
                kwargs={
                    "client": client,
                    "args": args,
                    "allowed_uids": configured_uids,
                    "auth_token": auth_token,
                    "connections": connections,
                    "drain_event": drain_event,
                },
                name="gascity-agent-client",
                daemon=False,
            )
            connections.add_thread(thread)
            thread.start()
        connections.request_stop_all("service-drain")
    finally:
        listener.close()
        connections.request_stop_all("service-drain")
        connections.join_all()
        try:
            if socket_path.is_socket():
                socket_path.unlink()
        except OSError:
            pass
        signal.signal(signal.SIGTERM, previous_term)
        signal.signal(signal.SIGINT, previous_int)
    return 0

def require_readiness(
    path: str | os.PathLike[str],
    *,
    generation: str,
    state_schema: str,
    profile: str,
) -> dict[str, object]:
    status_path = _absolute_existing(path, "readiness status")
    try:
        with status_path.open("r", encoding="utf-8") as stream:
            status = json.load(stream)
    except (OSError, json.JSONDecodeError) as error:
        raise StaleReadiness("readiness status is unreadable or malformed") from error
    if not isinstance(status, dict) or set(status) != {
        "generation",
        "state_schema",
        "ready",
        "effective_profiles",
        "error_code",
    }:
        raise StaleReadiness("readiness status is not an object")
    if (
        status.get("generation") != generation
        or status.get("state_schema") != state_schema
        or status.get("ready") is not True
        or status.get("error_code") is not None
    ):
        raise StaleReadiness("readiness status is stale or not ready")
    effective_profiles = status.get("effective_profiles")
    if (
        not isinstance(effective_profiles, dict)
        or effective_profiles.get("coding") != "code-luna"
        or effective_profiles.get("review") not in {"review-sol", "review-luna"}
        or profile not in effective_profiles.values()
    ):
        raise StaleReadiness(f"profile {profile} is not selected by readiness")
    return status


def _spawn_child(
    *,
    profile: str,
    tool_policy: str,
    settings_path: pathlib.Path,
    copilot: str,
    child_arguments: Sequence[str],
    worktree: pathlib.Path,
    state_root: pathlib.Path | None,
    home: pathlib.Path,
    environment: Mapping[str, str],
    args: argparse.Namespace,
    proxy_fd: int | None,
    progress_fd: int | None,
    check_fd: int | None,
) -> Child:
    sandbox_module = None
    if not args.skip_sandbox:
        sandbox_module = _load_sandbox_module(args.sandbox_script)
    validated_arguments = validate_child_arguments(child_arguments, profile=profile)
    fixture_script = getattr(args, "fixture_child_script", None)
    if fixture_script:
        command = [
            copilot,
            str(_absolute_existing(fixture_script, "fixture child script")),
            *validated_arguments,
        ]
    else:
        command = [copilot, *validated_arguments]
    inherited_fds = tuple(
        sorted(fd for fd in (proxy_fd, progress_fd, check_fd) if fd is not None)
    )
    if sandbox_module is not None:
        sandbox_argv, inherited_fds = sandbox_module.build_sandbox_argv(
            command,
            worktree=str(worktree),
            tool_policy=tool_policy,
            state_root=str(state_root) if state_root else None,
            copilot_home=str(home),
            runtime_paths=args.runtime_path,
            approved_wrappers=args.approved_wrapper,
            environment=environment,
            proxy_fd=proxy_fd,
            progress_fd=progress_fd,
            check_fd=check_fd,
            fdproxy_path=args.fdproxy_script,
            python_path=args.sandbox_python,
            bwrap_path=args.bwrap_path,
            proxy_port=args.proxy_port,
        )
        command = sandbox_argv

    def preexec() -> None:
        _install_nondumpable_and_fd_policy((0, 1, 2, *inherited_fds))

    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=worktree,
        env=dict(environment),
        start_new_session=True,
        close_fds=True,
        pass_fds=inherited_fds,
        preexec_fn=preexec,
    )
    if process.stdin is None or process.stdout is None or process.stderr is None:
        process.kill()
        process.wait()
        raise LauncherError("child stdio pipes were not created")
    try:
        pidfd = _pidfd_open(process.pid)
    except LauncherError:
        process.kill()
        process.wait()
        for stream in (process.stdin, process.stdout, process.stderr):
            stream.close()
        raise
    return Child(
        process=process,
        pidfd=pidfd,
        pgid=process.pid,
        stdin_fd=process.stdin.fileno(),
        stdout_fd=process.stdout.fileno(),
        stderr_fd=process.stderr.fileno(),
    )


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", action="store_true")
    parser.add_argument("--socket")
    parser.add_argument("--profile", choices=sorted(PROFILE_NAMES))
    parser.add_argument("--tool-policy", choices=["review", "planning", "coding"])
    parser.add_argument("--settings")
    parser.add_argument("--settings-root")
    parser.add_argument("--copilot", required=True)
    parser.add_argument("--run-id")
    parser.add_argument("--bead-id")
    parser.add_argument("--generation")
    parser.add_argument("--state-schema", default="1")
    parser.add_argument("--worktree", default=os.getcwd())
    parser.add_argument("--state-root")
    parser.add_argument("--terminal-state-path")
    parser.add_argument("--activation-script")
    parser.add_argument("--gc-root-directory")
    parser.add_argument("--gc-root-prefix", action="append", default=[])
    parser.add_argument("--package-path")
    parser.add_argument("--city-path")
    parser.add_argument("--pack-path")
    parser.add_argument("--profiles-path")
    parser.add_argument("--instructions-path")
    parser.add_argument("--lease-root", required=True)
    parser.add_argument("--runtime-root", required=True)
    parser.add_argument("--runtime-path", action="append", default=[])
    parser.add_argument("--approved-wrapper", action="append", default=[])
    parser.add_argument("--sandbox-script", required=True)
    parser.add_argument("--fdproxy-script", required=True)
    parser.add_argument("--sandbox-python")
    parser.add_argument("--bwrap-path")
    parser.add_argument("--proxy-fd", type=int)
    parser.add_argument("--progress-fd", type=int)
    parser.add_argument("--control-fd", type=int)
    parser.add_argument("--check-fd", type=int)
    parser.add_argument("--proxy-port", type=int, default=3128)
    parser.add_argument("--max-agents", type=int)
    parser.add_argument("--max-active-runs", type=int)
    parser.add_argument("--term-grace", type=float, default=2.0)
    parser.add_argument("--kill-grace", type=float, default=1.0)
    parser.add_argument("--require-ready", action="store_true")
    parser.add_argument("--readiness-status")
    parser.add_argument("--probe", action="store_true")
    parser.add_argument("--allow-unsafe-fixture", action="store_true")
    parser.add_argument("--client-uid", action="append", type=int, default=[])
    parser.add_argument("--auth-token-file")
    parser.add_argument("--fixture-child-arg", action="append", default=[])
    parser.add_argument("--fixture-child-script")
    parser.add_argument("child_arguments", nargs=argparse.REMAINDER)
    return parser.parse_args()


def _child_args(args: argparse.Namespace) -> list[str]:
    values = list(args.child_arguments)
    if values[:1] == ["--"]:
        values = values[1:]
    if not values:
        raise LauncherError("Copilot ACP child arguments are required")
    if "--acp" not in values:
        raise LauncherError("the launcher child must include ACP mode")
    return validate_child_arguments(values, profile=args.profile)


def run(args: argparse.Namespace) -> int:
    _set_nondumpable()
    _validate_gc_root_configuration(args)
    if not args.allow_unsafe_fixture or os.environ.get("GC_TEST_MODE") != "1":
        raise LauncherError(
            "single-child launcher mode is restricted to explicit GC_TEST_MODE fixtures"
        )
    if (
        not args.profile
        or not args.settings
        or not args.run_id
        or not args.bead_id
        or not args.tool_policy
    ):
        raise LauncherError("fixture launcher mode is missing launch metadata")
    run_id = _validate_identifier(args.run_id, "run id")
    bead_id = _validate_identifier(args.bead_id, "bead id")
    generation = args.generation or os.environ.get("GC_CITY_GENERATION", "")
    if not generation:
        raise LauncherError("city generation is required")
    state_schema = str(args.state_schema)
    if args.term_grace <= 0 or args.kill_grace <= 0:
        raise LauncherError("termination grace periods must be positive")
    for name, descriptor in (
        ("proxy", args.proxy_fd),
        ("progress", args.progress_fd),
        ("control", args.control_fd),
        ("check", args.check_fd),
    ):
        if descriptor is not None and descriptor < 3:
            raise LauncherError(f"{name} fd must not overlap stdio")
    descriptors = [
        descriptor
        for descriptor in (
            args.proxy_fd,
            args.progress_fd,
            args.control_fd,
            args.check_fd,
        )
        if descriptor is not None
    ]
    if len(set(descriptors)) != len(descriptors):
        raise LauncherError("launcher attachment fds must be distinct")
    skip_sandbox = args.allow_unsafe_fixture
    args.skip_sandbox = skip_sandbox
    if not args.allow_unsafe_fixture and not args.probe:
        if not args.readiness_status:
            raise StaleReadiness("readiness status is required")
        require_readiness(
            args.readiness_status,
            generation=generation,
            state_schema=state_schema,
            profile=args.profile,
        )
    worktree = _absolute_existing(args.worktree, "assigned worktree")
    if not worktree.is_dir():
        raise LauncherError("assigned worktree is not a directory")
    state_root = (
        _absolute_existing(args.state_root, "state root") if args.state_root else None
    )
    terminal_state_path = (
        _socket_path(args.terminal_state_path, "terminal workflow state")
        if args.terminal_state_path
        else None
    )
    if terminal_state_path is not None and state_root is not None:
        try:
            terminal_state_path.relative_to(state_root)
        except ValueError as error:
            raise LauncherError(
                "terminal workflow state is outside the assigned state root"
            ) from error
    if state_root is not None:
        try:
            worktree.relative_to(state_root)
            raise LauncherError("assigned worktree is inside state root")
        except ValueError:
            pass
        try:
            state_root.relative_to(worktree)
            raise LauncherError("state root is inside assigned worktree")
        except ValueError:
            pass
    settings = _absolute_existing(args.settings, "profile settings")
    _load_settings(settings, args.profile)
    copilot = _absolute_existing(args.copilot, "Copilot executable")
    child_arguments = _child_args(args)
    max_agents = _parse_positive(
        args.max_agents
        if args.max_agents is not None
        else os.environ.get("GC_MAX_AGENTS", "2"),
        "max agents",
    )
    max_active_runs = _parse_positive(
        args.max_active_runs
        if args.max_active_runs is not None
        else os.environ.get("GC_MAX_ACTIVE_RUNS", "2"),
        "max active runs",
    )
    runtime_root = _private_directory(args.runtime_root, "agent runtime root")
    home: pathlib.Path | None = materialize_copilot_home(
        settings,
        profile=args.profile,
        runtime_root=runtime_root,
        run_id=run_id,
        bead_id=bead_id,
    )
    environment = scrub_environment(
        profile=args.profile,
        run_id=run_id,
        bead_id=bead_id,
        generation=generation,
        state_schema=state_schema,
    )
    environment["COPILOT_HOME"] = str(home)
    if args.proxy_fd is None:
        for proxy_name in ("ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY"):
            environment.pop(proxy_name, None)
        environment.pop("GC_FDPROXY_FD", None)
    if args.progress_fd is None:
        environment.pop("GC_AGENT_FD", None)
    signal_state = {"stop": False}
    previous_term = signal.signal(
        signal.SIGTERM, lambda signum, frame: _signal_flag(signal_state, signum, frame)
    )
    previous_int = signal.signal(
        signal.SIGINT, lambda signum, frame: _signal_flag(signal_state, signum, frame)
    )
    child: Child | None = None
    active_roots = None
    try:
        with ConcurrencyLease.acquire(
            args.lease_root,
            run_id=run_id,
            bead_id=bead_id,
            max_agents=max_agents,
            max_active_runs=max_active_runs,
        ):
            active_roots = _create_active_run_roots(
                args,
                run_id=run_id,
                bead_id=bead_id,
                generation=generation,
                state_schema=state_schema,
                terminal_state_path=(
                    str(terminal_state_path)
                    if terminal_state_path is not None
                    else None
                ),
            )
            child = _spawn_child(
                profile=args.profile,
                tool_policy=args.tool_policy,
                settings_path=settings,
                copilot=str(copilot),
                child_arguments=child_arguments,
                worktree=worktree,
                state_root=state_root,
                home=home,
                environment=environment,
                args=args,
                proxy_fd=args.proxy_fd,
                progress_fd=args.progress_fd,
                check_fd=args.check_fd,
            )
            relay = ChildRelay(
                child,
                run_id=run_id,
                control_fd=args.control_fd,
                term_grace=args.term_grace,
                kill_grace=args.kill_grace,
                stop_flag=lambda: signal_state["stop"],
            )
            if signal_state["stop"]:
                relay.request_stop("launcher-signal")
            result = relay.run()
            if active_roots is not None and terminal_state_path is not None:
                _cleanup_active_run_roots(active_roots, str(terminal_state_path))
            return result
    finally:
        signal.signal(signal.SIGTERM, previous_term)
        signal.signal(signal.SIGINT, previous_int)
        if child is not None and child.process.poll() is None:
            _send_pid_signal(child.pidfd, child.process.pid, signal.SIGKILL)
            _send_group_signal(child.pgid, signal.SIGKILL)
            child.process.wait()
        try:
            if home is not None and home.is_dir() and not home.is_symlink():
                shutil.rmtree(home)
            elif home is not None and (home.is_symlink() or home.is_file()):
                home.unlink()
        except FileNotFoundError:
            pass


def main() -> int:
    args = _parse_args()
    if args.server:
        return serve_server(args)
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
