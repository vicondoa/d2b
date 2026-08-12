#!/usr/bin/env python3
"""Small NDJSON ACP server used by the Gas City profile fixtures."""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import sys
import time


SANDBOX_WORKSPACE = "/workspace"


def _settings() -> dict[str, str]:
    home = os.environ.get("COPILOT_HOME")
    if not home:
        raise RuntimeError("COPILOT_HOME is not set")
    path = pathlib.Path(home) / "settings.json"
    with path.open("r", encoding="utf-8") as stream:
        value = json.load(stream)
    if not isinstance(value, dict) or set(value) != {"model", "contextTier"}:
        raise RuntimeError("Copilot settings authority is malformed")
    if not all(isinstance(value[key], str) and value[key] for key in value):
        raise RuntimeError("Copilot settings authority has malformed values")
    return {"model": str(value["model"]), "contextTier": str(value["contextTier"])}


def _write(value: dict[str, object]) -> None:
    encoded = json.dumps(value, separators=(",", ":"))
    if "\n" in encoded or "\r" in encoded:
        raise RuntimeError("fake ACP response is not NDJSON-safe")
    sys.stdout.write(encoded + "\n")
    sys.stdout.flush()


def _models(current_model: str, *, include_current: bool = True) -> dict[str, object]:
    models: dict[str, object] = {
        "currentModelId": current_model,
        "availableModels": [
            {"modelId": "gpt-5.6-sol", "name": "GPT-5.6 Sol"},
            {"modelId": "gpt-5.6-luna", "name": "GPT-5.6 Luna"},
            {"modelId": "gpt-4.1", "name": "GPT-4.1"},
        ],
    }
    if not include_current:
        models.pop("currentModelId")
    return models


def _error(request: dict[str, object], message: str) -> None:
    _write(
        {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "error": {"code": -32600, "message": message},
        }
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--acp", action="store_true")
    parser.add_argument("--effort", required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--context", required=True)
    parser.add_argument("--silent-metadata", action="store_true")
    parser.add_argument("--fixture-direct-cwd")
    parser.add_argument("--ignore-eof", action="store_true")
    parser.add_argument("--close-after-initialize", action="store_true")
    args, _unknown = parser.parse_known_args()
    if not args.acp:
        raise SystemExit("--acp is required")
    settings = _settings()
    expected_effort = "xhigh" if settings["model"] == "gpt-5.6-sol" else "max"
    if args.effort != expected_effort:
        raise RuntimeError("ACP effort does not match the immutable profile")
    if args.model != settings["model"]:
        raise RuntimeError("ACP model does not match the immutable profile")
    if args.context != settings["contextTier"]:
        raise RuntimeError("ACP context does not match the immutable profile")
    session_id = "fake-session"
    expected_cwd = args.fixture_direct_cwd or SANDBOX_WORKSPACE
    models = _models(
        settings["model"],
        include_current=not args.silent_metadata,
    )
    for raw_line in sys.stdin:
        if not raw_line.strip():
            _error({}, "empty NDJSON line")
            continue
        try:
            request = json.loads(raw_line)
        except json.JSONDecodeError:
            _write(
                {
                    "jsonrpc": "2.0",
                    "id": None,
                    "error": {"code": -32700, "message": "invalid JSON"},
                }
            )
            continue
        if not isinstance(request, dict):
            _error({}, "request is not an object")
            continue
        method = request.get("method")
        if method == "initialize":
            _write(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": {
                        "protocolVersion": 1,
                        "agentInfo": {
                            "name": "fake-copilot",
                            "version": "1.0.79",
                        },
                        "models": models,
                    },
                }
            )
            if args.close_after_initialize:
                return 0
        elif method == "session/new":
            params = request.get("params")
            if not isinstance(params, dict) or params.get("cwd") != expected_cwd:
                _error(request, f"session/new cwd must be {expected_cwd}")
                continue
            _write(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": {
                        "sessionId": session_id,
                        "models": models,
                    },
                }
            )
        elif method == "session/prompt":
            params = request.get("params")
            if not isinstance(params, dict) or params.get("sessionId") != session_id:
                _error(request, "session id is invalid")
                continue
            _write(
                {
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": {
                                "type": "text",
                                "text": f"effective model: {settings['model']}",
                            },
                            "models": models,
                        },
                    },
                }
            )
            _write(
                {
                    "jsonrpc": "2.0",
                    "id": request.get("id"),
                    "result": {
                        "stopReason": "end_turn",
                        "models": models,
                    },
                }
            )
        else:
            _error(request, "unsupported ACP method")
    if args.ignore_eof:
        while True:
            time.sleep(0.05)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
