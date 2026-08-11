from __future__ import annotations

import importlib.util
import json
import pathlib
import unittest
import urllib.request


ROOT = pathlib.Path(__file__).resolve().parent
REPOSITORY_ROOT = ROOT.parents[3]
SPEC = importlib.util.spec_from_file_location("gascity_fake_buildbuddy", ROOT / "fake_upstream.py")
assert SPEC is not None and SPEC.loader is not None
FAKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FAKE)
PROXY_SPEC = importlib.util.spec_from_file_location(
    "gascity_buildbuddy_proxy",
    REPOSITORY_ROOT / "nix/gas-city-contributor/pack/scripts/buildbuddy-proxy.py",
)
assert PROXY_SPEC is not None and PROXY_SPEC.loader is not None
PROXY = importlib.util.module_from_spec(PROXY_SPEC)
PROXY_SPEC.loader.exec_module(PROXY)


class BuildBuddyRoundTripTests(unittest.TestCase):
    def setUp(self) -> None:
        self.server = FAKE.serve("fixture-key")

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def request(self, method: str, path: str, body: bytes = b"") -> bytes:
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.server.server_port}{path}",
            data=body,
            method=method,
            headers={"x-buildbuddy-api-key": "fixture-key"},
        )
        with urllib.request.urlopen(request, timeout=2) as response:
            return response.read()

    def test_cache_upload_and_download_round_trip(self) -> None:
        payload = b"bazel-8.7.0-cache-payload"
        self.request("PUT", "/cache/action-1", payload)
        self.assertEqual(self.request("GET", "/cache/action-1"), payload)

    def test_remote_execution_round_trip_is_authenticated(self) -> None:
        result = json.loads(self.request("POST", "/execute", b'{"command":"bazel"}'))
        self.assertEqual(result["operation"], "remote-execution")
        self.assertEqual(len(self.server.state.executions), 1)

    def test_key_is_required(self) -> None:
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.server.server_port}/cache/missing",
            method="GET",
        )
        with self.assertRaises(Exception):
            urllib.request.urlopen(request, timeout=2)

    def test_envoy_config_injects_key_only_at_runtime(self) -> None:
        config = PROXY.render_config(
            str(REPOSITORY_ROOT / "nix/gas-city-contributor/buildbuddy/envoy.yaml.tmpl"),
            "fixture-key",
        )
        self.assertIn("x-buildbuddy-api-key", config)
        self.assertIn("fixture-key", config)
        self.assertIn("remote.buildbuddy.io", config)
        self.assertIn("http2_protocol_options", config)
        with self.assertRaises(PROXY.BuildBuddyProxyError):
            PROXY.validate_upstream("attacker.example:443")


if __name__ == "__main__":
    unittest.main()
