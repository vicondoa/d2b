#!/usr/bin/env python3
"""Credential-isolated, bundle-based GitHub pull-request publisher.

The main Gas City service creates an anonymous Git bundle and passes its file
descriptor over the private publisher socket.  This process owns the GitHub
App key, imports the bundle into a publisher-owned bare repository, and
publishes only the exact fixed repository and branch ref.  Worktree paths,
Git configuration, hooks, helpers, and ambient credentials are never used by
the publisher.  Provider traffic uses the sidecar's loopback egress proxy.

Publication state is a small restart record for one root bead.  It is
integration state only; the publisher never performs a merge or auto-merge
operation.
"""

from __future__ import annotations

import argparse
import array
import base64
import contextlib
import datetime
import fcntl
import hashlib
import json
import os
import pathlib
import re
import secrets
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Mapping, Sequence
from typing import Any, BinaryIO


PROTOCOL = "gc-publisher/1"
MAX_FRAME_BYTES = 128 * 1024
MAX_BUNDLE_BYTES = 512 * 1024 * 1024
MAX_BODY_BYTES = 64 * 1024
MAX_TITLE_BYTES = 512
MAX_ATTEMPTS = 3
MAX_RETRY_AFTER_SECONDS = 300.0
GITHUB_REQUEST_TIMEOUT_SECONDS = 20.0
INSTALLATION_TOKEN_REFRESH_MARGIN_SECONDS = 60.0
PUBLISHER_OPERATION_BUDGET_SECONDS = MAX_ATTEMPTS * (
    GITHUB_REQUEST_TIMEOUT_SECONDS + MAX_RETRY_AFTER_SECONDS
)
RPC_TIMEOUT_SECONDS = PUBLISHER_OPERATION_BUDGET_SECONDS
DEFAULT_API_BASE = "https://api.github.com"
DEFAULT_REPOSITORY_HOST = "github.com"
DEFAULT_BRANCH_NAMESPACE = "gascity/"
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$")
BRANCH_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._/-]{0,127}$")
IDENTIFIER_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}$")


class PublicationError(RuntimeError):
    """Raised for a permanent publication or protocol failure."""


class RetryableGitHubError(PublicationError):
    """Raised for a provider response that may be retried."""

    def __init__(self, message: str, *, retry_after: float = 0.0) -> None:
        super().__init__(message)
        self.retry_after = max(0.0, float(retry_after))


class GitHubAuthenticationError(PublicationError):
    """Raised when GitHub rejects the bearer used for an API request."""

    def __init__(self, message: str, *, refreshable: bool) -> None:
        super().__init__(message)
        self.refreshable = refreshable


class AmbiguousMutation(PublicationError):
    """Raised when a push or PR mutation may have succeeded."""

    def __init__(self, message: str, *, retry_after: float = 0.0) -> None:
        super().__init__(message)
        self.retry_after = max(0.0, float(retry_after))


class CancelledPublication(PublicationError):
    """Raised when an atomic cancellation marker wins before publication."""


def _string(value: object, label: str, *, max_bytes: int = 512, required: bool = True) -> str:
    if not isinstance(value, str):
        raise PublicationError(f"{label} must be a string")
    value = value.strip()
    if required and not value:
        raise PublicationError(f"{label} must not be empty")
    if len(value.encode("utf-8")) > max_bytes:
        raise PublicationError(f"{label} exceeds the size limit")
    return value


def _identifier(value: object, label: str) -> str:
    value = _string(value, label, max_bytes=128)
    if not IDENTIFIER_PATTERN.fullmatch(value) or ".." in value:
        raise PublicationError(f"{label} is malformed")
    return value


def validate_repository(value: object, *, expected: str | None = None) -> str:
    value = _string(value, "repository", max_bytes=256)
    if not REPOSITORY_PATTERN.fullmatch(value):
        raise PublicationError("repository must be an owner/repository slug")
    if expected is not None and value != expected:
        raise PublicationError("request repository does not match the fixed repository")
    return value


def validate_branch(value: object, label: str, *, namespace: str = "") -> str:
    value = _string(value, label, max_bytes=256)
    if not BRANCH_PATTERN.fullmatch(value):
        raise PublicationError(f"{label} is malformed")
    components = value.split("/")
    if (
        ".." in value
        or "//" in value
        or "\\" in value
        or "@{" in value
        or value.startswith("/")
        or value.endswith("/")
        or any(
            not component
            or component in {".", ".."}
            or component.startswith(".")
            or component.endswith(".lock")
            for component in components
        )
    ):
        raise PublicationError(f"{label} is malformed")
    if namespace and not value.startswith(namespace):
        raise PublicationError(f"{label} is outside the managed branch namespace")
    return value


def validate_https_repository_url(repository: str) -> str:
    repository = validate_repository(repository)
    return f"https://{DEFAULT_REPOSITORY_HOST}/{repository}.git"


def _json_bytes(value: Mapping[str, object]) -> bytes:
    try:
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode(
            "utf-8"
        )
    except (TypeError, ValueError) as error:
        raise PublicationError("publication payload is not JSON-safe") from error
    if len(encoded) > MAX_FRAME_BYTES:
        raise PublicationError("publication payload exceeds the size limit")
    return encoded


def _atomic_write_json(path: pathlib.Path, payload: Mapping[str, object], *, mode: int = 0o600) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        encoded = _json_bytes(payload) + b"\n"
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb", closefd=False) as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        os.close(descriptor)
        pathlib.Path(temporary).unlink(missing_ok=True)


def _read_json(path: pathlib.Path) -> dict[str, object] | None:
    try:
        with path.open("rb") as stream:
            value = json.load(stream)
    except FileNotFoundError:
        return None
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise PublicationError(f"corrupt publication state: {path.name}") from error
    if not isinstance(value, dict):
        raise PublicationError(f"publication state is not an object: {path.name}")
    return value


@contextlib.contextmanager
def _exclusive_lock(path: pathlib.Path, *, mode: int = 0o600):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(
        path,
        os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0),
        mode,
    )
    try:
        os.fchmod(descriptor, mode)
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield descriptor
    finally:
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        os.close(descriptor)


def _retry_after(headers: Mapping[str, str] | None) -> float:
    if headers is None:
        return 0.0
    value = headers.get("Retry-After") or headers.get("retry-after")
    if value:
        try:
            return max(0.0, min(MAX_RETRY_AFTER_SECONDS, float(value)))
        except (TypeError, ValueError):
            pass
    reset = headers.get("X-RateLimit-Reset") or headers.get("x-ratelimit-reset")
    if reset:
        try:
            return max(0.0, min(MAX_RETRY_AFTER_SECONDS, float(reset) - time.time()))
        except (TypeError, ValueError):
            pass
    return 0.0


def _github_timestamp(value: object, label: str) -> float:
    text = _string(value, label, max_bytes=64)
    try:
        parsed = datetime.datetime.fromisoformat(text.replace("Z", "+00:00"))
    except (TypeError, ValueError) as error:
        raise PublicationError(f"{label} is malformed") from error
    if parsed.tzinfo is None:
        raise PublicationError(f"{label} must include a timezone")
    try:
        return parsed.timestamp()
    except (OverflowError, OSError, ValueError) as error:
        raise PublicationError(f"{label} is malformed") from error


def validate_publication_request(
    value: object,
    *,
    repository: str,
    base_branch: str,
    branch_namespace: str = DEFAULT_BRANCH_NAMESPACE,
    installation_id: str | None = None,
    app_id: str | None = None,
    cancel_root: str | os.PathLike[str] | None = None,
) -> dict[str, object]:
    if not isinstance(value, dict):
        raise PublicationError("publication request must be an object")
    allowed = {
        "protocol",
        "run_id",
        "bead_id",
        "repository",
        "base",
        "head",
        "head_sha",
        "branch_namespace",
        "worktree_id",
        "title",
        "body",
        "installation_id",
        "app_id",
        "cancel_path",
    }
    if set(value) - allowed:
        raise PublicationError("publication request contains unsupported fields")
    if value.get("protocol", PROTOCOL) != PROTOCOL:
        raise PublicationError("publisher protocol version mismatch")
    fixed_repository = validate_repository(repository)
    request_repository = validate_repository(value.get("repository"), expected=fixed_repository)
    fixed_base = validate_branch(base_branch, "configured base branch")
    base = validate_branch(value.get("base"), "base branch")
    if base != fixed_base:
        raise PublicationError("request base branch does not match the fixed base")
    request_namespace = _string(
        value.get("branch_namespace", branch_namespace),
        "branch namespace",
        max_bytes=64,
    )
    if request_namespace != branch_namespace:
        raise PublicationError("request branch namespace does not match the fixed namespace")
    head = validate_branch(value.get("head"), "head branch", namespace=branch_namespace)
    if head == base:
        raise PublicationError("head branch must not equal the base branch")
    run_id = _identifier(value.get("run_id"), "run_id")
    bead_id = _identifier(value.get("bead_id"), "bead_id")
    worktree_id = _identifier(value.get("worktree_id"), "worktree_id")
    title = _string(value.get("title"), "pull-request title", max_bytes=MAX_TITLE_BYTES)
    body = _string(value.get("body", ""), "pull-request body", max_bytes=MAX_BODY_BYTES, required=False)
    request_installation = _string(
        value.get("installation_id"),
        "installation_id",
        max_bytes=64,
        required=False,
    )
    request_app = _string(value.get("app_id"), "app_id", max_bytes=64, required=False)
    if installation_id is not None and request_installation != installation_id:
        raise PublicationError("request installation does not match the fixed installation")
    if app_id is not None and request_app != app_id:
        raise PublicationError("request app does not match the fixed application")
    head_sha = value.get("head_sha", "")
    if head_sha:
        head_sha = _string(head_sha, "head_sha", max_bytes=64)
        if not re.fullmatch(r"[0-9a-fA-F]{40,64}", head_sha):
            raise PublicationError("head_sha is malformed")
        head_sha = head_sha.lower()
    cancel_path = value.get("cancel_path", "")
    if cancel_path:
        cancel_path = _string(cancel_path, "cancel_path", max_bytes=512)
        if cancel_root is not None:
            root = pathlib.Path(cancel_root).resolve()
            candidate = pathlib.Path(cancel_path)
            if (
                not candidate.is_absolute()
                or candidate.parent != root
                or candidate.name != f"{run_id}.json"
            ):
                raise PublicationError("cancel_path is outside the fixed cancellation namespace")
    return {
        "protocol": PROTOCOL,
        "run_id": run_id,
        "bead_id": bead_id,
        "repository": request_repository,
        "base": base,
        "head": head,
        "head_sha": head_sha,
        "branch_namespace": request_namespace,
        "worktree_id": worktree_id,
        "title": title,
        "body": body,
        "installation_id": request_installation,
        "app_id": request_app,
        "cancel_path": cancel_path,
    }


class PublicationStore:
    """Durable state for one root bead and its exact publication identity."""

    def __init__(self, state_root: str | os.PathLike[str]) -> None:
        self.root = pathlib.Path(state_root).resolve()
        self.records = self.root / "publications"
        self.locks = self.root / "locks"
        self.records.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.locks.mkdir(mode=0o700, parents=True, exist_ok=True)

    @staticmethod
    def key(run_id: str) -> str:
        return f"{_identifier(run_id, 'run_id')}.json"

    def path(self, run_id: str) -> pathlib.Path:
        return self.records / self.key(run_id)

    def lock_path(self, run_id: str) -> pathlib.Path:
        return self.locks / f"{self.key(run_id)}.lock"

    def get(self, run_id: str) -> dict[str, object] | None:
        return _read_json(self.path(run_id))

    def write(self, record: Mapping[str, object]) -> dict[str, object]:
        run_id = _identifier(record.get("run_id"), "run_id")
        with _exclusive_lock(self.lock_path(run_id)):
            return self.write_locked(record)

    def write_locked(self, record: Mapping[str, object]) -> dict[str, object]:
        """Write a record while the caller owns this run's lock."""

        run_id = _identifier(record.get("run_id"), "run_id")
        current = _read_json(self.path(run_id))
        merged = {**(current or {}), **dict(record), "updated_at": int(time.time())}
        _atomic_write_json(self.path(run_id), merged)
        return merged


class GitRunner:
    """Git command runner with a closed configuration and helper surface."""

    def __init__(
        self,
        *,
        git: str = "git",
        subprocess_run: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
    ) -> None:
        self.git = git
        self.subprocess_run = subprocess_run

    @staticmethod
    def environment() -> dict[str, str]:
        allowed = {
            "PATH": os.environ.get("PATH", ""),
            "LANG": "C",
            "LC_ALL": "C",
            "HOME": "/var/lib/gascity-publisher/home",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_GLOBAL": "/dev/null",
            "GIT_CONFIG_SYSTEM": "/dev/null",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_SSH_COMMAND": "/bin/false",
            "GIT_PROXY_COMMAND": "/bin/false",
            "GIT_ALLOW_PROTOCOL": "https:file",
        }
        for variable in ("HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"):
            value = os.environ.get(variable)
            if value:
                allowed[variable] = value
        return allowed

    @classmethod
    def authenticated_environment(cls, token: str) -> dict[str, str]:
        environment = cls.environment()
        if token:
            environment.update(
                {
                    "GIT_CONFIG_COUNT": "1",
                    "GIT_CONFIG_KEY_0": "http.extraHeader",
                }
            )
            basic = base64.b64encode(f"x-access-token:{token}".encode("utf-8")).decode("ascii")
            environment["GIT_CONFIG_VALUE_0"] = "Authorization: Basic " + basic
        return environment

    def run(
        self,
        arguments: Sequence[str],
        *,
        cwd: str | os.PathLike[str] | None = None,
        pass_fds: Sequence[int] = (),
        input: str | None = None,
        timeout: float = 60.0,
        environment: Mapping[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        if not arguments or arguments[0] != self.git:
            command = [self.git, *arguments]
        else:
            command = list(arguments)
        try:
            return self.subprocess_run(
                command,
                cwd=str(cwd) if cwd is not None else None,
                env=dict(environment or self.environment()),
                input=input,
                capture_output=True,
                text=True,
                check=False,
                timeout=timeout,
                pass_fds=tuple(pass_fds),
            )
        except subprocess.TimeoutExpired as error:
            raise AmbiguousMutation("Git command outcome is ambiguous") from error

    def checked(
        self,
        arguments: Sequence[str],
        *,
        cwd: str | os.PathLike[str] | None = None,
        pass_fds: Sequence[int] = (),
        timeout: float = 60.0,
    ) -> str:
        result = self.run(arguments, cwd=cwd, pass_fds=pass_fds, timeout=timeout)
        if result.returncode != 0:
            detail = (result.stderr or result.stdout or "").strip()[-1024:]
            raise PublicationError(f"git command failed: {detail or result.returncode}")
        return result.stdout

    def configure_bare(self, repository: pathlib.Path, remote_url: str) -> None:
        self.checked(["init", "--bare", str(repository)])
        settings = {
            "core.hooksPath": "/dev/null",
            "credential.helper": "",
            "remote.origin.url": remote_url,
            "remote.origin.pushurl": remote_url,
            "remote.origin.fetch": "+refs/heads/*:refs/remotes/origin/*",
            "remote.origin.uploadpack": "git-upload-pack",
            "remote.origin.receivepack": "git-receive-pack",
        }
        for key, value in settings.items():
            self.checked(["-C", str(repository), "config", "--local", "--replace-all", key, value])
        for key in ("http.proxy", "https.proxy"):
            result = self.run(
                ["-C", str(repository), "config", "--local", "--unset-all", key]
            )
            if result.returncode not in {0, 5}:
                detail = (result.stderr or result.stdout or "").strip()[-1024:]
                raise PublicationError(
                    f"git proxy configuration cleanup failed: {detail or result.returncode}"
                )

    def list_heads(
        self,
        bundle_path: str,
        *,
        bundle_fd: int | None = None,
    ) -> list[tuple[str, str]]:
        output = self.checked(
            ["bundle", "list-heads", bundle_path],
            pass_fds=(bundle_fd,) if bundle_fd is not None else (),
        )
        heads: list[tuple[str, str]] = []
        for line in output.splitlines():
            fields = line.split()
            if len(fields) != 2 or not re.fullmatch(r"[0-9a-fA-F]{40,64}", fields[0]):
                raise PublicationError("bundle contains a malformed head")
            heads.append((fields[0].lower(), fields[1]))
        if not heads:
            raise PublicationError("bundle contains no heads")
        return heads

    def import_bundle(
        self,
        repository: pathlib.Path,
        bundle_fd: int,
        *,
        head: str,
        expected_sha: str = "",
    ) -> str:
        if bundle_fd < 3:
            raise PublicationError("bundle descriptor must not overlap stdio")
        os.set_inheritable(bundle_fd, False)
        try:
            size = os.fstat(bundle_fd).st_size
            if size <= 0 or size > MAX_BUNDLE_BYTES:
                raise PublicationError("bundle size is outside the allowed bound")
        except OSError as error:
            raise PublicationError("bundle descriptor is not readable") from error
        bundle_path = f"/proc/self/fd/{bundle_fd}"
        heads = self.list_heads(bundle_path, bundle_fd=bundle_fd)
        expected_ref = f"refs/heads/{head}"
        matching = [(sha, ref) for sha, ref in heads if ref == expected_ref]
        if len(matching) != 1 or len(heads) != 1:
            raise PublicationError("bundle contains an unexpected ref")
        bundle_sha = matching[0][0]
        if expected_sha and bundle_sha != expected_sha.lower():
            raise PublicationError("bundle head does not match bounded metadata")
        self.checked(
            [
                "-C",
                str(repository),
                "fetch",
                "--no-tags",
                "--no-prune",
                bundle_path,
                f"{expected_ref}:{expected_ref}",
            ],
            pass_fds=(bundle_fd,),
            timeout=120,
        )
        imported = self.checked(["-C", str(repository), "rev-parse", f"{expected_ref}"]).strip()
        if imported != bundle_sha:
            raise PublicationError("publisher imported an unexpected head")
        return imported

    def remote_head(self, repository: pathlib.Path, head: str, token: str = "") -> str:
        result = self.run(
            [
                "-C",
                str(repository),
                "ls-remote",
                "origin",
                f"refs/heads/{head}",
            ],
            environment=self.authenticated_environment(token),
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout or "").strip()[-1024:]
            if "timed out" in detail.lower() or "connection reset" in detail.lower():
                raise AmbiguousMutation("Git remote lookup outcome is ambiguous")
            raise RetryableGitHubError(f"Git remote lookup failed: {detail or result.returncode}")
        output = result.stdout
        fields = output.split()
        if not fields:
            return ""
        if len(fields) != 2 or fields[1] != f"refs/heads/{head}":
            raise PublicationError("remote returned an unexpected ref")
        if not re.fullmatch(r"[0-9a-fA-F]{40,64}", fields[0]):
            raise PublicationError("remote returned a malformed head")
        return fields[0].lower()

    def push(self, repository: pathlib.Path, head: str, token: str = "") -> None:
        arguments = [
            "-C",
            str(repository),
            "push",
            "--porcelain",
            "--no-thin",
            "origin",
            f"refs/heads/{head}:refs/heads/{head}",
        ]
        # Supplying the header through Git's ephemeral config channel keeps it
        # out of the remote URL and out of the bare clone config.
        environment = self.authenticated_environment(token)
        try:
            result = self.subprocess_run(
                [self.git, *arguments],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
                timeout=120,
                pass_fds=(),
            )
        except subprocess.TimeoutExpired as error:
            raise AmbiguousMutation("Git push outcome is ambiguous") from error
        if result.returncode != 0:
            detail = (result.stderr or result.stdout or "").strip()[-1024:]
            if "timed out" in detail.lower() or "connection reset" in detail.lower():
                raise AmbiguousMutation("Git push outcome is ambiguous")
            raise RetryableGitHubError(f"Git push failed: {detail or result.returncode}")


class GitHubAPI:
    """Small GitHub App REST client with bounded provider retries."""

    def __init__(
        self,
        *,
        app_id: str,
        installation_id: str,
        private_key_path: str,
        api_base: str = DEFAULT_API_BASE,
        opener: Callable[..., Any] | None = None,
        sleep: Callable[[float], None] = time.sleep,
        openssl: str = "openssl",
    ) -> None:
        self.app_id = _string(app_id, "GitHub app id", max_bytes=64)
        self.installation_id = _string(installation_id, "GitHub installation id", max_bytes=64)
        self.private_key_path = pathlib.Path(private_key_path)
        parsed = urllib.parse.urlparse(api_base)
        if (
            parsed.scheme != "https"
            or not parsed.netloc
            or parsed.username is not None
            or parsed.password is not None
            or parsed.query
            or parsed.fragment
        ):
            raise PublicationError("GitHub API base must use HTTPS")
        self.api_base = api_base.rstrip("/")
        self.opener = opener or urllib.request.build_opener(
            urllib.request.ProxyHandler()
        ).open
        self.sleep = sleep
        self.openssl = openssl
        self._installation_token = ""
        self._installation_token_expires_at = 0.0

    def _jwt(self) -> str:
        now = int(time.time())
        header = _base64url(json.dumps({"alg": "RS256", "typ": "JWT"}, separators=(",", ":")).encode())
        claims = _base64url(
            json.dumps(
                {"iat": now - 60, "exp": now + 540, "iss": self.app_id},
                separators=(",", ":"),
            ).encode()
        )
        unsigned = f"{header}.{claims}".encode("ascii")
        result = subprocess.run(
            [self.openssl, "dgst", "-sha256", "-sign", str(self.private_key_path)],
            input=unsigned,
            capture_output=True,
            check=False,
            timeout=10,
            env={
                "PATH": os.environ.get("PATH", ""),
                "HOME": "/var/lib/gascity-publisher/home",
                "LANG": "C",
                "LC_ALL": "C",
                "OPENSSL_CONF": "/dev/null",
            },
        )
        if result.returncode != 0:
            raise PublicationError("GitHub App key could not sign an authentication token")
        return f"{header}.{claims}.{_base64url(result.stdout)}"

    def _request_once(
        self,
        method: str,
        path: str,
        *,
        payload: Mapping[str, object] | None = None,
        token: str = "",
        bearer: bool = False,
        mutating: bool = False,
    ) -> tuple[int, dict[str, object] | list[object]]:
        if not path.startswith("/") or ".." in path:
            raise PublicationError("GitHub API path is malformed")
        body = None
        headers = {
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "gascity-contributor/1",
        }
        if token:
            headers["Authorization"] = f"{'Bearer' if bearer else 'token'} {token}"
        if payload is not None:
            body = _json_bytes(payload)
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            f"{self.api_base}{path}",
            data=body,
            headers=headers,
            method=method,
        )
        try:
            with self.opener(request, timeout=GITHUB_REQUEST_TIMEOUT_SECONDS) as response:
                status = int(response.status)
                if status == 401:
                    raise GitHubAuthenticationError(
                        "GitHub authentication failed",
                        refreshable=not bearer and bool(token),
                    )
                raw = response.read(MAX_FRAME_BYTES + 1)
                if len(raw) > MAX_FRAME_BYTES:
                    raise PublicationError("GitHub response exceeds the size limit")
                if not raw:
                    return status, {}
                value = json.loads(raw)
                if not isinstance(value, (dict, list)):
                    raise PublicationError("GitHub response is not a JSON object or list")
                return status, value
        except urllib.error.HTTPError as error:
            if error.code == 401:
                raise GitHubAuthenticationError(
                    "GitHub authentication failed",
                    refreshable=not bearer and bool(token),
                ) from error
            if error.code == 429 or error.code >= 500:
                if mutating and error.code >= 500:
                    raise AmbiguousMutation(
                        f"GitHub mutating request returned HTTP {error.code}",
                        retry_after=_retry_after(error.headers),
                    ) from error
                raise RetryableGitHubError(
                    f"GitHub returned HTTP {error.code}",
                    retry_after=_retry_after(error.headers),
                ) from error
            raise PublicationError(f"GitHub returned permanent HTTP {error.code}") from error
        except (urllib.error.URLError, TimeoutError, socket.timeout, OSError) as error:
            raise AmbiguousMutation("GitHub request outcome is ambiguous") from error

    def installation_token(self, *, force_refresh: bool = False) -> str:
        now = time.time()
        if (
            not force_refresh
            and self._installation_token
            and now + INSTALLATION_TOKEN_REFRESH_MARGIN_SECONDS
            < self._installation_token_expires_at
        ):
            return self._installation_token
        status, response = self._request_once(
            "POST",
            f"/app/installations/{urllib.parse.quote(self.installation_id, safe='')}/access_tokens",
            token=self._jwt(),
            bearer=True,
        )
        if status < 200 or status >= 300 or not isinstance(response, dict):
            raise PublicationError("GitHub installation token response is malformed")
        token = response.get("token")
        expires_at = _github_timestamp(
            response.get("expires_at"),
            "GitHub installation token expiry",
        )
        parsed_token = _string(token, "GitHub installation token", max_bytes=4096)
        self._installation_token = parsed_token
        self._installation_token_expires_at = expires_at
        return self._installation_token

    def _invalidate_installation_token(self, token: str) -> None:
        if token and token == self._installation_token:
            self._installation_token = ""
            self._installation_token_expires_at = 0.0

    def request(
        self,
        method: str,
        path: str,
        *,
        payload: Mapping[str, object] | None = None,
        mutating: bool = False,
    ) -> dict[str, object] | list[object]:
        forced_refresh = False
        for attempt in range(1, MAX_ATTEMPTS + 1):
            token = ""
            try:
                token = self.installation_token()
                status, response = self._request_once(
                    method,
                    path,
                    payload=payload,
                    token=token,
                    mutating=mutating,
                )
                if status < 200 or status >= 300:
                    raise PublicationError(f"GitHub returned unexpected HTTP {status}")
                return response
            except GitHubAuthenticationError as error:
                if attempt == MAX_ATTEMPTS or not error.refreshable or forced_refresh:
                    raise PublicationError("GitHub authentication failed") from error
                self._invalidate_installation_token(token)
                self.installation_token(force_refresh=True)
                forced_refresh = True
            except RetryableGitHubError as error:
                if attempt == MAX_ATTEMPTS:
                    raise PublicationError("GitHub retry ceiling reached") from error
                self.sleep(error.retry_after)
            except AmbiguousMutation as error:
                # A mutating request is reconciled by its caller using exact
                # repository/head/base identity.  Blind retrying here could
                # create a second pull request after an accepted response.
                if mutating:
                    raise
                if attempt == MAX_ATTEMPTS:
                    raise PublicationError("GitHub read retry ceiling reached") from error
                self.sleep(error.retry_after)
        raise AssertionError("GitHub request loop exhausted")

    def repository_identity(self, repository: str) -> dict[str, object]:
        value = self.request("GET", f"/repos/{urllib.parse.quote(repository, safe='/')}")
        if not isinstance(value, dict):
            raise PublicationError("GitHub repository response is malformed")
        if value.get("full_name") != repository:
            raise PublicationError("GitHub repository identity does not match the fixed slug")
        return value

    def installation_identity(self, repository: str) -> dict[str, object]:
        value = self.request(
            "GET",
            f"/repos/{urllib.parse.quote(repository, safe='/')}/installation",
        )
        if not isinstance(value, dict):
            raise PublicationError("GitHub installation response is malformed")
        raw_id = value.get("id")
        if str(raw_id) != self.installation_id:
            raise PublicationError("GitHub repository is attached to a different installation")
        return value

    def find_pull_requests(self, repository: str, *, head: str, base: str) -> list[dict[str, object]]:
        query = urllib.parse.urlencode(
            {"state": "all", "base": base, "per_page": "100"}
        )
        value = self.request(
            "GET",
            f"/repos/{urllib.parse.quote(repository, safe='/')}/pulls?{query}",
        )
        if not isinstance(value, list):
            raise PublicationError("GitHub pull-request lookup response is malformed")
        matches: list[dict[str, object]] = []
        for item in value:
            if not isinstance(item, dict):
                continue
            head_data = item.get("head") if isinstance(item.get("head"), dict) else {}
            base_data = item.get("base") if isinstance(item.get("base"), dict) else {}
            if head_data.get("ref") == head and base_data.get("ref") == base:
                matches.append(item)
        return matches

    def create_pull_request(
        self,
        repository: str,
        *,
        head: str,
        base: str,
        title: str,
        body: str,
    ) -> dict[str, object]:
        value = self.request(
            "POST",
            f"/repos/{urllib.parse.quote(repository, safe='/')}/pulls",
            payload={"head": head, "base": base, "title": title, "body": body},
            mutating=True,
        )
        if not isinstance(value, dict):
            raise PublicationError("GitHub pull-request creation response is malformed")
        return value


def _base64url(value: bytes) -> str:
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode("ascii")


def _pr_url(value: Mapping[str, object], repository: str) -> str:
    url = value.get("html_url") or value.get("url")
    if not isinstance(url, str):
        raise PublicationError("GitHub pull request has no URL")
    parsed = urllib.parse.urlparse(url)
    expected_path = f"/{repository}/pull/"
    number = parsed.path[len(expected_path) :] if parsed.path.startswith(expected_path) else ""
    if number.endswith("/"):
        number = number[:-1]
    if (
        parsed.scheme != "https"
        or parsed.netloc != DEFAULT_REPOSITORY_HOST
        or parsed.params
        or parsed.query
        or parsed.fragment
        or not number.isdigit()
    ):
        raise PublicationError("GitHub pull-request URL is outside the fixed repository")
    return url


def _is_merged(value: Mapping[str, object]) -> bool:
    state = str(value.get("state", "")).lower()
    return (
        bool(value.get("merged"))
        or state == "merged"
        or (state == "closed" and value.get("merged_at") is not None)
    )


class Publisher:
    """Restart-safe publication state machine."""

    def __init__(
        self,
        *,
        state: PublicationStore,
        git: GitRunner,
        github: Any,
        repository: str,
        base_branch: str,
        branch_namespace: str = DEFAULT_BRANCH_NAMESPACE,
        cancellation_root: str | os.PathLike[str] | None = None,
        app_id: str | None = None,
        installation_id: str | None = None,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        self.repository = validate_repository(repository)
        self.base_branch = validate_branch(base_branch, "base branch")
        self.branch_namespace = _string(branch_namespace, "branch namespace", max_bytes=64)
        if not self.branch_namespace.endswith("/"):
            raise PublicationError("branch namespace must end with '/'")
        self.app_id = _string(app_id, "GitHub app id", max_bytes=64) if app_id is not None else ""
        self.installation_id = (
            _string(installation_id, "GitHub installation id", max_bytes=64)
            if installation_id is not None
            else ""
        )
        self.state = state
        self.git = git
        self.github = github
        self.cancellation_root = pathlib.Path(cancellation_root).resolve() if cancellation_root else None
        self.sleep = sleep

    def _cancel_path(self, record: Mapping[str, object]) -> pathlib.Path | None:
        supplied = str(record.get("cancel_path") or "")
        if self.cancellation_root is None:
            if supplied:
                raise PublicationError("request cancellation path is unavailable")
            return None
        path = self.cancellation_root / f"{record['run_id']}.json"
        if supplied and pathlib.Path(supplied) != path:
            raise PublicationError("request cancellation path does not match the fixed run path")
        return path

    def _check_cancelled(self, record: Mapping[str, object]) -> None:
        path = self._cancel_path(record)
        if path is not None:
            try:
                info = path.lstat()
            except FileNotFoundError:
                return
            except OSError as error:
                raise PublicationError("cancellation marker is unreadable") from error
            if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o002:
                raise PublicationError("cancellation marker is not a private regular file")
            self.state.write_locked(self._state_record(record, phase="cancelled"))
            raise CancelledPublication("publication was cancelled before its next mutation")

    @contextlib.contextmanager
    def _mutation_guard(self, record: Mapping[str, object]):
        """Serialize cancellation-marker creation with each external mutation."""

        if self.cancellation_root is None:
            self._check_cancelled(record)
            yield
            return
        with _exclusive_lock(self.cancellation_root / ".lock", mode=0o660):
            self._check_cancelled(record)
            yield

    def _state_record(self, request: Mapping[str, object], *, phase: str, **extra: object) -> dict[str, object]:
        return {
            "protocol": PROTOCOL,
            "run_id": request["run_id"],
            "bead_id": request["bead_id"],
            "repository": request["repository"],
            "base": request["base"],
            "head": request["head"],
            "head_sha": request.get("head_sha", ""),
            "worktree_id": request["worktree_id"],
            "branch_namespace": request["branch_namespace"],
            "title": request["title"],
            "body": request["body"],
            "installation_id": request["installation_id"],
            "app_id": request["app_id"],
            "cancel_path": request["cancel_path"],
            "phase": phase,
            **extra,
        }

    def _existing_pr(self, record: Mapping[str, object]) -> str | None:
        matches = self.github.find_pull_requests(
            str(record["repository"]),
            head=str(record["head"]),
            base=str(record["base"]),
        )
        if len(matches) > 1:
            raise PublicationError("multiple exact pull requests match the publication")
        if not matches:
            return None
        pull = matches[0]
        head_data = pull.get("head") if isinstance(pull.get("head"), dict) else {}
        base_data = pull.get("base") if isinstance(pull.get("base"), dict) else {}
        head_repo = head_data.get("repo") if isinstance(head_data.get("repo"), dict) else {}
        base_repo = base_data.get("repo") if isinstance(base_data.get("repo"), dict) else {}
        if (
            head_data.get("ref") != str(record["head"])
            or base_data.get("ref") != str(record["base"])
            or head_repo.get("full_name") != str(record["repository"])
            or base_repo.get("full_name") != str(record["repository"])
        ):
            raise PublicationError("pull-request match is outside the exact repository and refs")
        expected_sha = str(record.get("head_sha") or "").lower()
        actual_sha = str(head_data.get("sha") or "").lower()
        if expected_sha and actual_sha != expected_sha:
            raise PublicationError("existing pull request head diverges from the bundle")
        if _is_merged(pull):
            return _pr_url(pull, str(record["repository"]))
        state = str(pull.get("state", "")).lower()
        if state == "open":
            return _pr_url(pull, str(record["repository"]))
        raise PublicationError("a closed unmerged pull request blocks publication")

    def _validate_provider_identity(self) -> None:
        repository_identity = getattr(self.github, "repository_identity", None)
        if callable(repository_identity):
            value = repository_identity(self.repository)
            if not isinstance(value, Mapping) or value.get("full_name") != self.repository:
                raise PublicationError("GitHub repository identity does not match the fixed repository")
        installation_identity = getattr(self.github, "installation_identity", None)
        if callable(installation_identity):
            value = installation_identity(self.repository)
            if not isinstance(value, Mapping):
                raise PublicationError("GitHub installation identity response is malformed")
            if self.installation_id and str(value.get("id")) != self.installation_id:
                raise PublicationError("GitHub installation identity does not match the fixed installation")

    def publish(self, value: object, bundle_fd: int) -> dict[str, object]:
        request = validate_publication_request(
            value,
            repository=self.repository,
            base_branch=self.base_branch,
            branch_namespace=self.branch_namespace,
            installation_id=self.installation_id or None,
            app_id=self.app_id or None,
            cancel_root=self.cancellation_root,
        )
        run_id = str(request["run_id"])
        with _exclusive_lock(self.state.lock_path(run_id)):
            current = self.state.get(run_id)
            pushed = bool(current.get("pushed")) if current is not None else False
            if current is not None:
                for field in ("repository", "base", "head", "head_sha", "worktree_id"):
                    if current.get(field) != request.get(field):
                        raise PublicationError("publication restart identity diverges")
                for field in (
                    "branch_namespace",
                    "title",
                    "body",
                    "installation_id",
                    "app_id",
                    "cancel_path",
                ):
                    if field in current and current.get(field) != request.get(field):
                        raise PublicationError("publication restart metadata diverges")
                if current.get("phase") == "complete":
                    return current
                if current.get("phase") == "cancelled":
                    raise CancelledPublication("publication was cancelled")
            self._check_cancelled(request)
            self._validate_provider_identity()
            publisher_repo = self.state.root / "bare" / re.sub(r"[^A-Za-z0-9_.-]", "-", self.repository)
            publisher_repo.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            if not publisher_repo.exists():
                self.git.configure_bare(publisher_repo, validate_https_repository_url(self.repository))
            else:
                # Re-assert the fixed local configuration on every restart.
                self.git.configure_bare(publisher_repo, validate_https_repository_url(self.repository))
            self.state.write_locked(self._state_record(request, phase="validated"))
            imported_sha = self.git.import_bundle(
                publisher_repo,
                bundle_fd,
                head=str(request["head"]),
                expected_sha=str(request.get("head_sha") or ""),
            )
            request["head_sha"] = imported_sha
            record = self.state.write_locked(
                self._state_record(request, phase="bundle-imported", head_sha=imported_sha)
            )
            self._check_cancelled(record)

            existing = self._existing_pr(record)
            if existing is None:
                pushed = False
                token = ""
                for attempt in range(1, MAX_ATTEMPTS + 1):
                    self._check_cancelled(record)
                    try:
                        with self._mutation_guard(record):
                            token = self.github.installation_token()
                            self.git.push(publisher_repo, str(request["head"]), token)
                        pushed = True
                        break
                    except AmbiguousMutation as error:
                        try:
                            remote = self.git.remote_head(
                                publisher_repo,
                                str(request["head"]),
                                token,
                            )
                        except (AmbiguousMutation, RetryableGitHubError) as reconcile_error:
                            if attempt == MAX_ATTEMPTS:
                                raise PublicationError("ambiguous push reconciliation reached its retry ceiling")
                            self.sleep(
                                max(
                                    error.retry_after,
                                    getattr(reconcile_error, "retry_after", 0.0),
                                )
                            )
                            continue
                        if remote == imported_sha:
                            pushed = True
                            break
                        if remote:
                            raise PublicationError("ambiguous push reconciled to a divergent head")
                        if attempt == MAX_ATTEMPTS:
                            raise PublicationError("ambiguous push did not reconcile to the exact head")
                        self.sleep(error.retry_after)
                    except RetryableGitHubError as error:
                        if attempt == MAX_ATTEMPTS:
                            raise PublicationError("push retry ceiling reached") from error
                        self.sleep(error.retry_after)
                if not pushed:
                    raise PublicationError("publisher did not complete the exact push")
                record = self.state.write_locked(
                    self._state_record(request, phase="pushed", head_sha=imported_sha)
                )
                self._check_cancelled(record)
                for attempt in range(1, MAX_ATTEMPTS + 1):
                    try:
                        existing = self._existing_pr(record)
                        break
                    except RetryableGitHubError as error:
                        if attempt == MAX_ATTEMPTS:
                            raise PublicationError("PR lookup retry ceiling reached") from error
                        self.sleep(error.retry_after)
                if existing is None:
                    for attempt in range(1, MAX_ATTEMPTS + 1):
                        self._check_cancelled(record)
                        try:
                            with self._mutation_guard(record):
                                created = self.github.create_pull_request(
                                    str(request["repository"]),
                                    head=str(request["head"]),
                                    base=str(request["base"]),
                                    title=str(request["title"]),
                                    body=str(request["body"]),
                                )
                            existing = _pr_url(created, str(request["repository"]))
                            break
                        except AmbiguousMutation as error:
                            # Never blindly create a second PR after an
                            # uncertain response.  Exact lookup is the only
                            # reconciliation allowed before another attempt.
                            try:
                                existing = self._existing_pr(record)
                            except RetryableGitHubError as lookup_error:
                                if attempt == MAX_ATTEMPTS:
                                    raise PublicationError(
                                        "ambiguous PR lookup reached its retry ceiling"
                                    ) from lookup_error
                                self.sleep(
                                    max(error.retry_after, lookup_error.retry_after)
                                )
                                continue
                            if existing is not None:
                                break
                            if attempt == MAX_ATTEMPTS:
                                raise PublicationError("ambiguous PR creation remained unresolved")
                            self.sleep(error.retry_after)
                        except RetryableGitHubError as error:
                            if attempt == MAX_ATTEMPTS:
                                raise PublicationError("PR creation retry ceiling reached") from error
                            self.sleep(error.retry_after)
            if existing is None:
                raise PublicationError("publisher could not obtain a pull-request URL")
            self._check_cancelled(record)
            final = self.state.write_locked(
                self._state_record(
                    request,
                    phase="complete",
                    head_sha=imported_sha,
                    pr_url=existing,
                    pushed=pushed,
                )
            )
            return final


def _peer_uid(connection: socket.socket) -> int:
    raw = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
    _pid, uid, _gid = struct_unpack_three_ints(raw)
    return uid


def struct_unpack_three_ints(value: bytes) -> tuple[int, int, int]:
    import struct

    return struct.unpack("3i", value)


def _open_listener(path: str, group: str) -> socket.socket:
    target = pathlib.Path(path)
    if (
        not target.is_absolute()
        or os.path.normpath(path) != path
        or any(part == ".." for part in target.parts)
        or target.is_symlink()
    ):
        raise PublicationError("publisher socket path must be an absolute non-symlink")
    for ancestor in target.parents:
        if ancestor == ancestor.parent:
            break
        if ancestor.is_symlink():
            raise PublicationError("publisher socket path has a symlinked ancestor")
    target = target.absolute()
    target.parent.mkdir(mode=0o770, parents=True, exist_ok=True)
    if os.path.lexists(target):
        info = os.lstat(target)
        if not stat.S_ISSOCK(info.st_mode):
            raise PublicationError("publisher socket is occupied by a non-socket")
        target.unlink()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(str(target))
    os.chmod(target, 0o660)
    try:
        import grp

        os.chown(target, -1, grp.getgrnam(group).gr_gid)
    except (KeyError, OSError) as error:
        listener.close()
        target.unlink(missing_ok=True)
        raise PublicationError("publisher socket group is unavailable") from error
    listener.listen(32)
    return listener


def _read_frame(connection: socket.socket) -> dict[str, object]:
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = connection.recv(min(4096, MAX_FRAME_BYTES - len(data)))
        if not chunk:
            raise PublicationError("publisher channel closed")
        data.extend(chunk)
        if len(data) > MAX_FRAME_BYTES:
            raise PublicationError("publisher channel frame exceeds the size limit")
    try:
        value = json.loads(bytes(data))
    except json.JSONDecodeError as error:
        raise PublicationError("publisher channel frame is not JSON") from error
    if not isinstance(value, dict):
        raise PublicationError("publisher channel frame must be an object")
    return value


def _write_frame(connection: socket.socket, value: Mapping[str, object]) -> None:
    connection.sendall(_json_bytes(value) + b"\n")


def _rpc_with_fd(socket_path: str, request: Mapping[str, object], descriptor: int) -> dict[str, object]:
    if descriptor < 3:
        raise PublicationError("bundle descriptor must not overlap stdio")
    try:
        os.fstat(descriptor)
    except OSError as error:
        raise PublicationError("bundle descriptor is unavailable") from error
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        connection.settimeout(RPC_TIMEOUT_SECONDS)
        connection.connect(socket_path)
        payload = _json_bytes(request) + b"\n"
        rights = array.array("i", [descriptor])
        sent = connection.sendmsg([payload], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, rights)])
        if sent <= 0:
            raise PublicationError("publisher channel did not accept the request")
        if sent < len(payload):
            connection.sendall(payload[sent:])
        return _read_frame(connection)
    finally:
        connection.close()


class PublisherServer:
    def __init__(
        self,
        *,
        socket_path: str,
        socket_group: str,
        credential_path: str,
        state_root: str,
        repository: str,
        base_branch: str,
        branch_namespace: str,
        app_id: str,
        installation_id: str,
        cancellation_root: str | os.PathLike[str] | None,
        allowed_uid: int = 45100,
        api_base: str = DEFAULT_API_BASE,
    ) -> None:
        _validate_private_key_path(credential_path)
        self.socket_path = socket_path
        self.socket_group = socket_group
        self.allowed_uid = allowed_uid
        self.publisher = Publisher(
            state=PublicationStore(state_root),
            git=GitRunner(),
            github=GitHubAPI(
                app_id=app_id,
                installation_id=installation_id,
                private_key_path=credential_path,
                api_base=api_base,
            ),
            repository=repository,
            base_branch=base_branch,
            branch_namespace=branch_namespace,
            cancellation_root=cancellation_root,
            app_id=app_id,
            installation_id=installation_id,
        )

    def serve(self) -> None:
        listener = _open_listener(self.socket_path, self.socket_group)
        try:
            while True:
                connection, _ = listener.accept()
                self._serve_connection(connection)
        finally:
            listener.close()
            pathlib.Path(self.socket_path).unlink(missing_ok=True)

    def _serve_connection(self, connection: socket.socket) -> None:
        try:
            if _peer_uid(connection) != self.allowed_uid:
                raise PublicationError("publisher channel peer is unauthorized")
            request, descriptor = _recv_request_and_fd(connection)
            try:
                if request.get("protocol", PROTOCOL) != PROTOCOL:
                    raise PublicationError("publisher protocol version mismatch")
                if request.get("operation") != "publish":
                    raise PublicationError("unknown publisher operation")
                result = self.publisher.publish(request.get("request"), descriptor)
                response = {"protocol": PROTOCOL, "ok": True, "result": result}
            finally:
                os.close(descriptor)
        except (PublicationError, OSError, ValueError) as error:
            response = {"protocol": PROTOCOL, "ok": False, "error": str(error)[:1024]}
        try:
            _write_frame(connection, response)
        except OSError:
            pass
        finally:
            connection.close()


def _recv_request_and_fd(connection: socket.socket) -> tuple[dict[str, object], int]:
    descriptors: list[int] = []
    data = bytearray()
    try:
        while b"\n" not in data:
            chunk, ancillary, flags, _address = connection.recvmsg(
                min(4096, MAX_FRAME_BYTES - len(data)),
                socket.CMSG_SPACE(array.array("i", [0]).itemsize * 4),
            )
            if flags & getattr(socket, "MSG_CTRUNC", 0):
                raise PublicationError("publisher channel ancillary data was truncated")
            for level, kind, payload in ancillary:
                if level != socket.SOL_SOCKET or kind != socket.SCM_RIGHTS:
                    raise PublicationError("publisher channel sent unauthorized ancillary data")
                values = array.array("i")
                if len(payload) % values.itemsize:
                    raise PublicationError("publisher channel sent malformed descriptor data")
                values.frombytes(payload)
                descriptors.extend(values.tolist())
            if not chunk:
                raise PublicationError("publisher channel request is empty or incomplete")
            data.extend(chunk)
            if len(data) >= MAX_FRAME_BYTES:
                raise PublicationError("publisher channel request exceeds the size limit")
        frame, remainder = bytes(data).split(b"\n", 1)
        if remainder:
            raise PublicationError("publisher channel request contains trailing data")
        try:
            request = json.loads(frame)
        except json.JSONDecodeError as error:
            raise PublicationError("publisher channel request is not JSON") from error
        if not isinstance(request, dict):
            raise PublicationError("publisher channel request must be an object")
        if len(descriptors) != 1:
            raise PublicationError("publisher request must carry exactly one bundle descriptor")
        os.set_inheritable(descriptors[0], False)
        return request, descriptors[0]
    except BaseException:
        for descriptor in descriptors:
            os.close(descriptor)
        raise


def _validate_private_key_path(value: str) -> pathlib.Path:
    path = pathlib.Path(_string(value, "GitHub private key path", max_bytes=512))
    if not path.is_absolute():
        raise PublicationError("GitHub private key path must be absolute")
    for ancestor in path.parents:
        if ancestor == ancestor.parent:
            break
        if ancestor.is_symlink():
            raise PublicationError("GitHub private key path has a symlinked ancestor")
    try:
        info = path.stat()
    except OSError as error:
        raise PublicationError("GitHub private key path is unavailable") from error
    if not stat.S_ISREG(info.st_mode) or path.is_symlink() or info.st_mode & 0o022:
        raise PublicationError("GitHub private key path is not a private regular file")
    return path


def _read_worktree_value(path: str, label: str) -> str:
    path = _string(path, label, max_bytes=1024)
    source = pathlib.Path(path)
    if not source.is_absolute() or source.is_symlink():
        raise PublicationError(f"{label} is not a managed directory")
    for ancestor in source.parents:
        if ancestor == ancestor.parent:
            break
        if ancestor.is_symlink():
            raise PublicationError(f"{label} has a symlinked ancestor")
    candidate = source.resolve()
    if not candidate.is_dir() or candidate.is_symlink():
        raise PublicationError(f"{label} is not a managed directory")
    return str(candidate)


def _git_output(git: str, worktree: str, *arguments: str) -> str:
    environment = GitRunner.environment()
    result = subprocess.run(
        [
            git,
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "credential.helper=",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.ssh.allow=never",
            "-C",
            worktree,
            *arguments,
        ],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=60,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()[-1024:]
        raise PublicationError(f"worktree git command failed: {detail or result.returncode}")
    return result.stdout.strip()


def create_unlinked_bundle(
    *,
    worktree: str,
    head: str,
    base: str,
    branch_namespace: str = DEFAULT_BRANCH_NAMESPACE,
    git: str = "git",
) -> tuple[BinaryIO, str]:
    """Create an unlinked bundle for one exact committed branch."""

    branch_namespace = _string(branch_namespace, "branch namespace", max_bytes=64)
    if not branch_namespace.endswith("/"):
        raise PublicationError("branch namespace must end with '/'")
    worktree_path = _read_worktree_value(worktree, "worktree")
    head = validate_branch(head, "head branch", namespace=branch_namespace)
    base = validate_branch(base, "base branch")
    current_branch = _git_output(git, worktree_path, "symbolic-ref", "--quiet", "--short", "HEAD")
    if current_branch != head:
        raise PublicationError("worktree branch does not match the bounded head")
    status = _git_output(git, worktree_path, "status", "--porcelain", "--untracked-files=all")
    if status:
        raise PublicationError("worktree is dirty")
    head_sha = _git_output(git, worktree_path, "rev-parse", "--verify", f"refs/heads/{head}")
    base_sha = _git_output(git, worktree_path, "rev-parse", "--verify", f"refs/remotes/origin/{base}")
    if head_sha == base_sha:
        raise PublicationError("head commit must differ from base commit")
    temporary_directory = tempfile.mkdtemp(prefix="gascity-bundle-")
    bundle_path = pathlib.Path(temporary_directory) / "branch.bundle"
    try:
        result = subprocess.run(
            [
                git,
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "credential.helper=",
                "-c",
                "protocol.ext.allow=never",
                "-C",
                worktree_path,
                "bundle",
                "create",
                str(bundle_path),
                f"refs/heads/{head}",
            ],
            env={**GitRunner.environment(), "GIT_ALLOW_PROTOCOL": "file"},
            capture_output=True,
            text=True,
            check=False,
            timeout=120,
        )
        if result.returncode != 0:
            detail = (result.stderr or result.stdout or "").strip()[-1024:]
            raise PublicationError(
                f"git bundle creation failed: {detail or result.returncode}"
            )
        descriptor = os.open(
            bundle_path,
            os.O_RDONLY | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0),
        )
        bundle = os.fdopen(descriptor, "rb")
        bundle_path.unlink()
        bundle_size = os.fstat(descriptor).st_size
        if bundle_size <= 0 or bundle_size > MAX_BUNDLE_BYTES:
            bundle.close()
            raise PublicationError("created bundle exceeds the size limit")
        return bundle, head_sha
    finally:
        bundle_path.unlink(missing_ok=True)
        pathlib.Path(temporary_directory).rmdir()


def _rpc_response(response: Mapping[str, object]) -> dict[str, object]:
    if not response.get("ok"):
        raise PublicationError(str(response.get("error") or "publisher rejected request"))
    result = response.get("result")
    if not isinstance(result, dict):
        raise PublicationError("publisher returned an invalid result")
    return result


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="operation", required=True)

    serve = subparsers.add_parser("serve")
    serve.add_argument("--socket", required=True)
    serve.add_argument("--socket-group", default="gascity-publisher-channel")
    serve.add_argument("--credential", required=True)
    serve.add_argument("--state-root", required=True)
    serve.add_argument("--repository", required=True)
    serve.add_argument("--base-branch", required=True)
    serve.add_argument("--branch-namespace", default=os.environ.get("GC_BRANCH_NAMESPACE", DEFAULT_BRANCH_NAMESPACE))
    serve.add_argument("--app-id", required=True)
    serve.add_argument("--installation-id", required=True)
    serve.add_argument("--cancellation-root", default=os.environ.get("GC_CANCEL_ROOT", ""))
    serve.add_argument("--api-base", default=os.environ.get("GC_GITHUB_API_BASE", DEFAULT_API_BASE))
    serve.add_argument("--allowed-uid", type=int, default=45100)

    publish = subparsers.add_parser("request")
    publish.add_argument("--socket", default=os.environ.get("GC_PUBLISHER_CHANNEL_SOCKET", ""))
    publish.add_argument("--worktree", required=True)
    publish.add_argument("--worktree-id", required=True)
    publish.add_argument("--run-id", required=True)
    publish.add_argument("--bead-id", required=True)
    publish.add_argument("--repository", required=True)
    publish.add_argument("--base", required=True)
    publish.add_argument("--head", required=True)
    publish.add_argument("--title", required=True)
    publish.add_argument("--body", default="")
    publish.add_argument("--installation-id", default=os.environ.get("GC_GITHUB_INSTALLATION_ID", ""))
    publish.add_argument("--app-id", default=os.environ.get("GC_GITHUB_APP_ID", ""))
    publish.add_argument("--branch-namespace", default=os.environ.get("GC_BRANCH_NAMESPACE", DEFAULT_BRANCH_NAMESPACE))
    publish.add_argument("--cancellation-root", default=os.environ.get("GC_CANCEL_ROOT", ""))
    publish.add_argument("--discord-socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))
    publish.add_argument("--notification", default="")
    publish.add_argument("--no-notify", action="store_true")
    return parser.parse_args(argv)


def _notify_discord(socket_path: str, body: str) -> dict[str, object] | None:
    if not socket_path or not body:
        return None
    body = _string(body, "Discord notification", max_bytes=8 * 1024)
    import json as _json

    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        connection.settimeout(30)
        connection.connect(socket_path)
        connection.sendall(
            _json.dumps(
                {
                    "protocol": "gc-discord-decision/1",
                    "operation": "publication-notify",
                    "body": body,
                },
                separators=(",", ":"),
            ).encode()
            + b"\n"
        )
        data = bytearray()
        while not data.endswith(b"\n"):
            chunk = connection.recv(min(4096, MAX_FRAME_BYTES - len(data)))
            if not chunk:
                raise PublicationError("Discord notification channel closed")
            data.extend(chunk)
            if len(data) > MAX_FRAME_BYTES:
                raise PublicationError("Discord notification response exceeds the size limit")
        response = _json.loads(bytes(data))
        if not isinstance(response, dict) or not response.get("ok"):
            raise PublicationError("Discord notification was rejected")
        result = response.get("result")
        return result if isinstance(result, dict) else {}
    finally:
        connection.close()


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    if args.operation == "serve":
        cancellation_root = args.cancellation_root or None
        PublisherServer(
            socket_path=args.socket,
            socket_group=args.socket_group,
            credential_path=args.credential,
            state_root=args.state_root,
            repository=args.repository,
            base_branch=args.base_branch,
            branch_namespace=args.branch_namespace,
            app_id=args.app_id,
            installation_id=args.installation_id,
            cancellation_root=cancellation_root,
            allowed_uid=args.allowed_uid,
            api_base=args.api_base,
        ).serve()
        return 0
    if not args.socket:
        raise PublicationError("publisher socket is required")
    bundle, head_sha = create_unlinked_bundle(
        worktree=args.worktree,
        head=args.head,
        base=args.base,
        branch_namespace=args.branch_namespace,
    )
    try:
        cancellation_root = pathlib.Path(args.cancellation_root).resolve() if args.cancellation_root else None
        cancel_path = str(cancellation_root / f"{args.run_id}.json") if cancellation_root else ""
        request = {
            "protocol": PROTOCOL,
            "operation": "publish",
            "request": {
                "protocol": PROTOCOL,
                "run_id": args.run_id,
                "bead_id": args.bead_id,
                "repository": args.repository,
                "base": args.base,
                "head": args.head,
                "head_sha": head_sha,
                "branch_namespace": args.branch_namespace,
                "worktree_id": args.worktree_id,
                "title": args.title,
                "body": args.body,
                "installation_id": args.installation_id,
                "app_id": args.app_id,
                "cancel_path": cancel_path,
            },
        }
        response = _rpc_with_fd(args.socket, request, bundle.fileno())
        result = _rpc_response(response)
        pr_url = str(result.get("pr_url", ""))
        notification = None
        if not args.no_notify and pr_url and args.notification and args.discord_socket:
            notification = _notify_discord(
                args.discord_socket,
                args.notification.replace("{pr_url}", pr_url),
            )
        output = {"publication": result, "discord_notification": notification}
        print(json.dumps(output, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
        return 0
    finally:
        bundle.close()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (PublicationError, OSError) as error:
        print(f"pull-request publication rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
