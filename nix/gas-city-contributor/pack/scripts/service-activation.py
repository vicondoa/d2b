#!/usr/bin/env python3
"""Preflight, readiness, durable continuation, and active-run GC roots."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import pathlib
import re
import subprocess
import sys
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
IDENTIFIER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")


class ActivationError(RuntimeError):
    """Raised for a closed, actionable activation or continuation failure."""


class StaleGeneration(ActivationError):
    """Raised when state or readiness belongs to a different generation."""


class RootLifecycleError(ActivationError):
    """Raised when active-run GC-root cleanup violates lifecycle ownership."""


@dataclass(frozen=True)
class ProbeResult:
    profile: str
    ok: bool
    model: str | None = None
    context: str | None = None
    effort: str | None = None
    error_code: str | None = None
    error: str | None = None


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


class ActiveRunGCRoots:
    """Create and remove only this run's immutable Nix GC-root symlinks."""

    def __init__(
        self,
        directory: pathlib.Path,
        names: Sequence[str],
        *,
        generation: str | None = None,
        state_schema: str | None = None,
    ):
        self.directory = directory
        self.names = tuple(names)
        self.generation = generation
        self.state_schema = state_schema
        self.terminal = False

    @classmethod
    def create(
        cls,
        root_directory: str | os.PathLike[str],
        *,
        run_id: str,
        generation_paths: Mapping[str, str | os.PathLike[str]],
        allowed_prefixes: Sequence[str] = ("/nix/store/",),
        generation: str | None = None,
        state_schema: str | None = None,
    ) -> "ActiveRunGCRoots":
        _identifier(run_id, "run id")
        if generation is not None and not generation:
            raise RootLifecycleError("GC-root generation is empty")
        if state_schema is not None and not state_schema:
            raise RootLifecycleError("GC-root state schema is empty")
        if set(generation_paths) != set(GC_ROOT_NAMES):
            raise RootLifecycleError("active-run GC roots have an incomplete shape")
        root = pathlib.Path(root_directory)
        if not root.is_absolute() or any(part == ".." for part in root.parts):
            raise RootLifecycleError("GC-root directory must be absolute and normalized")
        directory = root / run_id
        directory.mkdir(mode=0o700, parents=True, exist_ok=False)
        directory_stat = directory.stat()
        if directory_stat.st_uid != os.geteuid() or directory_stat.st_mode & 0o077:
            directory.rmdir()
            raise RootLifecycleError("GC-root directory is not service-owned")
        created: list[pathlib.Path] = []
        try:
            for name in GC_ROOT_NAMES:
                target = pathlib.Path(generation_paths[name])
                target_text = str(target)
                if not target.is_absolute() or any(part == ".." for part in target.parts):
                    raise RootLifecycleError(f"GC target is malformed: {target}")
                if not any(target_text.startswith(prefix) for prefix in allowed_prefixes):
                    raise RootLifecycleError(f"GC target is outside the approved store: {target}")
                if not target.exists():
                    raise RootLifecycleError(f"GC target does not exist: {target}")
                link = directory / name
                os.symlink(target_text, link)
                created.append(link)
        except (OSError, RootLifecycleError):
            for link in reversed(created):
                link.unlink(missing_ok=True)
            directory.rmdir()
            raise
        return cls(
            directory,
            GC_ROOT_NAMES,
            generation=generation,
            state_schema=state_schema,
        )

    def cleanup(self, *, terminal: bool) -> None:
        if self.terminal:
            raise RootLifecycleError("active-run GC roots were already cleaned")
        if not terminal:
            raise RootLifecycleError("active-run GC roots may be removed only at terminal cleanup")
        for name in self.names:
            link = self.directory / name
            if link.is_symlink():
                link.unlink()
            elif link.exists():
                raise RootLifecycleError(f"GC-root path is not a symlink: {link}")
        self.directory.rmdir()
        self.terminal = True


def _probe_environment() -> dict[str, str]:
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
            env=_probe_environment(),
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
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
    activate_parser.add_argument("--timeout", type=float, default=20.0)

    prompt_parser = subparsers.add_parser("reconstruct-prompt")
    prompt_parser.add_argument("--context", required=True)
    prompt_parser.add_argument("--generation", required=True)
    prompt_parser.add_argument("--state-schema", required=True)

    roots_parser = subparsers.add_parser("gc-root-cleanup")
    roots_parser.add_argument("--root-directory", required=True)
    roots_parser.add_argument("--run-id", required=True)
    roots_parser.add_argument("--terminal", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = _parse_args()
    if args.command == "activate":
        def probe(profile: str) -> ProbeResult:
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
            )

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
    if args.command == "gc-root-cleanup":
        _identifier(args.run_id, "run id")
        root_directory = pathlib.Path(args.root_directory)
        if not root_directory.is_absolute() or any(
            part == ".." for part in root_directory.parts
        ):
            raise RootLifecycleError("GC-root directory must be absolute and normalized")
        directory = root_directory / args.run_id
        roots = ActiveRunGCRoots(directory, GC_ROOT_NAMES)
        roots.cleanup(terminal=args.terminal)
        return 0
    raise ActivationError("unknown activation command")


if __name__ == "__main__":
    raise SystemExit(main())
