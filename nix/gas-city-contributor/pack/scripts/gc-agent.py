#!/usr/bin/env python3
"""Current-run progress helper for an ACP worker.

This is deliberately not a general Gas City client.  The only wire
operations are progress observations and heartbeats for the run identity
projected by the launcher.  Decision, assignment, publication, merge, and
control operations are rejected before they reach the service channel.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import sys
from collections.abc import Mapping


PROTOCOL = "gc-agent/1"
ALLOWED_OPERATIONS = frozenset({"status", "progress", "heartbeat", "checkpoint"})
FORBIDDEN_OPERATION_WORDS = frozenset(
    {
        "assign",
        "cancel",
        "close",
        "decision",
        "merge",
        "operation",
        "publish",
        "retry",
        "route",
    }
)
MAX_MESSAGE_BYTES = 8192


class AgentChannelError(RuntimeError):
    """Raised when the current-run channel rejects a request."""


def _required_identity(environment: Mapping[str, str]) -> dict[str, str]:
    keys = ("GC_RUN_ID", "GC_BEAD_ID", "GC_CITY_GENERATION", "GC_STATE_SCHEMA")
    missing = [key for key in keys if not environment.get(key)]
    if missing:
        raise AgentChannelError(f"current-run identity is incomplete: {', '.join(missing)}")
    return {
        "run_id": environment["GC_RUN_ID"],
        "bead_id": environment["GC_BEAD_ID"],
        "generation": environment["GC_CITY_GENERATION"],
        "state_schema": environment["GC_STATE_SCHEMA"],
    }


def _reject_decision_fields(value: object) -> None:
    if isinstance(value, dict):
        for key, nested in value.items():
            if not isinstance(key, str) or any(
                key.lower() == word or key.lower().startswith(f"{word}_")
                for word in FORBIDDEN_OPERATION_WORDS
            ):
                raise AgentChannelError("progress payload contains a decision field")
            _reject_decision_fields(nested)
    elif isinstance(value, list):
        for nested in value:
            _reject_decision_fields(nested)


def _json_safe_payload(payload: object) -> dict[str, object]:
    if payload is None:
        return {}
    if not isinstance(payload, dict):
        raise AgentChannelError("progress payload must be an object")
    encoded = json.dumps(payload, ensure_ascii=False, separators=(",", ":"))
    if len(encoded.encode("utf-8")) > MAX_MESSAGE_BYTES:
        raise AgentChannelError("progress payload exceeds the size limit")
    _reject_decision_fields(payload)
    return dict(payload)


def validate_request(
    request: object,
    *,
    environment: Mapping[str, str],
) -> dict[str, object]:
    if not isinstance(request, dict):
        raise AgentChannelError("request must be an object")
    if request.get("protocol") != PROTOCOL:
        raise AgentChannelError("progress protocol version mismatch")
    operation = request.get("operation")
    if not isinstance(operation, str) or operation not in ALLOWED_OPERATIONS:
        raise AgentChannelError("operation is not an allowed progress operation")
    identity = _required_identity(environment)
    request_identity = request.get("identity")
    if request_identity is not None and request_identity != identity:
        raise AgentChannelError("request identity does not match the current run")
    payload = _json_safe_payload(request.get("payload"))
    return {
        "protocol": PROTOCOL,
        "operation": operation,
        "identity": identity,
        "payload": payload,
    }


def make_request(
    operation: str,
    *,
    environment: Mapping[str, str] | None = None,
    payload: Mapping[str, object] | None = None,
) -> dict[str, object]:
    if operation not in ALLOWED_OPERATIONS:
        raise AgentChannelError("operation is not an allowed progress operation")
    identity = _required_identity(environment or os.environ)
    return validate_request(
        {
            "protocol": PROTOCOL,
            "operation": operation,
            "identity": identity,
            "payload": dict(payload or {}),
        },
        environment=environment or os.environ,
    )


def _receive_line(channel: socket.socket) -> dict[str, object]:
    data = bytearray()
    while not data.endswith(b"\n"):
        chunk = channel.recv(4096)
        if not chunk:
            raise AgentChannelError("current-run channel closed")
        data.extend(chunk)
        if len(data) > MAX_MESSAGE_BYTES:
            raise AgentChannelError("current-run response exceeds the size limit")
    try:
        value = json.loads(bytes(data))
    except json.JSONDecodeError as error:
        raise AgentChannelError("current-run response is not JSON") from error
    if not isinstance(value, dict):
        raise AgentChannelError("current-run response is not an object")
    return value


def exchange(
    channel_fd: int,
    request: Mapping[str, object],
) -> dict[str, object]:
    if channel_fd < 3:
        raise AgentChannelError("current-run fd must not overlap stdio")
    channel = socket.socket(fileno=os.dup(channel_fd))
    try:
        encoded = json.dumps(request, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        if len(encoded) > MAX_MESSAGE_BYTES:
            raise AgentChannelError("current-run request exceeds the size limit")
        channel.sendall(encoded + b"\n")
        response = _receive_line(channel)
    finally:
        channel.close()
    return response


def serve(
    channel_fd: int,
    *,
    environment: Mapping[str, str],
    handler,
) -> None:
    if channel_fd < 3:
        raise AgentChannelError("current-run fd must not overlap stdio")
    channel = socket.socket(fileno=os.dup(channel_fd))
    try:
        while True:
            request = _receive_line(channel)
            validated = validate_request(request, environment=environment)
            response = handler(validated)
            if not isinstance(response, dict):
                raise AgentChannelError("current-run handler returned a non-object")
            encoded = json.dumps(response, ensure_ascii=False, separators=(",", ":")).encode(
                "utf-8"
            )
            if len(encoded) > MAX_MESSAGE_BYTES:
                raise AgentChannelError("current-run handler response exceeds the size limit")
            channel.sendall(encoded + b"\n")
    finally:
        channel.close()


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fd", type=int, default=None)
    parser.add_argument(
        "operation",
        choices=sorted(ALLOWED_OPERATIONS),
        help="non-decision progress operation",
    )
    parser.add_argument("--message")
    parser.add_argument("--next-action")
    parser.add_argument("--summary")
    parser.add_argument("--server", action="store_true")
    return parser.parse_args()


def _fd_from_args(args: argparse.Namespace) -> int:
    if args.fd is not None:
        return args.fd
    raw = os.environ.get("GC_AGENT_FD")
    if not raw:
        raise AgentChannelError("GC_AGENT_FD or --fd is required")
    try:
        return int(raw, 10)
    except ValueError as error:
        raise AgentChannelError("GC_AGENT_FD is not an integer") from error


def _payload(args: argparse.Namespace) -> dict[str, object]:
    payload: dict[str, object] = {}
    if args.message is not None:
        payload["message"] = args.message
    if args.next_action is not None:
        payload["next_action"] = args.next_action
    if args.summary is not None:
        payload["summary"] = args.summary
    return payload


def main() -> int:
    args = _parse_args()
    fd = _fd_from_args(args)
    request = make_request(args.operation, payload=_payload(args))
    if args.server:
        def handler(validated: Mapping[str, object]) -> dict[str, object]:
            return {
                "protocol": PROTOCOL,
                "ok": True,
                "operation": validated["operation"],
                "identity": validated["identity"],
            }

        serve(fd, environment=os.environ, handler=handler)
        return 0
    response = exchange(fd, request)
    json.dump(response, sys.stdout, ensure_ascii=False, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
