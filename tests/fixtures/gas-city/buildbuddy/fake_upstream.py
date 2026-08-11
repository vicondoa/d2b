#!/usr/bin/env python3
"""Hermetic BuildBuddy cache and remote-execution HTTP fixture."""

from __future__ import annotations

import argparse
import http.server
import json
import threading
from collections.abc import Mapping


class FakeBuildBuddyState:
    def __init__(self, api_key: str):
        self.api_key = api_key
        self.cache: dict[str, bytes] = {}
        self.executions: list[dict[str, object]] = []
        self.lock = threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    server: "FakeBuildBuddyServer"

    def _authorized(self) -> bool:
        return self.headers.get("x-buildbuddy-api-key") == self.server.state.api_key

    def _json(self, status: int, value: Mapping[str, object]) -> None:
        encoded = json.dumps(dict(value), sort_keys=True).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def do_PUT(self) -> None:  # noqa: N802 - stdlib handler API
        if not self._authorized() or not self.path.startswith("/cache/"):
            self._json(403, {"ok": False})
            return
        key = self.path.removeprefix("/cache/")
        length = int(self.headers.get("content-length", "0"))
        with self.server.state.lock:
            self.server.state.cache[key] = self.rfile.read(length)
        self._json(200, {"ok": True, "operation": "cache-upload"})

    def do_GET(self) -> None:  # noqa: N802 - stdlib handler API
        if not self._authorized() or not self.path.startswith("/cache/"):
            self._json(403, {"ok": False})
            return
        key = self.path.removeprefix("/cache/")
        with self.server.state.lock:
            value = self.server.state.cache.get(key)
        if value is None:
            self._json(404, {"ok": False})
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(value)))
        self.end_headers()
        self.wfile.write(value)

    def do_POST(self) -> None:  # noqa: N802 - stdlib handler API
        if not self._authorized() or self.path != "/execute":
            self._json(403, {"ok": False})
            return
        length = int(self.headers.get("content-length", "0"))
        payload = json.loads(self.rfile.read(length))
        with self.server.state.lock:
            self.server.state.executions.append(payload)
        self._json(200, {"ok": True, "operation": "remote-execution", "result": "success"})

    def log_message(self, *_args: object) -> None:
        return


class FakeBuildBuddyServer(http.server.ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int], api_key: str):
        self.state = FakeBuildBuddyState(api_key)
        super().__init__(address, Handler)


def serve(api_key: str, host: str = "127.0.0.1", port: int = 0) -> FakeBuildBuddyServer:
    server = FakeBuildBuddyServer((host, port), api_key)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-key", required=True)
    parser.add_argument("--listen", default="127.0.0.1:0")
    args = parser.parse_args()
    host, port_text = args.listen.rsplit(":", 1)
    server = serve(args.api_key, host, int(port_text))
    print(json.dumps({"host": host, "port": server.server_port}, sort_keys=True), flush=True)
    try:
        threading.Event().wait()
    except KeyboardInterrupt:
        server.shutdown()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
