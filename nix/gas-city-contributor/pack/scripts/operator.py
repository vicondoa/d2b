#!/usr/bin/env python3
"""Bounded operator wrappers for the Gas City contributor service."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import stat
import subprocess
import sys
from typing import Any


MAX_REQUEST_BYTES = 64 * 1024
GASCITY_UID = 45100
IDENTIFIER = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")
SAFE_REQUEST_KEYS = {
    "submit": {"run_id", "bead_id", "summary", "base_branch", "repository"},
    "cancel": {"run_id", "reason"},
}
FORBIDDEN_KEYS = {
    "token",
    "password",
    "secret",
    "private_key",
    "credential",
    "api_key",
}


class OperatorError(RuntimeError):
    """Raised for malformed or unauthorized operator input."""


def _identifier(value: object, label: str) -> str:
    if not isinstance(value, str) or not IDENTIFIER.fullmatch(value) or ".." in value:
        raise OperatorError(f"{label} is malformed")
    return value


def _bounded_json(stream: Any) -> dict[str, object]:
    source = getattr(stream, "buffer", stream)
    payload = source.read(MAX_REQUEST_BYTES + 1)
    if isinstance(payload, str):
        payload = payload.encode("utf-8")
    if len(payload) > MAX_REQUEST_BYTES:
        raise OperatorError("operator request exceeds the size limit")
    try:
        value = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise OperatorError("operator request is not valid JSON") from error
    if not isinstance(value, dict):
        raise OperatorError("operator request must be a JSON object")
    if any(key.lower() in FORBIDDEN_KEYS for key in value):
        raise OperatorError("operator request contains a credential field")
    return value


def validate_request(operation: str, value: dict[str, object]) -> dict[str, object]:
    if operation not in SAFE_REQUEST_KEYS:
        raise OperatorError("unknown operator operation")
    if set(value) - SAFE_REQUEST_KEYS[operation]:
        raise OperatorError("operator request contains unsupported fields")
    _identifier(value.get("run_id"), "run_id")
    if operation == "submit":
        _identifier(value.get("bead_id"), "bead_id")
        repository = value.get("repository")
        branch = value.get("base_branch")
        summary = value.get("summary")
        if (
            not isinstance(repository, str)
            or not re.fullmatch(r"[A-Za-z0-9_.-]{1,100}/[A-Za-z0-9_.-]{1,100}", repository)
            or not isinstance(branch, str)
            or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/-]{0,127}", branch)
            or ".." in branch
            or not isinstance(summary, str)
            or not summary.strip()
            or len(summary.encode("utf-8")) > 16 * 1024
        ):
            raise OperatorError("submit request contains an invalid repository, branch, or summary")
    else:
        reason = value.get("reason", "")
        if not isinstance(reason, str) or len(reason.encode("utf-8")) > 4096:
            raise OperatorError("cancel reason is malformed")
    return dict(value)


def _request_directory() -> pathlib.Path:
    path = pathlib.Path("/run/gascity-contributor/operator-requests")
    path.mkdir(mode=0o770, parents=True, exist_ok=True)
    info = os.lstat(path)
    if (
        stat.S_ISLNK(info.st_mode)
        or not stat.S_ISDIR(info.st_mode)
        or info.st_uid != 0
        or info.st_mode & 0o002
    ):
        raise OperatorError("operator request directory has unsafe ownership or mode")
    return path


def write_request(operation: str, request: dict[str, object]) -> pathlib.Path:
    directory = _request_directory()
    run_id = _identifier(request.get("run_id"), "run_id")
    target = directory / f"{run_id}.{operation}.json"
    if target.exists() or target.is_symlink():
        raise OperatorError("an operator request for this run already exists")
    descriptor = os.open(
        target,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0),
        0o660,
    )
    try:
        os.fchmod(descriptor, 0o660)
        encoded = json.dumps(
            {"operation": operation, "request": request},
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
        os.write(descriptor, encoded)
        os.write(descriptor, b"\n")
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return target


def _systemctl(*arguments: str) -> subprocess.CompletedProcess[str]:
    command = "/run/current-system/sw/bin/systemctl"
    return subprocess.run(
        [command, "--system", *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=10,
        env={"PATH": "/run/current-system/sw/bin", "LANG": "C"},
    )


def status() -> dict[str, object]:
    result = _systemctl(
        "show",
        "gas-city-contributor.service",
        "--property=ActiveState,SubState,MainPID,Result",
        "--no-pager",
    )
    values: dict[str, str] = {}
    for line in result.stdout.splitlines():
        key, separator, value = line.partition("=")
        if separator and key in {"ActiveState", "SubState", "MainPID", "Result"}:
            values[key] = value[:128]
    return {
        "ok": result.returncode == 0,
        "service": "gas-city-contributor.service",
        "state": values,
        "error": None if result.returncode == 0 else "systemd-status-failed",
    }


def _operation_from_argv(argv0: str, argument: str | None) -> str:
    if argument:
        operation = argument
    else:
        operation = {
            "gascity-submit": "submit",
            "gascity-status": "status",
            "gascity-cancel": "cancel",
        }.get(pathlib.Path(argv0).name, "")
    if operation not in {"submit", "status", "cancel"}:
        raise OperatorError("operator operation must be submit, status, or cancel")
    return operation


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", nargs="?")
    args = parser.parse_args(argv)
    operation = _operation_from_argv(sys.argv[0], args.operation)
    if os.geteuid() != GASCITY_UID:
        raise OperatorError("operator wrappers must be invoked through the scoped sudo rule")
    if operation == "status":
        print(json.dumps(status(), sort_keys=True, separators=(",", ":")))
        return 0
    request = validate_request(operation, _bounded_json(sys.stdin))
    path = write_request(operation, request)
    print(
        json.dumps(
            {"accepted": True, "operation": operation, "request": str(path)},
            sort_keys=True,
            separators=(",", ":"),
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OperatorError, OSError, subprocess.SubprocessError) as error:
        print(f"operator request rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
