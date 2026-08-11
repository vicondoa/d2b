#!/usr/bin/env python3
"""Durable, outbound-only Discord decision routing for Gas City.

The Discord process is the only process that reads the bot token.  Gas City
talks to it over the private Unix socket and receives only bounded, validated
decision data.  A gate bead remains the authority for accepting a choice:
the sidecar records a pending event, the workflow performs the beads
conditional update, and the workflow acknowledges the result here.

The persisted prompt record is integration state, not an approval ledger.  It
contains only the correlation data needed to reconcile delivery and restart.
No approval record, signature, evidence, or decision-history artifact is written.
"""

from __future__ import annotations

import argparse
import base64
import contextlib
import fcntl
import hashlib
import json
import math
import os
import pathlib
import secrets
import socket
import ssl
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Mapping
from typing import Any


PROTOCOL = "gc-discord-decision/1"
MAX_FRAME_BYTES = 128 * 1024
MAX_PROMPT_BYTES = 16 * 1024
MAX_BODY_BYTES = 8 * 1024
MAX_CHOICES = 32
MAX_CHOICE_BYTES = 128
MAX_DELIVERY_ATTEMPTS = 3
DEFAULT_API_BASE = "https://discord.com/api/v10"
IDLE_WAIT_SECONDS = 0.25
IDENTIFIER_ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"


def _loopback_proxy() -> tuple[str, int]:
    value = os.environ.get("HTTPS_PROXY") or os.environ.get("HTTP_PROXY")
    if not value:
        raise DecisionError("Discord egress proxy is not configured")
    parsed = urllib.parse.urlparse(value)
    if (
        parsed.scheme != "http"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.path not in {"", "/"}
        or parsed.query
        or parsed.fragment
    ):
        raise DecisionError("Discord egress proxy must be a loopback HTTP proxy")
    host = parsed.hostname
    if host not in {"127.0.0.1", "::1"}:
        raise DecisionError("Discord egress proxy must be loopback-only")
    try:
        port = parsed.port or 3128
    except ValueError as error:
        raise DecisionError("Discord egress proxy port is malformed") from error
    if not 1 <= port <= 65535:
        raise DecisionError("Discord egress proxy port is outside the TCP range")
    return host, port


class DecisionError(RuntimeError):
    """Raised when a decision request or event is rejected."""


class RetryableDiscordError(DecisionError):
    """Raised for a provider response that may be retried."""

    def __init__(self, message: str, *, retry_after: float = 0.0) -> None:
        super().__init__(message)
        self.retry_after = max(0.0, float(retry_after))


class AmbiguousSend(DecisionError):
    """Raised when Discord may have accepted a message before the connection failed."""

    def __init__(self, message: str, *, retry_after: float = 0.0) -> None:
        super().__init__(message)
        self.retry_after = max(0.0, float(retry_after))


class ConflictError(DecisionError):
    """Raised when a different first answer already won the gate."""


class PeerError(DecisionError):
    """Raised when a caller is not the main Gas City service."""


def _string(value: object, label: str, *, max_bytes: int = 512, required: bool = True) -> str:
    if not isinstance(value, str):
        raise DecisionError(f"{label} must be a string")
    value = value.strip()
    if required and not value:
        raise DecisionError(f"{label} must not be empty")
    if len(value.encode("utf-8")) > max_bytes:
        raise DecisionError(f"{label} exceeds the size limit")
    return value


def _identifier(value: object, label: str, *, max_bytes: int = 128) -> str:
    value = _string(value, label, max_bytes=max_bytes)
    if ".." in value or "/" in value or "\\" in value:
        raise DecisionError(f"{label} is malformed")
    if not all(character.isalnum() or character in "_.:-" for character in value):
        raise DecisionError(f"{label} is malformed")
    if not value[0].isalnum():
        raise DecisionError(f"{label} is malformed")
    return value


def _discord_id(value: object, label: str) -> str:
    value = _string(value, label, max_bytes=32)
    if not value.isdigit() or int(value) <= 0:
        raise DecisionError(f"{label} is malformed")
    return value


def _choice(value: object, label: str = "choice") -> str:
    value = _string(value, label, max_bytes=MAX_CHOICE_BYTES)
    if not all(character.isalnum() or character in "_.:-" for character in value):
        raise DecisionError(f"{label} is malformed")
    return value


def _json_bytes(value: Mapping[str, object]) -> bytes:
    try:
        encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode(
            "utf-8"
        )
    except (TypeError, ValueError) as error:
        raise DecisionError("decision payload is not JSON-safe") from error
    if len(encoded) > MAX_FRAME_BYTES:
        raise DecisionError("decision payload exceeds the size limit")
    return encoded


def _truncate_utf8(value: str, max_bytes: int) -> str:
    encoded = value.encode("utf-8")
    if len(encoded) <= max_bytes:
        return value
    return encoded[:max_bytes].decode("utf-8", errors="ignore")


def _atomic_write_json(path: pathlib.Path, payload: Mapping[str, object], *, mode: int = 0o600) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    encoded = _json_bytes(payload) + b"\n"
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
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
        raise DecisionError(f"corrupt durable decision state: {path.name}") from error
    if not isinstance(value, dict):
        raise DecisionError(f"durable decision state is not an object: {path.name}")
    return value


@contextlib.contextmanager
def _exclusive_lock(path: pathlib.Path):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(
        path,
        os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0),
        0o600,
    )
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield descriptor
    finally:
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        os.close(descriptor)


def _event_id(value: object) -> str:
    return _string(value, "gateway event id", max_bytes=256)


def _prompt_marker(record: Mapping[str, object]) -> str:
    nonce = _string(record.get("prompt_nonce"), "prompt_nonce", max_bytes=256)
    decision_id = _identifier(record.get("decision_id"), "decision_id")
    run_id = _identifier(record.get("run_id"), "run_id")
    digest = hashlib.sha256(f"{run_id}\0{decision_id}\0{nonce}".encode("utf-8")).hexdigest()[:24]
    return f"gc-decision:{run_id}:{decision_id}:{nonce}:{digest}"


def _normalise_choices(value: object) -> list[str]:
    if not isinstance(value, list) or not value or len(value) > MAX_CHOICES:
        raise DecisionError("choices must be a non-empty bounded list")
    result: list[str] = []
    seen: set[str] = set()
    for item in value:
        item = _choice(item)
        folded = item.casefold()
        if folded in seen:
            raise DecisionError("choices must be unique")
        seen.add(folded)
        result.append(item)
    return result


def validate_prompt_request(
    value: object,
    *,
    expected_guild_id: str | None = None,
    expected_channel_id: str | None = None,
    expected_operator_ids: set[str] | None = None,
) -> dict[str, object]:
    if not isinstance(value, dict):
        raise DecisionError("prompt request must be an object")
    allowed = {
        "protocol",
        "run_id",
        "bead_id",
        "decision_id",
        "prompt_nonce",
        "assignee",
        "guild_id",
        "channel_id",
        "message",
        "choices",
    }
    if set(value) - allowed:
        raise DecisionError("prompt request contains unsupported fields")
    if value.get("protocol", PROTOCOL) != PROTOCOL:
        raise DecisionError("decision protocol version mismatch")
    run_id = _identifier(value.get("run_id"), "run_id")
    bead_id = _identifier(value.get("bead_id"), "bead_id")
    decision_id = _identifier(value.get("decision_id"), "decision_id")
    guild_id = _discord_id(value.get("guild_id"), "guild_id")
    channel_id = _discord_id(value.get("channel_id"), "channel_id")
    if expected_guild_id is not None and guild_id != expected_guild_id:
        raise DecisionError("guild is not allowed")
    if expected_channel_id is not None and channel_id != expected_channel_id:
        raise DecisionError("channel is not allowed")
    if expected_operator_ids is not None and not expected_operator_ids:
        raise DecisionError("operator allowlist is empty")
    prompt_nonce = value.get("prompt_nonce")
    if prompt_nonce is None or prompt_nonce == "":
        prompt_nonce = secrets.token_urlsafe(18)
    prompt_nonce = _identifier(prompt_nonce, "prompt_nonce", max_bytes=256)
    assignee = _identifier(value.get("assignee"), "assignee")
    message = _string(value.get("message"), "message", max_bytes=MAX_BODY_BYTES)
    choices = _normalise_choices(value.get("choices"))
    return {
        "protocol": PROTOCOL,
        "run_id": run_id,
        "bead_id": bead_id,
        "decision_id": decision_id,
        "prompt_nonce": prompt_nonce,
        "assignee": assignee,
        "guild_id": guild_id,
        "channel_id": channel_id,
        "message": message,
        "choices": choices,
    }


def _nested_id(value: object, key: str) -> str:
    if isinstance(value, dict):
        nested = value.get(key)
        return str(nested).strip() if nested is not None else ""
    return ""


def normalize_gateway_event(value: object) -> dict[str, object]:
    if not isinstance(value, dict):
        raise DecisionError("gateway event must be an object")
    dispatch = value.get("type") or value.get("t") or value.get("event_type") or "MESSAGE_CREATE"
    dispatch = _string(dispatch, "gateway event type", max_bytes=64)
    data = value.get("d") if isinstance(value.get("d"), dict) else value
    if not isinstance(data, dict):
        raise DecisionError("gateway event data must be an object")
    raw_event_id = value.get("event_id", value.get("sequence", value.get("s")))
    if raw_event_id is None:
        raw_event_id = data.get("event_id")
    if raw_event_id is None:
        raise DecisionError("gateway event id is required")
    event_id = _event_id(raw_event_id if isinstance(raw_event_id, str) else f"gateway:{raw_event_id}")
    author = data.get("author")
    author_id = data.get("author_id") or _nested_id(author, "id")
    reference = data.get("message_reference")
    reply_to = data.get("reply_to_message_id") or _nested_id(reference, "message_id")
    message_id = data.get("message_id") or data.get("id")
    result: dict[str, object] = {
        "protocol": PROTOCOL,
        "event_id": event_id,
        "event_type": dispatch,
        "guild_id": str(data.get("guild_id", "")).strip(),
        "channel_id": str(data.get("channel_id", "")).strip(),
        "operator_id": str(author_id or "").strip(),
        "message_id": str(message_id or "").strip(),
        "reply_to_message_id": str(reply_to or "").strip(),
        "run_id": str(data.get("run_id", "")).strip(),
        "decision_id": str(data.get("decision_id", "")).strip(),
        "prompt_nonce": str(data.get("prompt_nonce", "")).strip(),
        "choice": data.get("choice"),
        "content": data.get("content", ""),
        "edited": dispatch in {"MESSAGE_UPDATE", "MESSAGE_EDIT"} or bool(data.get("edited")),
    }
    if result["edited"] or data.get("edited_timestamp") is not None:
        result["edited"] = True
    return result


def validate_gateway_event(
    event: object,
    record: Mapping[str, object],
    *,
    operator_ids: set[str],
) -> dict[str, object]:
    normalized = normalize_gateway_event(event)
    if normalized["event_type"] not in {"MESSAGE_CREATE", "message_create", "message"}:
        raise DecisionError("edited or unsupported gateway event")
    if normalized["edited"]:
        raise DecisionError("message edits cannot answer a decision")
    expected_event_id = _event_id(normalized["event_id"])
    guild_id = _discord_id(normalized["guild_id"], "event guild_id")
    channel_id = _discord_id(normalized["channel_id"], "event channel_id")
    operator_id = _discord_id(normalized["operator_id"], "event operator_id")
    message_id = _discord_id(normalized["message_id"], "event message_id")
    reply_to = _discord_id(normalized["reply_to_message_id"], "reply target")
    if guild_id != str(record.get("guild_id")):
        raise DecisionError("event guild does not match the prompt")
    if channel_id != str(record.get("channel_id")):
        raise DecisionError("event channel does not match the prompt")
    if operator_id not in operator_ids:
        raise DecisionError("event operator is unauthorized")
    if reply_to != str(record.get("message_id")):
        raise DecisionError("event does not reply to the active prompt")
    for field in ("run_id", "decision_id", "prompt_nonce"):
        supplied = str(normalized.get(field) or "").strip()
        if supplied and supplied != str(record.get(field)):
            raise DecisionError(f"event {field} does not match the prompt")
    choice_value = normalized.get("choice")
    if choice_value is None:
        content = normalized.get("content")
        if not isinstance(content, str):
            raise DecisionError("event choice is malformed")
        content = content.strip()
        if content.lower().startswith("choice:"):
            content = content.split(":", 1)[1].strip()
        choice_value = content
    choice_value = _choice(choice_value)
    choices = {str(item).casefold(): str(item) for item in record.get("choices", [])}
    if choice_value.casefold() not in choices:
        raise DecisionError("event choice is not declared by the prompt")
    return {
        **normalized,
        "event_id": expected_event_id,
        "guild_id": guild_id,
        "channel_id": channel_id,
        "operator_id": operator_id,
        "message_id": message_id,
        "reply_to_message_id": reply_to,
        "choice": choices[choice_value.casefold()],
    }


class DecisionStore:
    """Small locked store for one durable prompt record per gate."""

    def __init__(self, state_root: str | os.PathLike[str]) -> None:
        self.root = pathlib.Path(state_root).resolve()
        self.prompts = self.root / "prompts"
        self.locks = self.root / "locks"
        self.prompts.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.locks.mkdir(mode=0o700, parents=True, exist_ok=True)

    @staticmethod
    def key(run_id: str, decision_id: str) -> str:
        return f"{_identifier(run_id, 'run_id')}.{_identifier(decision_id, 'decision_id')}.json"

    def path(self, run_id: str, decision_id: str) -> pathlib.Path:
        return self.prompts / self.key(run_id, decision_id)

    def _lock_path(self, run_id: str, decision_id: str) -> pathlib.Path:
        return self.locks / f"{self.key(run_id, decision_id)}.lock"

    def get(self, run_id: str, decision_id: str) -> dict[str, object] | None:
        return _read_json(self.path(run_id, decision_id))

    def ensure_prompt(self, request: Mapping[str, object]) -> dict[str, object]:
        run_id = _identifier(request.get("run_id"), "run_id")
        decision_id = _identifier(request.get("decision_id"), "decision_id")
        path = self.path(run_id, decision_id)
        lock_path = self.locks / f"{path.name}.lock"
        with _exclusive_lock(lock_path):
            existing = _read_json(path)
            if existing is not None:
                immutable = (
                    "run_id",
                    "bead_id",
                    "decision_id",
                    "prompt_nonce",
                    "assignee",
                    "guild_id",
                    "channel_id",
                    "message",
                    "choices",
                )
                if any(existing.get(field) != request.get(field) for field in immutable):
                    raise ConflictError("a different prompt already owns this gate")
                return existing
            record = {
                **dict(request),
                "state": "delivery",
                "message_id": "",
                "event_id": "",
                "answer": "",
                "pending_answer": None,
                "delivery_attempts": 0,
                "created_at": int(time.time()),
                "updated_at": int(time.time()),
            }
            _atomic_write_json(path, record)
            return record

    def update(self, record: Mapping[str, object]) -> dict[str, object]:
        run_id = _identifier(record.get("run_id"), "run_id")
        decision_id = _identifier(record.get("decision_id"), "decision_id")
        path = self.path(run_id, decision_id)
        lock_path = self.locks / f"{path.name}.lock"
        with _exclusive_lock(lock_path):
            current = _read_json(path)
            if current is None:
                raise DecisionError("prompt state does not exist")
            merged = {**current, **dict(record), "updated_at": int(time.time())}
            _atomic_write_json(path, merged)
            return merged

    def reserve_answer(self, event: Mapping[str, object]) -> dict[str, object]:
        run_id = _identifier(event.get("run_id"), "run_id")
        decision_id = _identifier(event.get("decision_id"), "decision_id")
        path = self.path(run_id, decision_id)
        lock_path = self.locks / f"{path.name}.lock"
        with _exclusive_lock(lock_path):
            record = _read_json(path)
            if record is None:
                raise DecisionError("orphaned decision prompt")
            state = str(record.get("state", ""))
            answer = str(record.get("answer", ""))
            if state in {"answered", "closed"}:
                if answer.casefold() == str(event.get("choice", "")).casefold():
                    return {**record, "router_status": "duplicate"}
                raise ConflictError("a different answer already won this gate")
            pending = record.get("pending_answer")
            if isinstance(pending, dict):
                if (
                    str(pending.get("event_id")) == str(event.get("event_id"))
                    and str(pending.get("choice")).casefold() == str(event.get("choice")).casefold()
                ):
                    return {**record, "router_status": "pending"}
                if str(pending.get("choice")).casefold() != str(event.get("choice")).casefold():
                    raise ConflictError("a different answer is already pending")
            else:
                pending = {
                    "event_id": str(event["event_id"]),
                    "choice": str(event["choice"]),
                    "operator_id": str(event["operator_id"]),
                    "received_at": int(time.time()),
                }
                record["pending_answer"] = pending
                record["state"] = "answer-pending"
                record["updated_at"] = int(time.time())
                _atomic_write_json(path, record)
            return {**record, "router_status": "pending"}

    def acknowledge(
        self,
        run_id: str,
        decision_id: str,
        *,
        event_id: str,
        choice: str,
        accepted: bool,
    ) -> dict[str, object]:
        path = self.path(run_id, decision_id)
        lock_path = self.locks / f"{path.name}.lock"
        with _exclusive_lock(lock_path):
            record = _read_json(path)
            if record is None:
                raise DecisionError("orphaned decision prompt")
            state = str(record.get("state", ""))
            answer = str(record.get("answer", ""))
            if state in {"answered", "closed"}:
                if answer.casefold() == choice.casefold():
                    return {**record, "router_status": "duplicate"}
                raise ConflictError("acknowledgement conflicts with the first answer")
            pending = record.get("pending_answer")
            if not isinstance(pending, dict):
                raise DecisionError("no pending answer is available")
            if str(pending.get("event_id")) != event_id or str(pending.get("choice")).casefold() != choice.casefold():
                raise ConflictError("acknowledgement does not match the pending answer")
            if not accepted:
                record["pending_answer"] = None
                record["state"] = "waiting"
                record["updated_at"] = int(time.time())
                _atomic_write_json(path, record)
                return {**record, "router_status": "rejected"}
            record["pending_answer"] = None
            record["state"] = "answered"
            record["event_id"] = event_id
            record["answer"] = choice
            record["updated_at"] = int(time.time())
            _atomic_write_json(path, record)
            return {**record, "router_status": "accepted"}

    def close(
        self,
        run_id: str,
        decision_id: str,
        *,
        event_id: str,
        choice: str,
    ) -> dict[str, object]:
        path = self.path(run_id, decision_id)
        lock_path = self.locks / f"{path.name}.lock"
        with _exclusive_lock(lock_path):
            record = _read_json(path)
            if record is None:
                raise DecisionError("orphaned decision prompt")
            state = str(record.get("state", ""))
            answer = str(record.get("answer", ""))
            if state == "closed":
                if answer.casefold() == choice.casefold() and str(record.get("event_id")) == event_id:
                    return {**record, "router_status": "duplicate"}
                raise ConflictError("close acknowledgement conflicts with the first answer")
            if state != "answered":
                raise DecisionError("decision is not answered")
            if answer.casefold() != choice.casefold() or str(record.get("event_id")) != event_id:
                raise ConflictError("close acknowledgement does not match the answer")
            record["state"] = "closed"
            record["updated_at"] = int(time.time())
            _atomic_write_json(path, record)
            return {**record, "router_status": "closed"}

    def reject_pending(
        self,
        run_id: str,
        decision_id: str,
        *,
        event_id: str,
        choice: str,
    ) -> dict[str, object]:
        """Permanently discard a bead-CAS loser without reopening the gate."""

        path = self.path(run_id, decision_id)
        lock_path = self.locks / f"{path.name}.lock"
        with _exclusive_lock(lock_path):
            record = _read_json(path)
            if record is None:
                raise DecisionError("orphaned decision prompt")
            if record.get("state") == "closed" and not record.get("answer"):
                return {**record, "router_status": "duplicate"}
            pending = record.get("pending_answer")
            if (
                record.get("state") != "answer-pending"
                or not isinstance(pending, dict)
                or str(pending.get("event_id")) != event_id
                or str(pending.get("choice")).casefold() != choice.casefold()
            ):
                raise ConflictError("rejected answer does not match the pending answer")
            record["pending_answer"] = None
            record["state"] = "closed"
            record["delivery_error"] = "gate-cas-lost"
            record["updated_at"] = int(time.time())
            _atomic_write_json(path, record)
            return {**record, "router_status": "rejected"}

    def answered_open(self) -> list[dict[str, object]]:
        records: list[dict[str, object]] = []
        for path in sorted(self.prompts.glob("*.json")):
            record = _read_json(path)
            if record is None:
                continue
            if record.get("state") in {"answered", "answer-pending"}:
                records.append(record)
        return records


class DiscordREST:
    """One-request Discord REST client; retry policy is owned by the router."""

    def __init__(
        self,
        token: str,
        *,
        api_base: str = DEFAULT_API_BASE,
        opener: Callable[..., Any] | None = None,
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        token = _string(token, "Discord bot token", max_bytes=4096)
        parsed = urllib.parse.urlparse(api_base)
        if (
            parsed.scheme != "https"
            or not parsed.netloc
            or parsed.username is not None
            or parsed.password is not None
            or parsed.query
            or parsed.fragment
        ):
            raise DecisionError("Discord API base must use HTTPS")
        self.token = token
        self.api_base = api_base.rstrip("/")
        if opener is None:
            proxy_host, proxy_port = _loopback_proxy()
            proxy_literal = f"[{proxy_host}]" if ":" in proxy_host else proxy_host
            proxy = f"http://{proxy_literal}:{proxy_port}"
            self.opener = urllib.request.build_opener(
                urllib.request.ProxyHandler({"http": proxy, "https": proxy})
            ).open
        else:
            self.opener = opener
        self.sleep = sleep

    def request_once(
        self,
        method: str,
        path: str,
        *,
        payload: Mapping[str, object] | None = None,
    ) -> tuple[int, dict[str, object] | list[object]]:
        if not path.startswith("/") or ".." in path:
            raise DecisionError("Discord API path is malformed")
        body = None
        headers = {
            "Authorization": f"Bot {self.token}",
            "User-Agent": "gascity-contributor/1",
            "Accept": "application/json",
        }
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
            with self.opener(request, timeout=15) as response:
                raw = response.read(MAX_FRAME_BYTES + 1)
                if len(raw) > MAX_FRAME_BYTES:
                    raise DecisionError("Discord response exceeds the size limit")
                if not raw:
                    return int(response.status), {}
                parsed = json.loads(raw)
                if not isinstance(parsed, (dict, list)):
                    raise DecisionError("Discord response is not a JSON object or list")
                return int(response.status), parsed
        except urllib.error.HTTPError as error:
            retry_after = _retry_after(error.headers)
            if method == "POST" and error.code >= 500:
                raise AmbiguousSend(
                    f"Discord mutating request returned HTTP {error.code}",
                    retry_after=retry_after,
                ) from error
            if error.code == 429 or error.code >= 500:
                raise RetryableDiscordError(
                    f"Discord returned HTTP {error.code}",
                    retry_after=retry_after,
                ) from error
            raise DecisionError(f"Discord returned permanent HTTP {error.code}") from error
        except (urllib.error.URLError, TimeoutError, socket.timeout, OSError) as error:
            raise AmbiguousSend("Discord request outcome is ambiguous") from error


def _retry_after(headers: Mapping[str, str] | None) -> float:
    if headers is None:
        return 0.0
    value = headers.get("Retry-After") or headers.get("retry-after")
    if value:
        try:
            return max(0.0, min(300.0, float(value)))
        except (TypeError, ValueError):
            pass
    reset = headers.get("X-RateLimit-Reset") or headers.get("x-ratelimit-reset")
    if reset:
        try:
            return max(0.0, min(300.0, float(reset) - time.time()))
        except (TypeError, ValueError):
            pass
    return 0.0


class DiscordTransport:
    """Discord REST operations used by :class:`DecisionRouter`."""

    def __init__(self, rest: DiscordREST) -> None:
        self.rest = rest

    def send_prompt(self, record: Mapping[str, object]) -> dict[str, object]:
        marker = _prompt_marker(record)
        choices = ", ".join(str(choice) for choice in record["choices"])
        suffix = f"\n\nReply to this message with one choice: {choices}\n<!-- {marker} -->"
        message = _truncate_utf8(
            str(record["message"]),
            max(1, MAX_PROMPT_BYTES - len(suffix.encode("utf-8"))),
        )
        content = f"{message}{suffix}"
        status, response = self.rest.request_once(
            "POST",
            f"/channels/{urllib.parse.quote(str(record['channel_id']), safe='')}/messages",
            payload={
                "content": content,
                "nonce": str(record["prompt_nonce"]),
            },
        )
        if status < 200 or status >= 300 or not isinstance(response, dict):
            raise DecisionError("Discord did not return a prompt message")
        message_id = response.get("id")
        if not isinstance(message_id, str) or not message_id.isdigit():
            raise DecisionError("Discord prompt response has no valid message id")
        return {"message_id": message_id, "raw": response}

    def find_prompt(self, record: Mapping[str, object]) -> dict[str, object] | None:
        channel = urllib.parse.quote(str(record["channel_id"]), safe="")
        status, response = self.rest.request_once("GET", f"/channels/{channel}/messages?limit=100")
        if status < 200 or status >= 300 or not isinstance(response, list):
            return None
        marker = _prompt_marker(record)
        for message in response:
            if not isinstance(message, dict):
                continue
            if marker in str(message.get("content", "")) and str(message.get("id", "")).isdigit():
                return {"message_id": str(message["id"]), "raw": message}
        return None

    def notify(self, record: Mapping[str, object], body: str) -> dict[str, object]:
        body = _string(body, "notification", max_bytes=MAX_BODY_BYTES)
        path = f"/channels/{urllib.parse.quote(str(record['channel_id']), safe='')}/messages"
        for attempt in range(1, MAX_DELIVERY_ATTEMPTS + 1):
            try:
                status, response = self.rest.request_once(
                    "POST",
                    path,
                    payload={"content": body},
                )
                if status < 200 or status >= 300 or not isinstance(response, dict):
                    raise DecisionError("Discord notification was not accepted")
                return response
            except RetryableDiscordError as error:
                if attempt == MAX_DELIVERY_ATTEMPTS:
                    raise DecisionError("Discord notification retry ceiling reached") from error
                self.rest.sleep(error.retry_after)
            except AmbiguousSend as error:
                raise DecisionError(
                    "Discord notification outcome is ambiguous; retry requires reconciliation"
                ) from error
        raise AssertionError("notification loop exhausted without a result")


class DecisionRouter:
    """Correlation, delivery retry, and first-answer staging."""

    def __init__(
        self,
        store: DecisionStore,
        transport: Any,
        *,
        guild_id: str,
        channel_id: str,
        operator_ids: set[str],
        sleep: Callable[[float], None] = time.sleep,
    ) -> None:
        self.store = store
        self.transport = transport
        self.guild_id = _discord_id(guild_id, "configured guild_id")
        self.channel_id = _discord_id(channel_id, "configured channel_id")
        self.operator_ids = {_discord_id(value, "configured operator id") for value in operator_ids}
        if not self.operator_ids:
            raise DecisionError("configured operator allowlist is empty")
        self.sleep = sleep

    def _deliver(self, record: dict[str, object]) -> dict[str, object]:
        completed_attempts = int(record.get("delivery_attempts", 0) or 0)
        if completed_attempts:
            try:
                reconciled = self.transport.find_prompt(record)
            except RetryableDiscordError as error:
                self.sleep(error.retry_after)
                reconciled = None
            except DecisionError:
                reconciled = None
            if reconciled is not None:
                message_id = str(reconciled.get("message_id", ""))
                if message_id.isdigit():
                    record["message_id"] = message_id
                    record["state"] = "waiting"
                    record["delivery_error"] = ""
                    return self.store.update(record)
        if completed_attempts >= MAX_DELIVERY_ATTEMPTS:
            record["state"] = "delivery-failed"
            record["delivery_error"] = "retry-ceiling"
            self.store.update(record)
            raise DecisionError("Discord prompt retry ceiling reached")
        for attempt in range(completed_attempts + 1, MAX_DELIVERY_ATTEMPTS + 1):
            record["delivery_attempts"] = attempt
            self.store.update(record)
            try:
                delivered = self.transport.send_prompt(record)
                message_id = str(delivered.get("message_id", ""))
                if not message_id.isdigit():
                    raise DecisionError("Discord prompt delivery returned an invalid message id")
                record["message_id"] = message_id
                record["state"] = "waiting"
                record["delivery_error"] = ""
                return self.store.update(record)
            except AmbiguousSend as error:
                retry_after = error.retry_after
                try:
                    reconciled = self.transport.find_prompt(record)
                except RetryableDiscordError as reconcile_error:
                    retry_after = max(retry_after, reconcile_error.retry_after)
                    reconciled = None
                except DecisionError:
                    reconciled = None
                if reconciled is not None:
                    message_id = str(reconciled.get("message_id", ""))
                    if message_id.isdigit():
                        record["message_id"] = message_id
                        record["state"] = "waiting"
                        record["delivery_error"] = ""
                        return self.store.update(record)
                if attempt == MAX_DELIVERY_ATTEMPTS:
                    record["state"] = "delivery-failed"
                    record["delivery_error"] = "ambiguous-send-unreconciled"
                    self.store.update(record)
                    raise DecisionError("Discord prompt delivery remained ambiguous") from error
                self.sleep(retry_after)
            except RetryableDiscordError as error:
                if attempt == MAX_DELIVERY_ATTEMPTS:
                    record["state"] = "delivery-failed"
                    record["delivery_error"] = "retry-ceiling"
                    self.store.update(record)
                    raise DecisionError("Discord prompt retry ceiling reached") from error
                self.sleep(error.retry_after)
            except DecisionError as error:
                record["state"] = "delivery-failed"
                record["delivery_error"] = "permanent-send-failure"
                self.store.update(record)
                raise
        raise AssertionError("delivery loop exhausted without a result")

    def request(self, value: object) -> dict[str, object]:
        request = validate_prompt_request(
            value,
            expected_guild_id=self.guild_id,
            expected_channel_id=self.channel_id,
            expected_operator_ids=self.operator_ids,
        )
        record = self.store.ensure_prompt(request)
        if record.get("state") in {"waiting", "answered", "closed"} and record.get("message_id"):
            return {**record, "router_status": "duplicate"}
        if record.get("state") == "answer-pending":
            return {**record, "router_status": "pending"}
        if record.get("state") == "delivery-failed":
            raise DecisionError(str(record.get("delivery_error") or "decision delivery failed"))
        return self._deliver(record)

    def answer(self, value: object) -> dict[str, object]:
        event = normalize_gateway_event(value)
        reply_to = str(event.get("reply_to_message_id", "")).strip()
        if not reply_to.isdigit():
            raise DecisionError("orphaned or malformed reply target")
        record = None
        for candidate in self.store.answered_open():
            if str(candidate.get("message_id")) == reply_to:
                record = candidate
                break
        if record is None:
            for path in sorted(self.store.prompts.glob("*.json")):
                candidate = _read_json(path)
                if candidate is not None and str(candidate.get("message_id")) == reply_to:
                    record = candidate
                    break
        if record is None:
            raise DecisionError("orphaned prompt reply")
        validated = validate_gateway_event(value, record, operator_ids=self.operator_ids)
        return self.store.reserve_answer(
            {
                **validated,
                "run_id": record["run_id"],
                "decision_id": record["decision_id"],
                "prompt_nonce": record["prompt_nonce"],
            }
        )

    def wait(self, run_id: str, decision_id: str, *, timeout: float = 0.0) -> dict[str, object]:
        run_id = _identifier(run_id, "run_id")
        decision_id = _identifier(decision_id, "decision_id")
        deadline = time.monotonic() + max(0.0, timeout)
        while True:
            record = self.store.get(run_id, decision_id)
            if record is None:
                raise DecisionError("decision prompt does not exist")
            if record.get("state") in {"answered", "answer-pending", "closed"}:
                pending = record.get("pending_answer")
                if isinstance(pending, dict):
                    return {
                        **record,
                        "event_id": pending.get("event_id", ""),
                        "choice": pending.get("choice", ""),
                        "router_status": "pending",
                    }
                return {
                    **record,
                    "choice": record.get("answer", ""),
                    "router_status": "closed" if record.get("state") == "closed" else "answered",
                }
            if record.get("state") == "delivery-failed":
                raise DecisionError(str(record.get("delivery_error") or "decision delivery failed"))
            if time.monotonic() >= deadline:
                return {**record, "router_status": "waiting"}
            self.sleep(IDLE_WAIT_SECONDS)

    def acknowledge(
        self,
        run_id: str,
        decision_id: str,
        *,
        event_id: str,
        choice: str,
        accepted: bool,
    ) -> dict[str, object]:
        return self.store.acknowledge(
            _identifier(run_id, "run_id"),
            _identifier(decision_id, "decision_id"),
            event_id=_event_id(event_id),
            choice=_choice(choice),
            accepted=bool(accepted),
        )

    def close(
        self,
        run_id: str,
        decision_id: str,
        *,
        event_id: str,
        choice: str,
    ) -> dict[str, object]:
        return self.store.close(
            _identifier(run_id, "run_id"),
            _identifier(decision_id, "decision_id"),
            event_id=_event_id(event_id),
            choice=_choice(choice),
        )

    def reject(
        self,
        run_id: str,
        decision_id: str,
        *,
        event_id: str,
        choice: str,
    ) -> dict[str, object]:
        return self.store.reject_pending(
            _identifier(run_id, "run_id"),
            _identifier(decision_id, "decision_id"),
            event_id=_event_id(event_id),
            choice=_choice(choice),
        )

    def reconcile(self) -> list[dict[str, object]]:
        return self.store.answered_open()

    def notify(self, run_id: str, decision_id: str, body: str) -> dict[str, object]:
        record = self.store.get(run_id, decision_id)
        if record is None:
            raise DecisionError("notification refers to an unknown decision")
        if not record.get("message_id"):
            raise DecisionError("notification cannot use a prompt without a message")
        return self.transport.notify(record, body)

    def notify_publication(self, body: str) -> dict[str, object]:
        """Send a bounded publication notice without creating decision state."""

        body = _string(body, "notification", max_bytes=MAX_BODY_BYTES)
        return self.transport.notify(
            {
                "guild_id": self.guild_id,
                "channel_id": self.channel_id,
            },
            body,
        )


def _peer_uid(connection: socket.socket) -> int:
    raw = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i"))
    _pid, uid, _gid = struct.unpack("3i", raw)
    return uid


def _open_listener(path: str, group: str) -> socket.socket:
    target = pathlib.Path(path)
    if (
        not target.is_absolute()
        or os.path.normpath(path) != path
        or any(part == ".." for part in target.parts)
        or target.is_symlink()
    ):
        raise DecisionError("decision socket path must be an absolute non-symlink")
    for ancestor in target.parents:
        if ancestor == ancestor.parent:
            break
        if ancestor.is_symlink():
            raise DecisionError("decision socket path has a symlinked ancestor")
    target = target.absolute()
    target.parent.mkdir(mode=0o770, parents=True, exist_ok=True)
    if os.path.lexists(target):
        info = os.lstat(target)
        if not stat.S_ISSOCK(info.st_mode):
            raise DecisionError("decision socket is occupied by a non-socket")
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
        raise DecisionError("decision socket group is unavailable") from error
    listener.listen(32)
    return listener


def _read_frame(connection: socket.socket) -> dict[str, object]:
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = connection.recv(min(4096, MAX_FRAME_BYTES - len(data)))
        if not chunk:
            raise DecisionError("decision channel closed")
        data.extend(chunk)
        if len(data) > MAX_FRAME_BYTES:
            raise DecisionError("decision channel frame exceeds the size limit")
    try:
        value = json.loads(bytes(data))
    except json.JSONDecodeError as error:
        raise DecisionError("decision channel frame is not JSON") from error
    if not isinstance(value, dict):
        raise DecisionError("decision channel frame must be an object")
    return value


def _write_frame(connection: socket.socket, value: Mapping[str, object]) -> None:
    connection.sendall(_json_bytes(value) + b"\n")


def _rpc(socket_path: str, request: Mapping[str, object]) -> dict[str, object]:
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        connection.settimeout(30)
        connection.connect(socket_path)
        _write_frame(connection, request)
        return _read_frame(connection)
    finally:
        connection.close()


def _read_token(path: str) -> str:
    descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | getattr(os, "O_NOFOLLOW", 0))
    try:
        token = os.read(descriptor, 4097).decode("utf-8").strip()
    except (UnicodeDecodeError, OSError) as error:
        raise DecisionError("Discord credential is unreadable") from error
    finally:
        os.close(descriptor)
    if not token or len(token.encode("utf-8")) > 4096:
        raise DecisionError("Discord credential is malformed")
    return token


class DiscordGateway:
    """Minimal Discord Gateway websocket client for MESSAGE_CREATE events."""

    def __init__(self, token: str, router: DecisionRouter, *, gateway_url: str) -> None:
        self.token = token
        self.router = router
        self.gateway_url = gateway_url
        self.stop = threading.Event()

    def run(self) -> None:
        # The REST sidecar remains available when a reconnect is needed.  The
        # gateway loop is deliberately best-effort and never exposes the token
        # to the main service.
        while not self.stop.is_set():
            try:
                self._run_once()
            except (OSError, RuntimeError, ValueError, DecisionError):
                self.stop.wait(2.0)

    def _run_once(self) -> None:
        parsed = urllib.parse.urlparse(self.gateway_url)
        if parsed.scheme != "wss" or not parsed.hostname:
            raise DecisionError("Discord gateway URL must use wss")
        port = parsed.port or 443
        proxy_host, proxy_port = _loopback_proxy()
        raw = socket.create_connection((proxy_host, proxy_port), timeout=15)
        authority = (
            f"[{parsed.hostname}]" if ":" in parsed.hostname else parsed.hostname
        )
        try:
            raw.sendall(
                (
                    f"CONNECT {authority}:{port} HTTP/1.1\r\n"
                    f"Host: {authority}:{port}\r\n"
                    "Proxy-Connection: close\r\n\r\n"
                ).encode("ascii")
            )
            header = bytearray()
            while b"\r\n\r\n" not in header:
                chunk = raw.recv(4096)
                if not chunk:
                    raise RuntimeError("Discord egress proxy closed the tunnel")
                header.extend(chunk)
                if len(header) > 16 * 1024:
                    raise RuntimeError("Discord egress proxy response is too large")
            status_line = bytes(header).split(b"\r\n", 1)[0].split()
            if len(status_line) < 2 or status_line[1] != b"200":
                raise RuntimeError("Discord egress proxy denied the gateway tunnel")
        except BaseException:
            raw.close()
            raise
        context = ssl.create_default_context()
        connection = context.wrap_socket(raw, server_hostname=parsed.hostname)
        try:
            key = base64.b64encode(os.urandom(16)).decode("ascii")
            path = parsed.path or "/"
            if parsed.query:
                path += f"?{parsed.query}"
            request = (
                f"GET {path} HTTP/1.1\r\nHost: {parsed.hostname}\r\n"
                f"Upgrade: websocket\r\nConnection: Upgrade\r\n"
                f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            ).encode("ascii")
            connection.sendall(request)
            header = bytearray()
            while b"\r\n\r\n" not in header:
                header.extend(connection.recv(4096))
                if len(header) > 16 * 1024:
                    raise RuntimeError("Discord websocket headers exceed the size limit")
            if not bytes(header).split(b"\r\n", 1)[0].startswith(b"HTTP/1.1 101"):
                raise RuntimeError("Discord gateway websocket handshake failed")
            hello = self._recv_ws(connection)
            if hello.get("op") != 10:
                raise RuntimeError("Discord gateway did not send hello")
            heartbeat_ms = int((hello.get("d") or {}).get("heartbeat_interval", 45000))
            self._send_ws(
                connection,
                {
                    "op": 2,
                    "d": {
                        "token": self.token,
                        "intents": (1 << 0) | (1 << 9) | (1 << 15),
                        "properties": {"os": "linux", "browser": "gascity", "device": "gascity"},
                    },
                },
            )
            next_heartbeat = time.monotonic() + heartbeat_ms / 1000
            while not self.stop.is_set():
                connection.settimeout(max(0.1, next_heartbeat - time.monotonic()))
                try:
                    frame = self._recv_ws(connection)
                except socket.timeout:
                    self._send_ws(connection, {"op": 1, "d": None})
                    next_heartbeat = time.monotonic() + heartbeat_ms / 1000
                    continue
                if frame.get("op") == 0:
                    raw_event = {
                        "t": frame.get("t", ""),
                        "s": frame.get("s"),
                        "d": frame.get("d") if isinstance(frame.get("d"), dict) else {},
                    }
                    if raw_event["t"] in {"MESSAGE_CREATE", "MESSAGE_UPDATE"}:
                        try:
                            self.router.answer(raw_event)
                        except DecisionError:
                            # Invalid, stale, and unauthorized events are
                            # intentionally ignored after validation.
                            pass
                elif frame.get("op") == 1:
                    self._send_ws(connection, {"op": 1, "d": frame.get("d")})
        finally:
            connection.close()

    @staticmethod
    def _send_ws(connection: socket.socket, value: Mapping[str, object]) -> None:
        payload = _json_bytes(value)
        first = 0x80 | 0x1
        length = len(payload)
        if length < 126:
            header = bytes((first, 0x80 | length))
        elif length < 65536:
            header = bytes((first, 0x80 | 126)) + struct.pack("!H", length)
        else:
            header = bytes((first, 0x80 | 127)) + struct.pack("!Q", length)
        mask = os.urandom(4)
        masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        connection.sendall(header + mask + masked)

    @staticmethod
    def _recv_ws(connection: socket.socket) -> dict[str, object]:
        header = _read_exact(connection, 2)
        first, second = header
        opcode = first & 0x0F
        if opcode == 0x8:
            raise RuntimeError("Discord gateway closed the websocket")
        length = second & 0x7F
        if length == 126:
            length = struct.unpack("!H", _read_exact(connection, 2))[0]
        elif length == 127:
            length = struct.unpack("!Q", _read_exact(connection, 8))[0]
        if length > MAX_FRAME_BYTES:
            raise RuntimeError("Discord gateway frame exceeds the size limit")
        if second & 0x80:
            mask = _read_exact(connection, 4)
        else:
            mask = b""
        payload = _read_exact(connection, length)
        if mask:
            payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        value = json.loads(payload)
        if not isinstance(value, dict):
            raise RuntimeError("Discord gateway frame is not an object")
        return value


def _read_exact(connection: socket.socket, length: int) -> bytes:
    data = bytearray()
    while len(data) < length:
        chunk = connection.recv(length - len(data))
        if not chunk:
            raise RuntimeError("Discord gateway closed mid-frame")
        data.extend(chunk)
    return bytes(data)


class DecisionServer:
    def __init__(
        self,
        *,
        socket_path: str,
        socket_group: str,
        credential_path: str,
        state_root: str,
        guild_id: str,
        channel_id: str,
        operator_ids: set[str],
        api_base: str = DEFAULT_API_BASE,
        gateway_url: str = "",
        allowed_uid: int = 45100,
    ) -> None:
        token = _read_token(credential_path)
        self.router = DecisionRouter(
            DecisionStore(state_root),
            DiscordTransport(DiscordREST(token, api_base=api_base)),
            guild_id=guild_id,
            channel_id=channel_id,
            operator_ids=operator_ids,
        )
        self.socket_path = socket_path
        self.socket_group = socket_group
        self.credential_path = credential_path
        self.allowed_uid = allowed_uid
        self.gateway_url = gateway_url

    def serve(self) -> None:
        listener = _open_listener(self.socket_path, self.socket_group)
        gateway: DiscordGateway | None = None
        if self.gateway_url:
            gateway = DiscordGateway(
                _read_token(self.credential_path),
                self.router,
                gateway_url=self.gateway_url,
            )
            threading.Thread(target=gateway.run, name="discord-gateway", daemon=True).start()
        try:
            while True:
                connection, _ = listener.accept()
                threading.Thread(
                    target=self._serve_connection,
                    args=(connection,),
                    daemon=True,
                ).start()
        finally:
            if gateway is not None:
                gateway.stop.set()
            listener.close()
            pathlib.Path(self.socket_path).unlink(missing_ok=True)

    def _serve_connection(self, connection: socket.socket) -> None:
        try:
            if _peer_uid(connection) != self.allowed_uid:
                raise PeerError("decision channel peer is unauthorized")
            request = _read_frame(connection)
            response = self._dispatch(request)
        except (DecisionError, OSError, ValueError) as error:
            response = {"protocol": PROTOCOL, "ok": False, "error": str(error)[:512]}
        try:
            _write_frame(connection, response)
        except OSError:
            pass
        finally:
            connection.close()

    def _dispatch(self, request: Mapping[str, object]) -> dict[str, object]:
        if request.get("protocol", PROTOCOL) != PROTOCOL:
            raise DecisionError("decision protocol version mismatch")
        operation = request.get("operation")
        if operation == "request":
            return {"protocol": PROTOCOL, "ok": True, "result": self.router.request(request.get("request"))}
        if operation == "answer":
            return {"protocol": PROTOCOL, "ok": True, "result": self.router.answer(request.get("event"))}
        if operation == "wait":
            timeout = float(request.get("timeout", 0.0))
            if not math.isfinite(timeout) or timeout < 0 or timeout > 3600:
                raise DecisionError("decision wait timeout is outside the allowed bound")
            return {
                "protocol": PROTOCOL,
                "ok": True,
                "result": self.router.wait(
                    str(request.get("run_id", "")),
                    str(request.get("decision_id", "")),
                    timeout=timeout,
                ),
            }
        if operation == "ack":
            accepted = request.get("accepted")
            if not isinstance(accepted, bool):
                raise DecisionError("acknowledgement accepted flag is malformed")
            return {
                "protocol": PROTOCOL,
                "ok": True,
                "result": self.router.acknowledge(
                    str(request.get("run_id", "")),
                    str(request.get("decision_id", "")),
                    event_id=str(request.get("event_id", "")),
                    choice=str(request.get("choice", "")),
                    accepted=accepted,
                ),
            }
        if operation in {"close", "reject"}:
            method = self.router.close if operation == "close" else self.router.reject
            return {
                "protocol": PROTOCOL,
                "ok": True,
                "result": method(
                    str(request.get("run_id", "")),
                    str(request.get("decision_id", "")),
                    event_id=str(request.get("event_id", "")),
                    choice=str(request.get("choice", "")),
                ),
            }
        if operation == "reconcile":
            return {"protocol": PROTOCOL, "ok": True, "result": self.router.reconcile()}
        if operation == "notify":
            return {
                "protocol": PROTOCOL,
                "ok": True,
                "result": self.router.notify(
                    str(request.get("run_id", "")),
                    str(request.get("decision_id", "")),
                    str(request.get("body", "")),
                ),
            }
        if operation == "publication-notify":
            return {
                "protocol": PROTOCOL,
                "ok": True,
                "result": self.router.notify_publication(str(request.get("body", ""))),
            }
        raise DecisionError("unknown decision channel operation")


def _result_or_raise(response: Mapping[str, object]) -> dict[str, object]:
    if not response.get("ok"):
        raise DecisionError(str(response.get("error") or "decision sidecar rejected request"))
    result = response.get("result")
    if not isinstance(result, dict) and not isinstance(result, list):
        raise DecisionError("decision sidecar returned an invalid result")
    return {"protocol": PROTOCOL, "ok": True, "result": result}


def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="operation", required=True)

    serve = subparsers.add_parser("serve")
    serve.add_argument("--socket", required=True)
    serve.add_argument("--socket-group", default="gascity-discord-channel")
    serve.add_argument("--credential", required=True)
    serve.add_argument("--state-root", required=True)
    serve.add_argument("--guild-id", required=True)
    serve.add_argument("--channel-id", required=True)
    serve.add_argument("--operator-user-id", action="append", default=[])
    serve.add_argument("--api-base", default=os.environ.get("GC_DISCORD_API_BASE", DEFAULT_API_BASE))
    serve.add_argument("--gateway-url", default=os.environ.get("GC_DISCORD_GATEWAY_URL", ""))
    serve.add_argument("--allowed-uid", type=int, default=45100)

    request = subparsers.add_parser("request")
    request.add_argument("--socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))
    request.add_argument("--run-id", required=True)
    request.add_argument("--bead-id", required=True)
    request.add_argument("--decision-id", required=True)
    request.add_argument("--prompt-nonce")
    request.add_argument("--assignee", default=os.environ.get("GC_DECISION_ASSIGNEE", ""))
    request.add_argument("--guild-id", required=True)
    request.add_argument("--channel-id", required=True)
    request.add_argument("--message", required=True)
    request.add_argument("--choice", action="append", default=[])
    request.add_argument("--choices-json")

    wait = subparsers.add_parser("wait")
    wait.add_argument("--socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))
    wait.add_argument("--run-id", required=True)
    wait.add_argument("--decision-id", required=True)
    wait.add_argument("--timeout", type=float, default=300.0)

    ack = subparsers.add_parser("ack")
    ack.add_argument("--socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))
    ack.add_argument("--run-id", required=True)
    ack.add_argument("--decision-id", required=True)
    ack.add_argument("--event-id", required=True)
    ack.add_argument("--choice", required=True)
    ack.add_argument("--accepted", action="store_true")

    for operation in ("close", "reject"):
        gate = subparsers.add_parser(operation)
        gate.add_argument("--socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))
        gate.add_argument("--run-id", required=True)
        gate.add_argument("--decision-id", required=True)
        gate.add_argument("--event-id", required=True)
        gate.add_argument("--choice", required=True)

    reconcile = subparsers.add_parser("reconcile")
    reconcile.add_argument("--socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))

    notify = subparsers.add_parser("notify")
    notify.add_argument("--socket", default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""))
    notify.add_argument("--run-id", required=True)
    notify.add_argument("--decision-id", required=True)
    notify.add_argument("--body", required=True)

    publication_notify = subparsers.add_parser("publication-notify")
    publication_notify.add_argument(
        "--socket",
        default=os.environ.get("GC_DISCORD_CHANNEL_SOCKET", ""),
    )
    publication_notify.add_argument("--body", required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    if args.operation == "serve":
        if not args.operator_user_id:
            raise DecisionError("at least one Discord operator is required")
        DecisionServer(
            socket_path=args.socket,
            socket_group=args.socket_group,
            credential_path=args.credential,
            state_root=args.state_root,
            guild_id=args.guild_id,
            channel_id=args.channel_id,
            operator_ids=set(args.operator_user_id),
            api_base=args.api_base,
            gateway_url=args.gateway_url,
            allowed_uid=args.allowed_uid,
        ).serve()
        return 0
    if not args.socket:
        raise DecisionError("decision socket is required")
    if args.operation == "request":
        choices = list(args.choice)
        if args.choices_json is not None:
            try:
                parsed_choices = json.loads(args.choices_json)
            except json.JSONDecodeError as error:
                raise DecisionError("choices JSON is malformed") from error
            if not isinstance(parsed_choices, list):
                raise DecisionError("choices JSON must be a list")
            choices = parsed_choices
        if not choices:
            raise DecisionError("at least one choice is required")
        request = {
            "protocol": PROTOCOL,
            "run_id": args.run_id,
            "bead_id": args.bead_id,
            "decision_id": args.decision_id,
            "prompt_nonce": args.prompt_nonce,
            "assignee": args.assignee,
            "guild_id": args.guild_id,
            "channel_id": args.channel_id,
            "message": args.message,
            "choices": choices,
        }
        request = {key: value for key, value in request.items() if value is not None}
        response = _rpc(args.socket, {"protocol": PROTOCOL, "operation": "request", "request": request})
    elif args.operation == "wait":
        response = _rpc(
            args.socket,
            {
                "protocol": PROTOCOL,
                "operation": "wait",
                "run_id": args.run_id,
                "decision_id": args.decision_id,
                "timeout": args.timeout,
            },
        )
    elif args.operation == "ack":
        response = _rpc(
            args.socket,
            {
                "protocol": PROTOCOL,
                "operation": "ack",
                "run_id": args.run_id,
                "decision_id": args.decision_id,
                "event_id": args.event_id,
                "choice": args.choice,
                "accepted": args.accepted,
            },
        )
    elif args.operation in {"close", "reject"}:
        response = _rpc(
            args.socket,
            {
                "protocol": PROTOCOL,
                "operation": args.operation,
                "run_id": args.run_id,
                "decision_id": args.decision_id,
                "event_id": args.event_id,
                "choice": args.choice,
            },
        )
    elif args.operation == "reconcile":
        response = _rpc(args.socket, {"protocol": PROTOCOL, "operation": "reconcile"})
    elif args.operation == "notify":
        response = _rpc(
            args.socket,
            {
                "protocol": PROTOCOL,
                "operation": "notify",
                "run_id": args.run_id,
                "decision_id": args.decision_id,
                "body": args.body,
            },
        )
    else:
        response = _rpc(
            args.socket,
            {
                "protocol": PROTOCOL,
                "operation": "publication-notify",
                "body": args.body,
            },
        )
    result = _result_or_raise(response)
    print(json.dumps(result["result"], ensure_ascii=False, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (DecisionError, OSError) as error:
        print(f"discord decision rejected: {error}", file=sys.stderr)
        raise SystemExit(2)
