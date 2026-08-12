"""Hermetic contract coverage for the U3 ACP and inherited-channel boundaries."""

from __future__ import annotations

import array
import fcntl
import importlib.util
import io
import json
import os
import pathlib
import signal
import shutil
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import unittest
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[4]
SCRIPT_ROOT = ROOT / "nix/gas-city-contributor/pack/scripts"
COPILOT_ROOT = ROOT / "nix/gas-city-contributor/copilot"
FAKE_ACP = pathlib.Path(__file__).with_name("fake_acp.py")
CITY = ROOT / "nix/gas-city-contributor/city/city.toml"
ROLE_MATRIX = ROOT / "nix/gas-city-contributor/city/agent-role-matrix.toml"
MANAGED_ASSET_SCRATCH_ROOT_ENV = "GC_MANAGED_ASSET_SCRATCH_ROOT"


def load_script(name: str):
    path = SCRIPT_ROOT / name
    module_name = f"gascity_fixture_{name.replace('-', '_').replace('.', '_')}"
    specification = importlib.util.spec_from_file_location(module_name, path)
    if specification is None or specification.loader is None:
        raise AssertionError(f"cannot load fixture script: {path}")
    module = importlib.util.module_from_spec(specification)
    sys.modules[module_name] = module
    specification.loader.exec_module(module)
    return module


PROFILE = load_script("copilot-profile.py")
LAUNCHER = load_script("agent-launcher.py")
SANDBOX = load_script("agent-sandbox.py")
ACTIVATION = load_script("service-activation.py")
OPERATOR = load_script("operator.py")
FDPROXY = load_script("fdproxy.py")
GC_AGENT = load_script("gc-agent.py")
CHECK_RUNNER = load_script("check-runner.py")


def successful_probe(profile: str) -> dict[str, object]:
    expected = ACTIVATION.PROFILE_SETTINGS[profile]
    return {
        "profile": profile,
        "ok": True,
        "model": expected["model"],
        "context": expected["context"],
        "effort": expected["effort"],
    }


def complete_probe_exchange(
    models: dict[str, object] | None = None,
) -> list[dict[str, object]]:
    initialize_result: dict[str, object] = {
        "protocolVersion": 1,
        "agentCapabilities": {},
        "agentInfo": {"name": "fake-copilot", "version": "1.0.79"},
        "authMethods": [],
    }
    session_result: dict[str, object] = {"sessionId": "fake-session"}
    prompt_result: dict[str, object] = {"stopReason": "end_turn", "usage": {}}
    if models is not None:
        initialize_result["models"] = models
        session_result["models"] = models
        prompt_result["models"] = models
    return [
        {"jsonrpc": "2.0", "id": 1, "result": initialize_result},
        {"jsonrpc": "2.0", "id": 2, "result": session_result},
        {"jsonrpc": "2.0", "id": 3, "result": prompt_result},
    ]


class ProfileContractTests(unittest.TestCase):
    def test_settings_have_only_persistent_authority(self) -> None:
        for profile in ("review-sol", "review-luna", "code-luna"):
            path = COPILOT_ROOT / profile / "settings.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(set(value), {"model", "contextTier"})
            self.assertEqual(PROFILE.load_profile(profile), value)

    def test_model_context_and_effort_are_acp_startup_arguments(self) -> None:
        for profile, expected in PROFILE.PROFILE_SETTINGS.items():
            argv = PROFILE.child_argv(
                profile,
                tool_policy="coding" if profile == "code-luna" else "review",
            )
            self.assertEqual(argv[argv.index("--model") + 1], expected["model"])
            self.assertEqual(
                argv[argv.index("--context") + 1],
                expected["contextTier"],
            )
            self.assertEqual(
                argv[argv.index("--effort") + 1],
                PROFILE.PROFILE_EFFORT[profile],
            )
            self.assertEqual(argv.count("--model"), 1)
            self.assertEqual(argv.count("--context"), 1)
            self.assertEqual(argv.count("--effort"), 1)

    def test_profile_owned_startup_arguments_reject_overrides(self) -> None:
        with self.assertRaises(LAUNCHER.LauncherError):
            LAUNCHER.validate_child_arguments(
                ["--acp", "--model", "gpt-4"],
                profile="code-luna",
            )
        with self.assertRaises(LAUNCHER.LauncherError):
            LAUNCHER.validate_child_arguments(
                [
                    "--acp",
                    "--model",
                    "gpt-5.6-luna",
                    "--context",
                    "long_context",
                ],
                profile="code-luna",
            )
        with self.assertRaises(LAUNCHER.LauncherError):
            LAUNCHER.validate_child_arguments(
                ["--acp", "--effort", "xhigh"],
                profile="code-luna",
            )
        self.assertEqual(
            LAUNCHER.validate_child_arguments(["--acp"], profile="code-luna")[-6:],
            [
                "--model",
                "gpt-5.6-luna",
                "--context",
                "default",
                "--effort",
                "max",
            ],
        )

    def test_probe_accepts_valid_silent_full_exchange(self) -> None:
        result = PROFILE._probe_result(
            "code-luna",
            complete_probe_exchange(),
        )
        self.assertEqual(
            result,
            {
                "ok": True,
                "profile": "code-luna",
                "model": "gpt-5.6-luna",
                "context": "default",
                "effort": PROFILE.PROFILE_EFFORT["code-luna"],
            },
        )

    def test_probe_accepts_actual_copilot_response_shapes(self) -> None:
        exchange = complete_probe_exchange()
        self.assertEqual(exchange[0]["result"]["protocolVersion"], 1)
        self.assertEqual(exchange[1]["result"]["sessionId"], "fake-session")
        self.assertEqual(exchange[2]["result"]["stopReason"], "end_turn")
        self.assertEqual(
            set(PROFILE._validated_probe_responses(exchange)),
            {1, 2, 3},
        )

    def test_probe_accepts_expected_active_model_in_full_exchange(self) -> None:
        expected = PROFILE.PROFILE_SETTINGS["code-luna"]
        result = PROFILE._probe_result(
            "code-luna",
            complete_probe_exchange({"currentModelId": expected["model"]}),
        )
        self.assertEqual(result["ok"], True)
        self.assertEqual(result["model"], expected["model"])

    def test_probe_rejects_empty_observations(self) -> None:
        result = PROFILE._probe_result("code-luna", [])
        self.assertFalse(result["ok"])
        self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_missing_or_wrong_jsonrpc(self) -> None:
        for position in range(3):
            for version in (None, "1.0"):
                with self.subTest(position=position, version=version):
                    exchange = complete_probe_exchange()
                    if version is None:
                        exchange[position].pop("jsonrpc")
                    else:
                        exchange[position]["jsonrpc"] = version
                    result = PROFILE._probe_result("code-luna", exchange)
                    self.assertFalse(result["ok"])
                    self.assertEqual(result["error_code"], "malformed")

        for notification in (
            {"method": "session/update"},
            {"jsonrpc": "1.0", "method": "session/update"},
        ):
            with self.subTest(notification=notification):
                exchange = complete_probe_exchange()
                exchange.insert(1, notification)
                result = PROFILE._probe_result("code-luna", exchange)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_method_result_hybrids(self) -> None:
        exchange = complete_probe_exchange()
        exchange[0]["method"] = "server/request"
        result = PROFILE._probe_result("code-luna", exchange)
        self.assertFalse(result["ok"])
        self.assertEqual(result["error_code"], "malformed")

    def test_response_for_rejects_server_requests(self) -> None:
        class Reader:
            def read(self, _timeout: float) -> dict[str, object]:
                return {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "server/request",
                    "params": {},
                }

        with self.assertRaisesRegex(PROFILE.ProfileError, "server requests"):
            PROFILE._response_for(
                Reader(),
                1,
                1,
                observations=[],
                phase="initialize",
            )

    def test_socket_probe_classifies_server_requests_as_malformed(self) -> None:
        client, server = socket.socketpair()
        server.sendall(
            PROFILE._frame(
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "server/request",
                    "params": {},
                }
            )
        )
        server.shutdown(socket.SHUT_WR)
        args = PROFILE._default_namespace("code-luna", "coding")
        try:
            with mock.patch.object(
                PROFILE,
                "_connect_launcher",
                return_value=client,
            ):
                result = PROFILE._run_socket_probe(
                    "code-luna",
                    tool_policy="coding",
                    args=args,
                    timeout=1.0,
                )
        finally:
            server.close()

        self.assertFalse(result["ok"])
        self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_malformed_idless_messages(self) -> None:
        malformed = (
            {"jsonrpc": "2.0"},
            {"jsonrpc": "2.0", "params": {}},
            {"jsonrpc": "2.0", "method": ""},
            {"jsonrpc": "2.0", "method": "session/update", "params": []},
            {
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {},
                "result": {},
            },
            {
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {},
                "error": {},
            },
        )
        for value in malformed:
            with self.subTest(value=value):
                exchange = complete_probe_exchange()
                exchange.insert(1, value)
                result = PROFILE._probe_result("code-luna", exchange)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_accepts_notifications_between_responses(self) -> None:
        expected_model = PROFILE.PROFILE_SETTINGS["code-luna"]["model"]
        notification = {
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "fake-session",
                "update": {"models": {"currentModelId": expected_model}},
            },
        }
        exchange = complete_probe_exchange()
        exchange.insert(1, notification)
        exchange.insert(3, {"jsonrpc": "2.0", "method": "progress"})
        result = PROFILE._probe_result("code-luna", exchange)
        self.assertTrue(result["ok"])
        self.assertEqual(
            set(PROFILE._validated_probe_responses(exchange)),
            {1, 2, 3},
        )

    def test_probe_rejects_unexpected_or_out_of_order_response_ids(self) -> None:
        cases: list[list[dict[str, object]]] = []
        swapped = complete_probe_exchange()
        swapped[0], swapped[1] = swapped[1], swapped[0]
        cases.append(swapped)
        unexpected = complete_probe_exchange()
        unexpected[1]["id"] = 9
        cases.append(unexpected)
        duplicate = complete_probe_exchange()
        duplicate[1]["id"] = 1
        cases.append(duplicate)
        for exchange in cases:
            with self.subTest(exchange=exchange):
                result = PROFILE._probe_result("code-luna", exchange)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_initialize_protocol_version_mismatch_or_missing(self) -> None:
        cases = []
        missing = complete_probe_exchange()
        missing[0]["result"].pop("protocolVersion")
        cases.append(missing)
        mismatch = complete_probe_exchange()
        mismatch[0]["result"]["protocolVersion"] = 2
        cases.append(mismatch)
        non_integer = complete_probe_exchange()
        non_integer[0]["result"]["protocolVersion"] = True
        cases.append(non_integer)
        for exchange in cases:
            with self.subTest(exchange=exchange):
                result = PROFILE._probe_result("code-luna", exchange)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_nested_or_snake_case_session_ids(self) -> None:
        for session_result in (
            {"session": {"sessionId": "fake-session"}},
            {"session_id": "fake-session"},
            {"data": {"sessionId": "fake-session"}},
        ):
            with self.subTest(session_result=session_result):
                exchange = complete_probe_exchange()
                exchange[1]["result"] = session_result
                result = PROFILE._probe_result("code-luna", exchange)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_missing_unknown_or_non_string_stop_reason(self) -> None:
        cases = []
        missing = complete_probe_exchange()
        missing[2]["result"].pop("stopReason")
        cases.append(missing)
        unknown = complete_probe_exchange()
        unknown[2]["result"]["stopReason"] = "still_working"
        cases.append(unknown)
        non_string = complete_probe_exchange()
        non_string[2]["result"]["stopReason"] = 1
        cases.append(non_string)
        for exchange in cases:
            with self.subTest(exchange=exchange):
                result = PROFILE._probe_result("code-luna", exchange)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_missing_response_id_or_result(self) -> None:
        complete = complete_probe_exchange()
        missing_id = [complete[0], dict(complete[1]), complete[2]]
        missing_id[1].pop("id")
        missing_result = [complete[0], {"jsonrpc": "2.0", "id": 2}, complete[2]]
        for observations in (missing_id, missing_result):
            with self.subTest(observations=observations):
                result = PROFILE._probe_result("code-luna", observations)
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_id_only_responses(self) -> None:
        result = PROFILE._probe_result(
            "code-luna",
            [{"jsonrpc": "2.0", "id": request_id} for request_id in (1, 2, 3)],
        )
        self.assertFalse(result["ok"])
        self.assertEqual(result["error_code"], "malformed")

    def test_probe_accepts_catalog_only_full_exchange(self) -> None:
        expected = PROFILE.PROFILE_SETTINGS["code-luna"]
        result = PROFILE._probe_result(
            "code-luna",
            complete_probe_exchange(
                {
                    "availableModels": [
                        {"modelId": "gpt-5.6-sol", "name": "GPT-5.6 Sol"},
                        {"model_id": "gpt-4.1", "name": "GPT-4.1"},
                        {
                            "modelId": expected["model"],
                            "name": "GPT-5.6 Luna",
                        },
                    ]
                },
            ),
        )
        self.assertEqual(
            result,
            {
                "ok": True,
                "profile": "code-luna",
                "model": expected["model"],
                "context": expected["contextTier"],
                "effort": PROFILE.PROFILE_EFFORT["code-luna"],
            },
        )

    def test_probe_rejects_null_empty_or_non_string_active_model_values(self) -> None:
        cases = (
            ("currentModelId", None),
            ("current_model_id", ""),
            ("effectiveModel", 42),
            ("effective_model", False),
        )
        for key, value in cases:
            with self.subTest(key=key, value=value):
                result = PROFILE._probe_result(
                    "code-luna",
                    complete_probe_exchange({key: value}),
                )
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_probe_rejects_mismatched_or_conflicting_active_model(self) -> None:
        expected = PROFILE.PROFILE_SETTINGS["code-luna"]
        cases = (
            {"currentModelId": "gpt-5.6-sol"},
            {
                "currentModelId": expected["model"],
                "effectiveModel": "gpt-5.6-sol",
            },
            {
                "current_model_id": expected["model"],
                "effective_model": "gpt-5.6-sol",
            },
        )
        for models in cases:
            with self.subTest(models=models):
                result = PROFILE._probe_result(
                    "code-luna",
                    complete_probe_exchange(models),
                )
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "malformed")

    def test_only_copilot_auth_survives_environment_projection(self) -> None:
        projected = PROFILE.scrub_environment(
            {
                "PATH": "/runtime/bin",
                "COPILOT_GITHUB_TOKEN": "token",
                "GITHUB_TOKEN": "wrong-token",
                "DISCORD_TOKEN": "discord",
                "BUILD_BUDDY_API_KEY": "buildbuddy",
            }
        )
        self.assertEqual(projected["COPILOT_GITHUB_TOKEN"], "token")
        self.assertNotIn("GITHUB_TOKEN", projected)
        self.assertNotIn("DISCORD_TOKEN", projected)
        self.assertNotIn("BUILD_BUDDY_API_KEY", projected)
        self.assertNotIn("GC_AGENT_LAUNCHER_TOKEN", PROFILE.scrub_environment(
            {"GC_AGENT_LAUNCHER_TOKEN": "launcher-secret"}
        ))

    def test_probe_uses_ndjson_and_closed_is_classified(self) -> None:
        frame = PROFILE._frame({"jsonrpc": "2.0", "id": 1, "method": "initialize"})
        self.assertTrue(frame.endswith(b"\n"))
        self.assertNotIn(b"Content-Length", frame)
        self.assertEqual(
            PROFILE._probe_error_code("ACP process closed its stdout"),
            "closed",
        )
        self.assertNotIn("Content-Length:", FAKE_ACP.read_text(encoding="utf-8"))

    def test_probe_uses_sandbox_cwd_for_socket_and_host_cwd_for_direct_fixture(self) -> None:
        self.assertEqual(SANDBOX.SANDBOX_WORKSPACE, PROFILE.SANDBOX_WORKSPACE)
        with tempfile.TemporaryDirectory(prefix="gascity-probe-cwd-") as raw:
            worktree = pathlib.Path(raw) / "fixture-worktree"
            worktree.mkdir()
            expected_model = PROFILE.PROFILE_SETTINGS["code-luna"]["model"]

            for fixture_direct, expected_cwd in (
                (False, PROFILE.SANDBOX_WORKSPACE),
                (True, str(worktree)),
            ):
                with self.subTest(fixture_direct=fixture_direct):
                    args = PROFILE._default_namespace("code-luna", "coding")
                    args.fixture_direct = fixture_direct
                    args.worktree = str(worktree)
                    responses = iter(
                        complete_probe_exchange(
                            {"currentModelId": expected_model}
                        )
                    )
                    sent: list[bytes] = []

                    class Reader:
                        def read(self, _timeout: float) -> dict[str, object]:
                            return next(responses)

                    result = PROFILE._probe_exchange(
                        Reader(),
                        sent.append,
                        profile="code-luna",
                        args=args,
                        timeout=1,
                    )

                    self.assertTrue(result["ok"])
                    self.assertEqual(
                        json.loads(sent[1].decode("utf-8"))["params"]["cwd"],
                        expected_cwd,
                    )

    def test_normal_socket_sessions_rewrite_host_cwd_to_sandbox_cwd(self) -> None:
        message = (
            b'{"jsonrpc":"2.0","id":2,"method":"session/new",'
            b'"params":{"cwd":"/host/worktree","mcpServers":[]}}\n'
        )
        rewritten = json.loads(PROFILE._sandbox_session_message(message))
        self.assertEqual(rewritten["params"]["cwd"], PROFILE.SANDBOX_WORKSPACE)


class ProxyStdioContractTests(unittest.TestCase):
    channel_fd = 91

    class PollHarness:
        def __init__(self, batches: list[list[tuple[int, int]]]):
            self.batches = iter(batches)
            self.calls = 0
            self.unregistered: list[int] = []

        def register(self, _descriptor: int, _events: int) -> None:
            return None

        def unregister(self, descriptor: int) -> None:
            self.unregistered.append(descriptor)

        def poll(self, _timeout: int) -> list[tuple[int, int]]:
            self.calls += 1
            try:
                return next(self.batches)
            except StopIteration as error:
                raise AssertionError("proxy polled after the channel closed") from error

    def _run_proxy(
        self,
        data: bytes,
        *,
        events: int,
    ) -> tuple[int, mock.Mock, str]:
        poller = self.PollHarness(
            [
                [(0, events), (self.channel_fd, PROFILE.select.POLLHUP)],
            ]
        )
        channel = mock.Mock()
        channel.fileno.return_value = self.channel_fd
        channel.recv.return_value = b""
        reads = iter((data, b""))
        diagnostic = io.StringIO()
        with mock.patch.object(
            PROFILE.select,
            "poll",
            return_value=poller,
        ), mock.patch.object(
            PROFILE.os,
            "read",
            side_effect=lambda _fd, _limit: next(reads),
        ), mock.patch.object(
            PROFILE.sys,
            "stderr",
            diagnostic,
        ):
            result = PROFILE._proxy_stdio(channel)
        return result, channel, diagnostic.getvalue()

    def test_pollin_hup_same_event_rewrites_final_newline_terminated_session(self) -> None:
        frame = (
            b'{"jsonrpc":"2.0","id":1,"method":"session/new",'
            b'"params":{"cwd":"/host/worktree"}}\n'
        )
        result, channel, diagnostic = self._run_proxy(
            frame,
            events=PROFILE.select.POLLIN | PROFILE.select.POLLHUP,
        )
        self.assertEqual(result, 0)
        self.assertEqual(
            channel.sendall.call_args_list,
            [mock.call(PROFILE._sandbox_session_message(frame))],
        )
        self.assertEqual(diagnostic, "")

    def test_multiple_lines_are_framed_and_partial_final_is_dropped(self) -> None:
        initialize = b'{"jsonrpc":"2.0","id":1,"method":"initialize"}\n'
        session = (
            b'{"jsonrpc":"2.0","id":2,"method":"session/new",'
            b'"params":{"cwd":"/host/worktree"}}\n'
        )
        partial = (
            b'{"jsonrpc":"2.0","id":3,"method":"session/new",'
            b'"params":{"cwd":"/host/worktree/partial"}'
        )
        result, channel, diagnostic = self._run_proxy(
            initialize + session + partial,
            events=PROFILE.select.POLLIN | PROFILE.select.POLLHUP,
        )
        forwarded = b"".join(
            call.args[0] for call in channel.sendall.call_args_list
        )
        self.assertEqual(
            forwarded,
            initialize + PROFILE._sandbox_session_message(session),
        )
        self.assertEqual(result, 1)
        self.assertIn("unterminated", diagnostic)
        self.assertNotIn(
            b"/host/worktree/partial",
            forwarded + diagnostic.encode(),
        )

    def test_non_session_final_frame_is_preserved_byte_for_byte(self) -> None:
        frame = (
            b'{"jsonrpc":"2.0","id":7,"method":"session/prompt",'
            b'"params":{"cwd":"/host/worktree"}}\n'
        )
        result, channel, diagnostic = self._run_proxy(
            frame,
            events=PROFILE.select.POLLIN | PROFILE.select.POLLHUP,
        )
        self.assertEqual(result, 0)
        self.assertEqual(channel.sendall.call_args_list, [mock.call(frame)])
        self.assertEqual(diagnostic, "")

    def test_unterminated_session_new_is_rejected_without_host_path_leak(self) -> None:
        partial = (
            b'{"jsonrpc":"2.0","id":8,"method":"session/new",'
            b'"params":{"cwd":"/host/worktree/secret"}'
        )
        result, channel, diagnostic = self._run_proxy(
            partial,
            events=PROFILE.select.POLLIN | PROFILE.select.POLLHUP,
        )
        self.assertEqual(result, 1)
        self.assertEqual(channel.sendall.call_args_list, [])
        self.assertIn("unterminated ACP frame", diagnostic)
        self.assertNotIn("/host/worktree/secret", diagnostic)

    def test_pollnval_stdin_closes_write_side_without_poll_spin(self) -> None:
        poller = self.PollHarness(
            [
                [
                    (0, PROFILE.select.POLLNVAL),
                    (self.channel_fd, PROFILE.select.POLLHUP),
                ]
            ]
        )
        channel = mock.Mock()
        channel.fileno.return_value = self.channel_fd
        channel.recv.return_value = b""
        read = mock.Mock(side_effect=AssertionError("POLLNVAL read from stdin"))
        with mock.patch.object(
            PROFILE.select,
            "poll",
            return_value=poller,
        ), mock.patch.object(
            PROFILE.os,
            "read",
            read,
        ):
            result = PROFILE._proxy_stdio(channel)

        self.assertEqual(result, 0)
        self.assertEqual(poller.calls, 1)
        self.assertEqual(poller.unregistered, [0])
        read.assert_not_called()
        channel.shutdown.assert_called_once_with(socket.SHUT_WR)
        channel.close.assert_called_once_with()


class RoleRoutingContractTests(unittest.TestCase):
    def test_matrix_tool_policies_match_city_and_launcher_mappings(self) -> None:
        matrix = tomllib.loads(ROLE_MATRIX.read_text(encoding="utf-8"))
        city = tomllib.loads(CITY.read_text(encoding="utf-8"))
        expected = {
            "planning-sol": ("planning-artifacts", "planning"),
            "review-sol": ("read-only-review", "review"),
            "review-luna": ("read-only-review", "review"),
            "code-luna": ("worktree-edit-check", "coding"),
        }
        for profile, (matrix_policy, executable_policy) in expected.items():
            profile_value = matrix["profiles"][profile]
            self.assertEqual(profile_value["tool_policy"], matrix_policy)
            provider = city["providers"][profile_value["provider"]]
            acp_args = provider["acp_args"]
            policy_index = acp_args.index("--tool-policy")
            self.assertEqual(acp_args[policy_index + 1], executable_policy)
            self.assertEqual(
                LAUNCHER.TOOL_POLICIES[executable_policy],
                PROFILE.TOOL_POLICIES[executable_policy],
            )


class ActivationContractTests(unittest.TestCase):
    def test_fdproxy_sidecar_accepts_real_runtime_executable(self) -> None:
        fdproxy_value = os.environ.get("GAS_CITY_FDPROXY")
        if not fdproxy_value:
            self.skipTest("GAS_CITY_FDPROXY is only set by the packaged smoke")
        fdproxy = pathlib.Path(fdproxy_value)
        self.assertTrue(fdproxy.is_file())
        self.assertFalse(fdproxy.is_symlink())

        with tempfile.TemporaryDirectory(prefix="gascity-fdproxy-sidecar-") as raw:
            socket_path = pathlib.Path(raw) / "egress.sock"
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(socket_path))
            listener.listen(1)
            child = mock.Mock()
            child.wait.return_value = 0
            try:
                with mock.patch.object(
                    ACTIVATION.subprocess,
                    "Popen",
                    return_value=child,
                ) as popen:
                    result = ACTIVATION.run_fdproxy_sidecar(
                        egress_socket=str(socket_path),
                        fdproxy_script=str(fdproxy),
                        listen="127.0.0.1:3128",
                        command=["wrapped-sidecar"],
                        server_uid=os.getuid(),
                    )
            finally:
                listener.close()

        self.assertEqual(result, 0)
        argv = popen.call_args.args[0]
        self.assertEqual(argv[1], str(fdproxy))

    def test_silent_metadata_selects_exact_readiness_profiles(self) -> None:
        calls: list[str] = []

        def probe(profile: str):
            calls.append(profile)
            return PROFILE._probe_result(
                profile,
                complete_probe_exchange(
                    {
                        "availableModels": [
                            {"modelId": "gpt-5.6-sol"},
                            {"modelId": "gpt-5.6-luna"},
                            {"modelId": "gpt-4.1"},
                        ]
                    },
                ),
            )

        status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
        self.assertEqual(
            status,
            {
                "generation": "g1",
                "state_schema": "1",
                "ready": True,
                "effective_profiles": {
                    "coding": "code-luna",
                    "review": "review-sol",
                },
                "error_code": None,
            },
        )
        self.assertEqual(calls, ["code-luna", "review-sol"])

    def test_only_unsupported_or_unavailable_sol_failures_fallback(self) -> None:
        for failure_code in ("unsupported", "unavailable"):
            with self.subTest(failure_code=failure_code):
                calls: list[str] = []

                def probe(profile: str):
                    calls.append(profile)
                    if profile == "review-sol":
                        return {
                            "profile": profile,
                            "ok": False,
                            "error_code": failure_code,
                        }
                    return successful_probe(profile)

                status = ACTIVATION.select_profiles(
                    probe,
                    generation="g1",
                    state_schema="1",
                )
                self.assertTrue(status["ready"])
                self.assertEqual(status["effective_profiles"]["review"], "review-luna")
                self.assertEqual(calls, ["code-luna", "review-sol", "review-luna"])

    def test_sol_server_request_blocks_without_luna_fallback(self) -> None:
        calls: list[str] = []
        server_request = {
            "profile": "review-sol",
            "ok": False,
            "error_code": "malformed",
            "error": "ACP server requests are malformed in preflight",
        }

        def probe(profile: str):
            calls.append(profile)
            if profile == "review-sol":
                return server_request
            return successful_probe(profile)

        status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
        self.assertFalse(status["ready"])
        self.assertEqual(status["error_code"], "review-sol-malformed")
        self.assertEqual(calls, ["code-luna", "review-sol"])

    def test_sol_closed_failure_does_not_fallback(self) -> None:
        self.assertEqual(ACTIVATION.classify_failure("ACP process closed EOF"), "closed")
        calls: list[str] = []

        def probe(profile: str):
            calls.append(profile)
            if profile == "review-sol":
                return {"profile": profile, "ok": False, "error_code": "closed"}
            return successful_probe(profile)

        status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
        self.assertFalse(status["ready"])
        self.assertEqual(status["error_code"], "review-sol-closed")
        self.assertEqual(calls, ["code-luna", "review-sol"])

    def test_sol_auth_network_quota_malformed_and_unknown_block(self) -> None:
        for code in ("authentication", "network", "quota", "malformed", "unknown"):
            calls: list[str] = []

            def probe(profile: str, failure_code: str = code):
                calls.append(profile)
                if profile == "review-sol":
                    return {
                        "profile": profile,
                        "ok": False,
                        "error_code": failure_code,
                    }
                return successful_probe(profile)

            status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
            self.assertFalse(status["ready"])
            self.assertEqual(calls, ["code-luna", "review-sol"])

    def test_luna_failure_never_falls_back_again(self) -> None:
        def probe(profile: str):
            if profile == "code-luna":
                return successful_probe(profile)
            return {"profile": profile, "ok": False, "error_code": "unsupported"}

        status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
        self.assertFalse(status["ready"])
        self.assertEqual(status["error_code"], "review-luna-unsupported")

    def test_generation_bound_readiness_rejects_work_before_ready(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-readiness-") as raw:
            status_path = pathlib.Path(raw) / "status.json"
            blocked = ACTIVATION._blocked_status("g1", "1", "code-luna-network")
            ACTIVATION.write_status(status_path, blocked)
            with self.assertRaises(ACTIVATION.ActivationError):
                ACTIVATION.require_ready(
                    status_path,
                    generation="g1",
                    state_schema="1",
                    profile="code-luna",
                )
            ready = ACTIVATION._ready_status("g1", "1", review_profile="review-sol")
            ACTIVATION.write_status(status_path, ready)
            with self.assertRaises(ACTIVATION.StaleGeneration):
                ACTIVATION.require_ready(
                    status_path,
                    generation="g2",
                    state_schema="1",
                    profile="code-luna",
                )


class ManagedStorePathValidationContractTests(unittest.TestCase):
    def test_missing_valid_store_objects_with_expected_asset_name_are_valid(self) -> None:
        for object_name in (
            "0123456789abcdfghijklmnpqrsvwxyz-retired-generation",
            "0123456789abcdfghijklmnpqrsvwxyz-name+with._?=-punctuation",
        ):
            with self.subTest(object_name=object_name):
                path = pathlib.Path(f"/nix/store/{object_name}/city")
                self.assertEqual(
                    ACTIVATION._validated_store_path(
                        path,
                        "old managed asset target",
                        expected_basename="city",
                        require_directory=True,
                    ),
                    path,
                )

    def test_missing_malformed_pseudo_store_object_is_rejected(self) -> None:
        for object_name in (
            "not-a-store-object",
            "0123456789abcdfghijklmnpqrsvwxyze-retired-generation",
            "0123456789abcdfghijklmnpqrsvwxyz-.",
            "0123456789abcdfghijklmnpqrsvwxyz-..",
        ):
            with self.subTest(object_name=object_name):
                with self.assertRaises(ACTIVATION.BoundaryError):
                    ACTIVATION._validated_store_path(
                        f"/nix/store/{object_name}/city",
                        "old managed asset target",
                        expected_basename="city",
                        require_directory=True,
                    )


class ManagedAssetDirectoryContractTests(unittest.TestCase):
    def test_missing_destination_sets_expected_metadata_on_anchored_fd(self) -> None:
        scratch = _managed_asset_scratch_root()
        scratch.mkdir(mode=0o700, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="gascity-managed-directory-",
            dir=scratch,
        ) as raw:
            destination = pathlib.Path(raw) / "managed"
            expected_uid = 0
            expected_gid = max(os.getgid(), 0) + 12345
            self.assertNotEqual(expected_gid, os.getgid())
            destination_fds: list[int] = []
            metadata_calls: list[tuple[object, ...]] = []
            real_open = ACTIVATION.os.open
            real_fstat = ACTIVATION.os.fstat

            def record_open(path, flags, mode=0o777, *, dir_fd=None):
                descriptor = real_open(path, flags, mode, dir_fd=dir_fd)
                if path == destination.name and dir_fd is not None:
                    destination_fds.append(descriptor)
                return descriptor

            def fake_fstat(fd: int):
                info = real_fstat(fd)
                if destination.exists():
                    destination_info = os.lstat(destination)
                    if (
                        info.st_dev == destination_info.st_dev
                        and info.st_ino == destination_info.st_ino
                    ):
                        return SimpleNamespace(
                            st_dev=info.st_dev,
                            st_ino=info.st_ino,
                            st_mode=stat.S_IFDIR | 0o750,
                            st_uid=expected_uid,
                            st_gid=expected_gid,
                        )
                return info

            def record_fchown(fd: int, uid: int, gid: int) -> None:
                metadata_calls.append(("fchown", fd, uid, gid))

            def record_fchmod(fd: int, mode: int) -> None:
                metadata_calls.append(("fchmod", fd, mode))

            parent_fd = -1
            destination_fd = -1
            with mock.patch.object(
                ACTIVATION.os,
                "open",
                side_effect=record_open,
            ), mock.patch.object(
                ACTIVATION.os,
                "fstat",
                side_effect=fake_fstat,
            ), mock.patch.object(
                ACTIVATION.os,
                "fchown",
                side_effect=record_fchown,
            ), mock.patch.object(
                ACTIVATION.os,
                "fchmod",
                side_effect=record_fchmod,
            ), mock.patch.object(
                ACTIVATION.os,
                "chown",
            ) as path_chown, mock.patch.object(
                ACTIVATION.os,
                "chmod",
            ) as path_chmod:
                parent_fd, destination_fd = ACTIVATION._open_managed_asset_directory(
                    destination,
                    expected_uid=expected_uid,
                    expected_gid=expected_gid,
                )
                path_chown.assert_not_called()
                path_chmod.assert_not_called()

            try:
                self.assertEqual(len(destination_fds), 1)
                anchored_fd = destination_fds[0]
                self.assertEqual(
                    metadata_calls,
                    [
                        ("fchown", anchored_fd, expected_uid, expected_gid),
                        ("fchmod", anchored_fd, 0o750),
                    ],
                )
                self.assertEqual(destination_fd, anchored_fd)
            finally:
                if destination_fd >= 0:
                    os.close(destination_fd)
                if parent_fd >= 0:
                    os.close(parent_fd)


def _managed_asset_scratch_root() -> pathlib.Path:
    raw_value = os.environ.get(MANAGED_ASSET_SCRATCH_ROOT_ENV)
    if raw_value is None:
        return ROOT / ".scratch"
    try:
        path = ACTIVATION._absolute_normalized_path(
            raw_value,
            "managed asset scratch root",
        )
    except ACTIVATION.BoundaryError as error:
        raise ValueError(
            "managed asset scratch root must be an absolute canonical path"
        ) from error
    if path == pathlib.Path("/"):
        raise ValueError("managed asset scratch root must not be the filesystem root")
    try:
        ACTIVATION._check_ancestor_chain(path, "managed asset scratch root")
    except ACTIVATION.BoundaryError as error:
        raise ValueError(
            "managed asset scratch root has an unsafe ancestor"
        ) from error
    try:
        info = os.lstat(path)
    except FileNotFoundError:
        return path
    except OSError as error:
        raise ValueError("managed asset scratch root is unavailable") from error
    if (
        stat.S_ISLNK(info.st_mode)
        or not stat.S_ISDIR(info.st_mode)
        or info.st_mode & 0o022
    ):
        raise ValueError(
            "managed asset scratch root must be a private directory"
        )
    return path


class ManagedAssetScratchRootContractTests(unittest.TestCase):
    def test_default_uses_repository_scratch(self) -> None:
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop(MANAGED_ASSET_SCRATCH_ROOT_ENV, None)
            self.assertEqual(_managed_asset_scratch_root(), ROOT / ".scratch")

    def test_override_rejects_nonabsolute_or_unsafe_values(self) -> None:
        for value in ("relative-scratch", "/tmp", "/tmp/gascity-fixtures/.scratch"):
            with self.subTest(value=value), mock.patch.dict(
                os.environ,
                {MANAGED_ASSET_SCRATCH_ROOT_ENV: value},
            ):
                with self.assertRaises(ValueError):
                    _managed_asset_scratch_root()


class ManagedAssetRotationContractTests(unittest.TestCase):
    def setUp(self) -> None:
        old_value = os.environ.get("GC_MANAGED_ASSET_OLD")
        new_value = os.environ.get("GC_MANAGED_ASSET_NEW")
        if not old_value or not new_value:
            self.skipTest("realized managed-asset fixtures are unavailable")
        self.old = pathlib.Path(old_value)
        self.new = pathlib.Path(new_value)
        for generation in (self.old, self.new):
            self.assertTrue(
                str(generation).startswith("/nix/store/"),
                f"managed fixture is not a store path: {generation}",
            )
            for name in ACTIVATION.MANAGED_ASSET_NAMES:
                self.assertTrue((generation / name).is_dir())

    def _temporary_destination(self) -> tempfile.TemporaryDirectory[str]:
        scratch = _managed_asset_scratch_root()
        scratch.mkdir(mode=0o700, exist_ok=True)
        return tempfile.TemporaryDirectory(
            prefix="gascity-managed-assets-",
            dir=scratch,
        )

    @staticmethod
    def _materialize(source: pathlib.Path, destination: pathlib.Path) -> None:
        ACTIVATION.materialize_assets(
            str(source),
            str(destination),
            expected_uid=os.geteuid(),
            expected_gid=os.getgid(),
        )

    @staticmethod
    def _links(destination: pathlib.Path) -> dict[str, str]:
        return {
            name: os.readlink(destination / name)
            for name in ACTIVATION.MANAGED_ASSET_NAMES
        }

    @staticmethod
    def _temporary_entries(destination: pathlib.Path) -> list[pathlib.Path]:
        return sorted(
            entry
            for entry in destination.iterdir()
            if entry.name.startswith(".") and entry.name.endswith(".tmp")
        )

    @staticmethod
    def _destination_fstat_override(
        destination: pathlib.Path,
        *,
        uid: int,
        gid: int,
        mode: int,
    ):
        real_fstat = ACTIVATION.os.fstat
        destination_info = os.stat(destination, follow_symlinks=False)

        def fake_fstat(fd: int):
            info = real_fstat(fd)
            if (
                info.st_dev == destination_info.st_dev
                and info.st_ino == destination_info.st_ino
            ):
                return SimpleNamespace(
                    st_dev=info.st_dev,
                    st_ino=info.st_ino,
                    st_mode=stat.S_IFDIR | mode,
                    st_uid=uid,
                    st_gid=gid,
                )
            return info

        return fake_fstat

    @staticmethod
    def _destination_fstat_transition(
        destination: pathlib.Path,
        *,
        initial_uid: int,
        initial_gid: int,
        initial_mode: int,
        final_uid: int,
        final_gid: int,
        final_mode: int,
    ):
        real_fstat = ACTIVATION.os.fstat
        destination_info = os.stat(destination, follow_symlinks=False)
        repaired = False

        def mark_repaired(*_args: object) -> None:
            nonlocal repaired
            repaired = True

        def fake_fstat(fd: int):
            info = real_fstat(fd)
            if (
                info.st_dev == destination_info.st_dev
                and info.st_ino == destination_info.st_ino
            ):
                if repaired:
                    uid, gid, mode = final_uid, final_gid, final_mode
                else:
                    uid, gid, mode = initial_uid, initial_gid, initial_mode
                return SimpleNamespace(
                    st_dev=info.st_dev,
                    st_ino=info.st_ino,
                    st_mode=stat.S_IFDIR | mode,
                    st_uid=uid,
                    st_gid=gid,
                )
            return info

        return fake_fstat, mark_repaired

    def test_same_generation_is_idempotent(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            before = {
                name: os.lstat(destination / name).st_ino
                for name in ACTIVATION.MANAGED_ASSET_NAMES
            }
            with mock.patch.object(
                ACTIVATION.os,
                "replace",
                side_effect=AssertionError("same-generation materialization rotated"),
            ):
                self._materialize(self.old, destination)
            self.assertEqual(
                before,
                {
                    name: os.lstat(destination / name).st_ino
                    for name in ACTIVATION.MANAGED_ASSET_NAMES
                },
            )
            self.assertEqual(self._links(destination), {
                name: str(self.old / name)
                for name in ACTIVATION.MANAGED_ASSET_NAMES
            })
            self.assertEqual(self._temporary_entries(destination), [])

    def test_destination_uses_fixture_identity_and_shared_read_mode(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            info = os.stat(destination, follow_symlinks=False)
            self.assertEqual(info.st_uid, os.geteuid())
            self.assertEqual(info.st_gid, os.getgid())
            self.assertEqual(stat.S_IMODE(info.st_mode), 0o750)

    def test_materializer_requires_explicit_expected_group(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            with self.assertRaises(ACTIVATION.BoundaryError):
                ACTIVATION.materialize_assets(
                    str(self.old),
                    str(destination),
                    expected_uid=os.geteuid(),
                )

    def test_group_contract_resolves_name_and_numeric_fixture_gid(self) -> None:
        gid = os.getgid()
        group = ACTIVATION.grp.getgrgid(gid).gr_name
        self.assertEqual(ACTIVATION._resolve_group_gid(group), gid)
        self.assertEqual(ACTIVATION._resolve_group_gid(str(gid)), gid)

    def test_destination_requires_expected_owner_group_and_mode(self) -> None:
        expected_gid = os.getgid()
        cases = (
            ("non-root owner", 1000, expected_gid, 0o750),
            ("unexpected group", 0, expected_gid + 1, 0o750),
            ("group-writable", 0, expected_gid, 0o760),
            ("other-readable", 0, expected_gid, 0o751),
            ("owner-not-searchable", 0, expected_gid, 0o650),
        )
        for label, uid, gid, mode in cases:
            with self.subTest(destination=label), self._temporary_destination() as raw:
                destination = pathlib.Path(raw) / "managed"
                destination.mkdir(mode=0o700)
                fake_fstat = self._destination_fstat_override(
                    destination,
                    uid=uid,
                    gid=gid,
                    mode=mode,
                )
                with mock.patch.object(ACTIVATION.os, "geteuid", return_value=0):
                    with mock.patch.object(
                        ACTIVATION.os,
                        "fstat",
                        side_effect=fake_fstat,
                    ):
                        with self.assertRaises(ACTIVATION.BoundaryError):
                            ACTIVATION.materialize_assets(
                                str(self.old),
                                str(destination),
                                expected_uid=0,
                                expected_gid=expected_gid,
                            )

    def test_interrupted_metadata_failure_recovers_on_restart(self) -> None:
        expected_uid = 0
        expected_gid = os.getgid() + 12345
        cases = (
            (
                "fchown",
                0,
                0o700,
                OSError("injected fchown failure"),
                None,
            ),
            (
                "fchmod",
                expected_gid,
                0o700,
                None,
                OSError("injected fchmod failure"),
            ),
        )
        for label, partial_gid, partial_mode, chown_failure, chmod_failure in cases:
            with self.subTest(failure=label), self._temporary_destination() as raw:
                destination = pathlib.Path(raw) / "managed"
                destination.mkdir(mode=0o700)
                partial_fstat = self._destination_fstat_override(
                    destination,
                    uid=0,
                    gid=partial_gid,
                    mode=partial_mode,
                )
                with mock.patch.object(
                    ACTIVATION.os,
                    "fstat",
                    side_effect=partial_fstat,
                ), mock.patch.object(
                    ACTIVATION.os,
                    "fchown",
                    side_effect=chown_failure,
                ) as fchown, mock.patch.object(
                    ACTIVATION.os,
                    "fchmod",
                    side_effect=chmod_failure,
                ) as fchmod:
                    with self.assertRaisesRegex(
                        ACTIVATION.BoundaryError,
                        "metadata could not be set",
                    ):
                        ACTIVATION._open_managed_asset_directory(
                            destination,
                            expected_uid=expected_uid,
                            expected_gid=expected_gid,
                        )
                fchown.assert_called_once()
                if label == "fchown":
                    fchmod.assert_not_called()
                else:
                    fchmod.assert_called_once()

                recovery_fstat, mark_repaired = self._destination_fstat_transition(
                    destination,
                    initial_uid=0,
                    initial_gid=partial_gid,
                    initial_mode=partial_mode,
                    final_uid=expected_uid,
                    final_gid=expected_gid,
                    final_mode=0o750,
                )
                metadata_calls: list[tuple[object, ...]] = []
                parent_fd = -1
                destination_fd = -1

                def record_fchown(fd: int, uid: int, gid: int) -> None:
                    metadata_calls.append(("fchown", fd, uid, gid))

                def record_fchmod(fd: int, mode: int) -> None:
                    metadata_calls.append(("fchmod", fd, mode))
                    mark_repaired()

                with mock.patch.object(
                    ACTIVATION.os,
                    "fstat",
                    side_effect=recovery_fstat,
                ), mock.patch.object(
                    ACTIVATION.os,
                    "fchown",
                    side_effect=record_fchown,
                ), mock.patch.object(
                    ACTIVATION.os,
                    "fchmod",
                    side_effect=record_fchmod,
                ), mock.patch.object(
                    ACTIVATION.os,
                    "chown",
                ) as path_chown, mock.patch.object(
                    ACTIVATION.os,
                    "chmod",
                ) as path_chmod:
                    parent_fd, destination_fd = ACTIVATION._open_managed_asset_directory(
                        destination,
                        expected_uid=expected_uid,
                        expected_gid=expected_gid,
                    )
                try:
                    self.assertEqual(
                        metadata_calls,
                        [
                            ("fchown", destination_fd, expected_uid, expected_gid),
                            ("fchmod", destination_fd, 0o750),
                        ],
                    )
                    path_chown.assert_not_called()
                    path_chmod.assert_not_called()
                finally:
                    if destination_fd >= 0:
                        os.close(destination_fd)
                    if parent_fd >= 0:
                        os.close(parent_fd)

    def test_root_owned_empty_partial_modes_recover_after_interruption(self) -> None:
        expected_uid = 0
        expected_gid = os.getgid() + 12345
        for partial_mode in (0o700, 0o750):
            with self.subTest(mode=oct(partial_mode)), self._temporary_destination() as raw:
                destination = pathlib.Path(raw) / "managed"
                destination.mkdir(mode=0o700)
                fake_fstat, mark_repaired = self._destination_fstat_transition(
                    destination,
                    initial_uid=0,
                    initial_gid=0,
                    initial_mode=partial_mode,
                    final_uid=expected_uid,
                    final_gid=expected_gid,
                    final_mode=0o750,
                )
                parent_fd = -1
                destination_fd = -1
                with mock.patch.object(
                    ACTIVATION.os,
                    "fstat",
                    side_effect=fake_fstat,
                ), mock.patch.object(
                    ACTIVATION.os,
                    "fchown",
                ) as fchown, mock.patch.object(
                    ACTIVATION.os,
                    "fchmod",
                    side_effect=mark_repaired,
                ) as fchmod:
                    parent_fd, destination_fd = ACTIVATION._open_managed_asset_directory(
                        destination,
                        expected_uid=expected_uid,
                        expected_gid=expected_gid,
                    )
                try:
                    fchown.assert_called_once_with(
                        destination_fd,
                        expected_uid,
                        expected_gid,
                    )
                    fchmod.assert_called_once_with(destination_fd, 0o750)
                finally:
                    if destination_fd >= 0:
                        os.close(destination_fd)
                    if parent_fd >= 0:
                        os.close(parent_fd)

    def test_partial_metadata_recovery_refuses_foreign_directories(self) -> None:
        expected_uid = 0
        expected_gid = os.getgid() + 12345
        cases = (
            (
                "nonempty",
                0,
                0,
                0o700,
                lambda destination: (destination / "foreign").write_text(
                    "foreign\n",
                    encoding="utf-8",
                ),
            ),
            (
                "foreign-symlink-entry",
                0,
                0,
                0o700,
                lambda destination: os.symlink(
                    "/tmp/foreign-managed-entry",
                    destination / "foreign",
                ),
            ),
            ("wrong-owner", 1000, 0, 0o700, None),
            ("writable", 0, 0, 0o770, None),
        )
        for label, uid, gid, mode, install in cases:
            with self.subTest(destination=label), self._temporary_destination() as raw:
                destination = pathlib.Path(raw) / "managed"
                destination.mkdir(mode=0o700)
                if install is not None:
                    install(destination)
                fake_fstat = self._destination_fstat_override(
                    destination,
                    uid=uid,
                    gid=gid,
                    mode=mode,
                )
                with mock.patch.object(
                    ACTIVATION.os,
                    "fstat",
                    side_effect=fake_fstat,
                ), mock.patch.object(
                    ACTIVATION.os,
                    "fchown",
                ) as fchown, mock.patch.object(
                    ACTIVATION.os,
                    "fchmod",
                ) as fchmod:
                    with self.assertRaises(ACTIVATION.BoundaryError):
                        ACTIVATION._open_managed_asset_directory(
                            destination,
                            expected_uid=expected_uid,
                            expected_gid=expected_gid,
                        )
                fchown.assert_not_called()
                fchmod.assert_not_called()

    def test_materialization_anchors_link_operations_to_one_directory_fd(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            lstat_fds: list[int | None] = []
            readlink_fds: list[int | None] = []
            symlink_fds: list[int | None] = []
            replace_fds: list[tuple[int | None, int | None]] = []
            fsync_fds: list[int] = []
            open_calls: list[tuple[object, int, int | None]] = []
            parent_open_fds: list[int] = []
            required_directory_flags = (
                os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
            )
            real_open = ACTIVATION.os.open
            real_lstat = ACTIVATION.os.lstat
            real_readlink = ACTIVATION.os.readlink
            real_symlink = ACTIVATION.os.symlink
            real_replace = ACTIVATION.os.replace
            real_fsync = ACTIVATION.os.fsync

            def record_open(path, flags, mode=0o777, *, dir_fd=None):
                open_calls.append((path, flags, dir_fd))
                descriptor = real_open(path, flags, mode, dir_fd=dir_fd)
                if path == destination.parent and dir_fd is None:
                    parent_open_fds.append(descriptor)
                return descriptor

            def record_lstat(path, *args, **kwargs):
                if path in ACTIVATION.MANAGED_ASSET_NAMES:
                    lstat_fds.append(kwargs.get("dir_fd"))
                return real_lstat(path, *args, **kwargs)

            def record_readlink(path, *args, **kwargs):
                if path in ACTIVATION.MANAGED_ASSET_NAMES:
                    readlink_fds.append(kwargs.get("dir_fd"))
                return real_readlink(path, *args, **kwargs)

            def record_symlink(source, target, *args, **kwargs):
                symlink_fds.append(kwargs.get("dir_fd"))
                return real_symlink(source, target, *args, **kwargs)

            def record_replace(source, target, *args, **kwargs):
                replace_fds.append(
                    (kwargs.get("src_dir_fd"), kwargs.get("dst_dir_fd"))
                )
                return real_replace(source, target, *args, **kwargs)

            def record_fsync(fd: int):
                fsync_fds.append(fd)
                return real_fsync(fd)

            with mock.patch.object(
                ACTIVATION.os,
                "open",
                side_effect=record_open,
            ), mock.patch.object(
                ACTIVATION.os,
                "lstat",
                side_effect=record_lstat,
            ), mock.patch.object(
                ACTIVATION.os,
                "readlink",
                side_effect=record_readlink,
            ), mock.patch.object(
                ACTIVATION.os,
                "symlink",
                side_effect=record_symlink,
            ), mock.patch.object(
                ACTIVATION.os,
                "replace",
                side_effect=record_replace,
            ), mock.patch.object(
                ACTIVATION.os,
                "fsync",
                side_effect=record_fsync,
            ):
                self._materialize(self.old, destination)
                self._materialize(self.new, destination)

            destination_fds = set(lstat_fds + readlink_fds + symlink_fds)
            destination_fds.update(
                fd
                for pair in replace_fds
                for fd in pair
            )
            self.assertEqual(len(destination_fds), 1)
            directory_fd = next(iter(destination_fds))
            self.assertIsNotNone(directory_fd)
            parent_fds = set(parent_open_fds)
            self.assertTrue(parent_fds)
            self.assertTrue(lstat_fds)
            self.assertTrue(readlink_fds)
            self.assertTrue(symlink_fds)
            self.assertTrue(replace_fds)
            self.assertTrue(fsync_fds)
            self.assertIn(directory_fd, fsync_fds)
            self.assertTrue(parent_fds.intersection(fsync_fds))
            destination_opens = [
                flags
                for path, flags, dir_fd in open_calls
                if path == destination.name and dir_fd is not None
            ]
            self.assertTrue(destination_opens)
            self.assertTrue(
                all(flags & required_directory_flags == required_directory_flags
                    for flags in destination_opens)
            )
            self.assertTrue(all(fd == directory_fd for fd in lstat_fds))
            self.assertTrue(all(fd == directory_fd for fd in readlink_fds))
            self.assertTrue(all(fd == directory_fd for fd in symlink_fds))
            self.assertTrue(
                all(
                    src_fd == directory_fd and dst_fd == directory_fd
                    for src_fd, dst_fd in replace_fds
                )
            )
            self.assertTrue(
                set(fsync_fds).issubset(parent_fds | {directory_fd})
            )

    def test_package_generation_rotation_replaces_only_managed_links(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            old_links = self._links(destination)
            self._materialize(self.new, destination)
            self.assertNotEqual(old_links, self._links(destination))
            self.assertEqual(self._links(destination), {
                name: str(self.new / name)
                for name in ACTIVATION.MANAGED_ASSET_NAMES
            })
            self.assertEqual(self._temporary_entries(destination), [])

    def test_gc_removed_valid_store_target_can_be_rotated(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            target = destination / "city"
            target.unlink()
            os.symlink(
                "/nix/store/0123456789abcdfghijklmnpqrsvwxyz-removed-generation/city",
                target,
            )
            self._materialize(self.new, destination)
            self.assertEqual(os.readlink(target), str(self.new / "city"))
            self.assertEqual(self._temporary_entries(destination), [])

    def test_directory_replacement_after_open_is_refused(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            real_open = ACTIVATION.os.open
            replaced = False

            def swap_after_open(path, flags, mode=0o777, *, dir_fd=None):
                nonlocal replaced
                fd = real_open(path, flags, mode, dir_fd=dir_fd)
                if path == destination.name and dir_fd is not None and not replaced:
                    replacement = destination.with_name("managed-replaced")
                    destination.rename(replacement)
                    destination.mkdir(mode=0o700)
                    replaced = True
                return fd

            with mock.patch.object(
                ACTIVATION.os,
                "open",
                side_effect=swap_after_open,
            ):
                with self.assertRaisesRegex(
                    ACTIVATION.BoundaryError,
                    "destination was replaced",
                ):
                    self._materialize(self.new, destination)
            self.assertTrue(replaced)
            self.assertEqual(list(destination.iterdir()), [])

    def test_changed_foreign_relative_wrong_basename_and_non_symlinks_are_refused(
        self,
    ) -> None:
        invalid_targets = (
            ("foreign", lambda target: os.symlink("/tmp/foreign/city", target)),
            ("relative", lambda target: os.symlink("../old/city", target)),
            (
                "missing-pseudo-store",
                lambda target: os.symlink(
                    "/nix/store/not-a-store-object/city",
                    target,
                ),
            ),
            (
                "wrong-basename",
                lambda target: os.symlink(str(self.old / "pack"), target),
            ),
            ("file", lambda target: target.write_text("durable\n", encoding="utf-8")),
            ("directory", lambda target: target.mkdir()),
        )
        for label, install in invalid_targets:
            with self.subTest(target=label), self._temporary_destination() as raw:
                destination = pathlib.Path(raw) / "managed"
                self._materialize(self.old, destination)
                target = destination / "city"
                target.unlink()
                install(target)
                before = os.lstat(target)
                before_link = os.readlink(target) if target.is_symlink() else None
                with self.assertRaises(ACTIVATION.BoundaryError):
                    self._materialize(self.new, destination)
                after = os.lstat(target)
                self.assertEqual(before.st_ino, after.st_ino)
                self.assertEqual(before.st_mode, after.st_mode)
                if before_link is not None:
                    self.assertEqual(os.readlink(target), before_link)
                self.assertEqual(self._temporary_entries(destination), [])

    def test_same_generation_retry_fsyncs_after_injected_failure(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            real_fsync = ACTIVATION.os.fsync
            calls = 0

            def fail_once(fd: int):
                nonlocal calls
                calls += 1
                if calls == 4:
                    raise OSError("injected managed-directory fsync failure")
                return real_fsync(fd)

            with mock.patch.object(
                ACTIVATION.os,
                "fsync",
                side_effect=fail_once,
            ):
                with self.assertRaises(ACTIVATION.BoundaryError):
                    self._materialize(self.old, destination)
            self.assertEqual(calls, 4)
            self.assertEqual(
                self._links(destination),
                {
                    name: str(self.old / name)
                    for name in ACTIVATION.MANAGED_ASSET_NAMES
                },
            )

            with mock.patch.object(
                ACTIVATION.os,
                "fsync",
                wraps=real_fsync,
            ) as fsync:
                self._materialize(self.old, destination)
            fsync.assert_called_once()

    def test_partial_rotation_fsyncs_successful_changes_and_preserves_failure(
        self,
    ) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            real_replace = ACTIVATION.os.replace
            real_fsync = ACTIVATION.os.fsync
            replace_calls = 0
            fsync_calls: list[int] = []

            def replace_with_failure(source, target, *args, **kwargs):
                nonlocal replace_calls
                replace_calls += 1
                if replace_calls == 2:
                    raise OSError("injected second managed-link replace failure")
                return real_replace(source, target, *args, **kwargs)

            def record_fsync(fd: int):
                fsync_calls.append(fd)
                return real_fsync(fd)

            with mock.patch.object(
                ACTIVATION.os,
                "replace",
                side_effect=replace_with_failure,
            ), mock.patch.object(
                ACTIVATION.os,
                "fsync",
                side_effect=record_fsync,
            ):
                with self.assertRaises(ACTIVATION.BoundaryError) as failure:
                    self._materialize(self.new, destination)

            self.assertEqual(replace_calls, 2)
            self.assertEqual(len(fsync_calls), 2)
            self.assertIsInstance(failure.exception.__cause__, OSError)
            self.assertIn(
                "injected second managed-link replace failure",
                str(failure.exception.__cause__),
            )
            self.assertEqual(
                os.readlink(destination / "city"),
                str(self.new / "city"),
            )
            self.assertEqual(
                os.readlink(destination / "pack"),
                str(self.old / "pack"),
            )
            self.assertEqual(self._temporary_entries(destination), [])

    def test_partial_rotation_sync_failure_is_retried_before_validation(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            real_replace = ACTIVATION.os.replace
            real_fsync = ACTIVATION.os.fsync
            replace_calls = 0
            fsync_calls = 0

            def replace_with_failure(source, target, *args, **kwargs):
                nonlocal replace_calls
                replace_calls += 1
                if replace_calls == 2:
                    raise OSError("injected partial-rotation failure")
                return real_replace(source, target, *args, **kwargs)

            def fsync_with_failure(fd: int):
                nonlocal fsync_calls
                fsync_calls += 1
                if fsync_calls == 2:
                    raise OSError("injected post-rotation fsync failure")
                return real_fsync(fd)

            with mock.patch.object(
                ACTIVATION.os,
                "replace",
                side_effect=replace_with_failure,
            ), mock.patch.object(
                ACTIVATION.os,
                "fsync",
                side_effect=fsync_with_failure,
            ):
                with self.assertRaises(ACTIVATION.BoundaryError) as failure:
                    self._materialize(self.new, destination)

            self.assertEqual(replace_calls, 2)
            self.assertEqual(fsync_calls, 2)
            self.assertIn("injected partial-rotation failure", str(failure.exception))
            self.assertIn("injected post-rotation fsync failure", str(failure.exception))
            mutation_error = failure.exception.mutation_error
            durability_error = failure.exception.durability_error
            self.assertIsInstance(mutation_error, ACTIVATION.BoundaryError)
            self.assertIsInstance(durability_error, ACTIVATION.BoundaryError)
            self.assertIs(failure.exception.__cause__, mutation_error)
            self.assertIsInstance(mutation_error.__cause__, OSError)
            self.assertIsInstance(durability_error.__cause__, OSError)
            self.assertEqual(
                os.readlink(destination / "city"),
                str(self.new / "city"),
            )

            invalid_target = destination / "pack"
            invalid_target.unlink()
            invalid_target.write_text("durable\n", encoding="utf-8")
            repair_fsyncs: list[int] = []

            def record_repair_fsync(fd: int):
                repair_fsyncs.append(fd)
                return real_fsync(fd)

            with mock.patch.object(
                ACTIVATION.os,
                "replace",
                side_effect=AssertionError("retry rotated before validation"),
            ), mock.patch.object(
                ACTIVATION.os,
                "fsync",
                side_effect=record_repair_fsync,
            ):
                with self.assertRaisesRegex(
                    ACTIVATION.BoundaryError,
                    "durable managed asset would be replaced",
                ):
                    self._materialize(self.new, destination)

            self.assertEqual(len(repair_fsyncs), 1)
            self.assertTrue(invalid_target.is_file())
            self.assertEqual(
                os.readlink(destination / "city"),
                str(self.new / "city"),
            )

    def test_replace_failure_cleans_private_temporary_link(self) -> None:
        with self._temporary_destination() as raw:
            destination = pathlib.Path(raw) / "managed"
            self._materialize(self.old, destination)
            with mock.patch.object(
                ACTIVATION.os,
                "replace",
                side_effect=OSError("injected managed-link replace failure"),
            ) as replace:
                with self.assertRaises(ACTIVATION.BoundaryError):
                    self._materialize(self.new, destination)
            replace.assert_called_once()
            self.assertEqual(
                self._links(destination),
                {
                    name: str(self.old / name)
                    for name in ACTIVATION.MANAGED_ASSET_NAMES
                },
            )
            self.assertEqual(self._temporary_entries(destination), [])


class ReserveReadinessContractTests(unittest.TestCase):
    def test_reserve_breach_blocks_before_zero_and_stays_submission_gated(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-reserve-") as raw:
            status_path = pathlib.Path(raw) / "status.json"
            ACTIVATION.write_status(
                status_path,
                ACTIVATION._ready_status("g1", "1", review_profile="review-sol"),
            )
            with mock.patch.object(
                ACTIVATION,
                "check_free_space",
                side_effect=ACTIVATION.BoundaryError("free-space-reserve"),
            ):
                with self.assertRaises(ACTIVATION.BoundaryError) as failure:
                    ACTIVATION.monitor_free_space_once(
                        path=raw,
                        reserve_bytes=1,
                        status_path=status_path,
                        generation="g1",
                        state_schema="1",
                    )
            self.assertEqual(str(failure.exception), "free-space-reserve")
            blocked = ACTIVATION.read_status(
                status_path,
                generation="g1",
                state_schema="1",
            )
            self.assertEqual(blocked["error_code"], "free-space-reserve")
            with mock.patch.object(OPERATOR, "READINESS_PATH", status_path):
                with self.assertRaises(OPERATOR.OperatorError):
                    OPERATOR.require_submission_readiness()


class HostProjectionContractTests(unittest.TestCase):
    @staticmethod
    def _fake_lstat(
        final_path: pathlib.Path,
        *,
        final_mode: int,
        final_uid: int = 0,
        ancestor_modes: dict[pathlib.Path, int] | None = None,
    ):
        ancestor_modes = ancestor_modes or {}

        def fake_lstat(candidate: pathlib.Path) -> SimpleNamespace:
            if candidate == final_path:
                return SimpleNamespace(st_mode=final_mode, st_uid=final_uid)
            return SimpleNamespace(
                st_mode=ancestor_modes.get(candidate, stat.S_IFDIR | 0o755),
                st_uid=0,
            )

        return fake_lstat

    def test_selected_root_owned_regular_file_below_etc_nixos_is_allowed(self) -> None:
        path = pathlib.Path("/etc/nixos/fixture-configuration.nix")
        with mock.patch.object(
            ACTIVATION.os,
            "lstat",
            side_effect=self._fake_lstat(
                path,
                final_mode=stat.S_IFREG | 0o644,
            ),
        ):
            self.assertEqual(ACTIVATION.validate_host_projection(str(path)), path)

    def test_etc_nixos_projection_rejects_root_directory_and_unsafe_descendants(self) -> None:
        cases = (
            (
                pathlib.Path("/etc/nixos"),
                None,
                {},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-directory"),
                stat.S_IFDIR | 0o755,
                {},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-link"),
                stat.S_IFLNK | 0o777,
                {},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-device"),
                stat.S_IFIFO | 0o600,
                {},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-user-owned.nix"),
                stat.S_IFREG | 0o600,
                {},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-writable.nix"),
                stat.S_IFREG | 0o666,
                {},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-symlinked/configuration.nix"),
                stat.S_IFREG | 0o644,
                {pathlib.Path("/etc/nixos/fixture-symlinked"): stat.S_IFLNK | 0o777},
            ),
            (
                pathlib.Path("/etc/nixos/fixture-writable/configuration.nix"),
                stat.S_IFREG | 0o644,
                {pathlib.Path("/etc/nixos/fixture-writable"): stat.S_IFDIR | 0o777},
            ),
            (
                pathlib.Path("/etc/ssh/fixture-key"),
                None,
                {},
            ),
            (
                pathlib.Path("/etc/shadow"),
                None,
                {},
            ),
        )
        for path, final_mode, ancestor_modes in cases:
            with self.subTest(path=path):
                if final_mode is None:
                    with self.assertRaises(ACTIVATION.BoundaryError):
                        ACTIVATION.validate_host_projection(str(path))
                    continue
                with mock.patch.object(
                    ACTIVATION.os,
                    "lstat",
                    side_effect=self._fake_lstat(
                        path,
                        final_mode=final_mode,
                        final_uid=1000 if "user-owned" in path.name else 0,
                        ancestor_modes=ancestor_modes,
                    ),
                ):
                    with self.assertRaises(ACTIVATION.BoundaryError):
                        ACTIVATION.validate_host_projection(str(path))


class AgentRelayContractTests(unittest.TestCase):
    @staticmethod
    def _private_listener(root: pathlib.Path) -> tuple[socket.socket, pathlib.Path]:
        path = root / "private.sock"
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(str(path))
        listener.listen(1)
        listener.settimeout(2)
        return listener, path

    @staticmethod
    def _fd_count() -> int:
        return len(list(pathlib.Path("/proc/self/fd").iterdir()))

    @staticmethod
    def _received_fds(ancillary: list[tuple[int, int, bytes]]) -> list[int]:
        descriptors: list[int] = []
        item_size = array.array("i").itemsize
        for level, kind, data in ancillary:
            if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                if len(data) % item_size:
                    raise AssertionError("received malformed SCM_RIGHTS data")
                values = array.array("i")
                values.frombytes(data)
                descriptors.extend(int(value) for value in values)
        return descriptors

    def test_peer_uid_uses_uid_field_not_process_id(self) -> None:
        left, right = socket.socketpair()
        try:
            raw = left.getsockopt(
                socket.SOL_SOCKET,
                socket.SO_PEERCRED,
                struct.calcsize("3i"),
            )
            peer_pid, peer_uid, _peer_gid = struct.unpack("3i", raw)
            self.assertNotEqual(peer_pid, peer_uid)
            self.assertEqual(ACTIVATION._peer_uid(left), peer_uid)
            self.assertEqual(ACTIVATION._peer_uid(left), os.geteuid())
        finally:
            left.close()
            right.close()

    def test_peer_uid_rejects_truncated_or_malformed_credentials(self) -> None:
        connection = mock.Mock()
        for value in (b"\x00" * 8, b"malformed-data"):
            with self.subTest(value=value):
                connection.getsockopt.return_value = value
                with self.assertRaises(ACTIVATION.BoundaryError):
                    ACTIVATION._peer_uid(connection)

    def test_authorized_relay_forwards_all_launcher_fds_once(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-agent-relay-") as raw:
            root = pathlib.Path(raw)
            listener, private_path = self._private_listener(root)
            relay_peer, public_client = socket.socketpair()
            upstream: socket.socket | None = None
            passed: list[socket.socket] = []
            passed_peers: list[socket.socket] = []
            received: list[int] = []
            thread = threading.Thread(
                target=ACTIVATION._relay_agent_connection,
                args=(relay_peer,),
                kwargs={
                    "private_socket": str(private_path),
                    "allowed_uid": os.geteuid(),
                },
                daemon=True,
            )
            try:
                thread.start()
                upstream, _ = listener.accept()
                upstream.settimeout(2)
                for _ in range(4):
                    sender, peer = socket.socketpair()
                    passed.append(sender)
                    passed_peers.append(peer)
                metadata = (
                    b'{"protocol":"gascity-agent/1","operation":"launch",'
                    b'"fds":["proxy","progress","control","check"]}\n'
                )
                sent = public_client.sendmsg(
                    [metadata],
                    [
                        (
                            socket.SOL_SOCKET,
                            socket.SCM_RIGHTS,
                            array.array("i", [sock.fileno() for sock in passed]).tobytes(),
                        )
                    ],
                )
                self.assertEqual(sent, len(metadata))
                for sock in passed:
                    sock.close()
                for sock in passed_peers:
                    sock.close()
                passed.clear()
                passed_peers.clear()

                data, ancillary, flags, _address = upstream.recvmsg(
                    4096,
                    socket.CMSG_SPACE(array.array("i").itemsize * 4),
                    getattr(socket, "MSG_CMSG_CLOEXEC", 0),
                )
                self.assertFalse(flags & getattr(socket, "MSG_CTRUNC", 0))
                self.assertEqual(data, metadata)
                received = self._received_fds(ancillary)
                self.assertEqual(len(received), 4)
                for descriptor in received:
                    self.assertTrue(
                        fcntl.fcntl(descriptor, fcntl.F_GETFD) & fcntl.FD_CLOEXEC
                    )

                acknowledgement = b'{"protocol":"gascity-agent/1","ok":true}\n'
                upstream.sendall(acknowledgement)
                self.assertEqual(public_client.recv(len(acknowledgement)), acknowledgement)

                public_client.sendall(b'{"jsonrpc":"2.0","method":"session/new"}\n')
                data, ancillary, _flags, _address = upstream.recvmsg(
                    4096,
                    socket.CMSG_SPACE(array.array("i").itemsize * 4),
                )
                self.assertEqual(data, b'{"jsonrpc":"2.0","method":"session/new"}\n')
                self.assertEqual(self._received_fds(ancillary), [])
            finally:
                for descriptor in received:
                    try:
                        os.close(descriptor)
                    except OSError:
                        pass
                for sock in passed:
                    sock.close()
                for sock in passed_peers:
                    sock.close()
                public_client.close()
                if upstream is not None:
                    upstream.close()
                listener.close()
                thread.join(timeout=2)
                self.assertFalse(thread.is_alive())

    def test_unauthorized_relay_peer_is_closed_before_private_connect(self) -> None:
        relay_peer, public_client = socket.socketpair()
        thread = threading.Thread(
            target=ACTIVATION._relay_agent_connection,
            args=(relay_peer,),
            kwargs={
                "private_socket": "/run/gascity-nonexistent.sock",
                "allowed_uid": os.geteuid() + 1,
            },
            daemon=True,
        )
        try:
            thread.start()
            self.assertEqual(public_client.recv(1), b"")
        finally:
            public_client.close()
            thread.join(timeout=2)
            self.assertFalse(thread.is_alive())

    def test_malformed_ancillary_data_is_rejected_and_descriptors_are_closed(self) -> None:
        with self.assertRaises(ACTIVATION.BoundaryError):
            ACTIVATION._extract_relay_descriptors(
                [
                    (
                        socket.SOL_SOCKET,
                        socket.SCM_RIGHTS,
                        b"\x00",
                    )
                ]
            )

        with tempfile.TemporaryDirectory(prefix="gascity-agent-relay-truncated-") as raw:
            root = pathlib.Path(raw)
            listener, private_path = self._private_listener(root)
            relay_peer, public_client = socket.socketpair()
            upstream: socket.socket | None = None
            passed: list[socket.socket] = []
            passed_peers: list[socket.socket] = []
            for _ in range(5):
                sender, peer = socket.socketpair()
                passed.append(sender)
                passed_peers.append(peer)
            before = self._fd_count()
            thread = threading.Thread(
                target=ACTIVATION._relay_agent_connection,
                args=(relay_peer,),
                kwargs={
                    "private_socket": str(private_path),
                    "allowed_uid": os.geteuid(),
                },
                daemon=True,
            )
            try:
                thread.start()
                upstream, _ = listener.accept()
                public_client.sendmsg(
                    [b'{"fds":[]}\n'],
                    [
                        (
                            socket.SOL_SOCKET,
                            socket.SCM_RIGHTS,
                            array.array("i", [sock.fileno() for sock in passed]).tobytes(),
                        )
                    ],
                )
                for sock in passed:
                    sock.close()
                for sock in passed_peers:
                    sock.close()
                passed.clear()
                passed_peers.clear()
                self.assertEqual(public_client.recv(1), b"")
            finally:
                for sock in passed:
                    sock.close()
                for sock in passed_peers:
                    sock.close()
                public_client.close()
                if upstream is not None:
                    upstream.close()
                listener.close()
                thread.join(timeout=2)
                self.assertFalse(thread.is_alive())
            self.assertLessEqual(self._fd_count(), before)


class SandboxContractTests(unittest.TestCase):
    def test_namespace_argv_has_no_direct_egress_or_host_state(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-sandbox-") as raw:
            root = pathlib.Path(raw)
            worktree = root / "worktree"
            worktree.mkdir()
            state = root / "state"
            state.mkdir()
            home = root / "home"
            home.mkdir()
            argv, inherited = SANDBOX.build_sandbox_argv(
                [sys.executable, "-c", "pass"],
                worktree=worktree,
                state_root=state,
                copilot_home=home,
                runtime_paths=[sys.executable],
                approved_wrappers=[SCRIPT_ROOT / "fdproxy.py"],
                environment={
                    "PATH": "/runtime/bin",
                    "COPILOT_GITHUB_TOKEN": "not-in-argv",
                    "GC_FDPROXY_AUTH": "sidecar-secret",
                },
                proxy_fd=9,
                progress_fd=10,
                fdproxy_path=SCRIPT_ROOT / "fdproxy.py",
                bwrap_path=sys.executable,
            )
            self.assertIn("--unshare-user", argv)
            self.assertIn("--unshare-pid", argv)
            self.assertIn("--unshare-net", argv)
            self.assertNotIn("--share-net", argv)
            self.assertIn("--tmpfs", argv)
            self.assertNotIn(str(state), argv)
            self.assertEqual(inherited, (9, 10))
            self.assertNotIn("not-in-argv", argv)
            self.assertNotIn("sidecar-secret", argv)
            self.assertIn("127.0.0.1:3128", argv)

    def test_role_specific_mounts_allow_planning_artifacts_only(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-sandbox-policy-") as raw:
            root = pathlib.Path(raw)
            worktree = root / "worktree"
            planning_root = worktree / "docs/plans"
            source_root = worktree / "src"
            planning_root.mkdir(parents=True)
            source_root.mkdir()
            (root / "home").mkdir()
            (root / "home-review").mkdir()
            (root / "home-coding").mkdir()

            planning_argv, _ = SANDBOX.build_sandbox_argv(
                [sys.executable, "-c", "pass"],
                worktree=worktree,
                copilot_home=root / "home",
                runtime_paths=[sys.executable],
                approved_wrappers=[],
                tool_policy="planning",
                bwrap_path=sys.executable,
            )
            self.assertIn(
                ["--ro-bind", str(worktree), "/workspace"],
                [
                    planning_argv[index : index + 3]
                    for index in range(len(planning_argv) - 2)
                ],
            )
            self.assertIn(
                ["--bind", str(planning_root), "/workspace/docs/plans"],
                [
                    planning_argv[index : index + 3]
                    for index in range(len(planning_argv) - 2)
                ],
            )
            self.assertNotIn(
                ["--bind", str(worktree), "/workspace"],
                [
                    planning_argv[index : index + 3]
                    for index in range(len(planning_argv) - 2)
                ],
            )

            review_argv, _ = SANDBOX.build_sandbox_argv(
                [sys.executable, "-c", "pass"],
                worktree=worktree,
                copilot_home=root / "home-review",
                runtime_paths=[sys.executable],
                approved_wrappers=[],
                tool_policy="review",
                bwrap_path=sys.executable,
            )
            self.assertIn(
                ["--ro-bind", str(worktree), "/workspace"],
                [
                    review_argv[index : index + 3]
                    for index in range(len(review_argv) - 2)
                ],
            )
            self.assertNotIn(
                ["--bind", str(worktree), "/workspace"],
                [
                    review_argv[index : index + 3]
                    for index in range(len(review_argv) - 2)
                ],
            )

            coding_argv, _ = SANDBOX.build_sandbox_argv(
                [sys.executable, "-c", "pass"],
                worktree=worktree,
                copilot_home=root / "home-coding",
                runtime_paths=[sys.executable],
                approved_wrappers=[],
                tool_policy="coding",
                bwrap_path=sys.executable,
            )
            self.assertIn(
                ["--bind", str(worktree), "/workspace"],
                [
                    coding_argv[index : index + 3]
                    for index in range(len(coding_argv) - 2)
                ],
            )

    def test_planning_mount_boundary_allows_artifact_and_denies_source_write(self) -> None:
        bwrap = shutil.which("bwrap")
        if bwrap is None:
            self.skipTest("bubblewrap is unavailable outside the Gas City package")
        bash = pathlib.Path("/bin/bash").resolve()
        nix_store = pathlib.Path("/nix/store")
        if not bash.exists() or not nix_store.is_dir():
            self.skipTest("the host does not expose the Nix shell runtime")
        with tempfile.TemporaryDirectory(prefix="gascity-sandbox-boundary-") as raw:
            root = pathlib.Path(raw)
            worktree = root / "worktree"
            planning_root = worktree / "docs/plans"
            source_root = worktree / "src"
            planning_root.mkdir(parents=True)
            source_root.mkdir()
            home = root / "home"
            home.mkdir()
            command = [
                str(bash),
                "-c",
                (
                    "set -eu; "
                    "printf planning > /workspace/docs/plans/positive.md; "
                    "printf source > /workspace/src/planted.md"
                ),
            ]
            argv, _ = SANDBOX.build_sandbox_argv(
                command,
                worktree=worktree,
                copilot_home=home,
                runtime_paths=[bash, nix_store],
                approved_wrappers=[],
                tool_policy="planning",
                bwrap_path=bwrap,
            )
            result = subprocess.run(
                argv,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(
                (planning_root / "positive.md").read_text(encoding="utf-8"),
                "planning",
            )
            self.assertFalse((source_root / "planted.md").exists())


class DurableStateContractTests(unittest.TestCase):
    def _context(self, generation: str = "g1") -> dict[str, object]:
        return {
            "run_id": "run-1",
            "bead_id": "bead-1",
            "generation": generation,
            "state_schema": "1",
            "open_work": "implement the change",
            "summary": "nothing committed yet",
            "branch": "gc/run-1",
            "commits": [],
            "worktree": "/worktree",
            "review_state": "not-reviewed",
            "retry_counters": {"session": 0},
            "next_action": "start implementation",
        }

    def test_reconstruction_is_fresh_and_generation_bound(self) -> None:
        prompt = ACTIVATION.reconstruct_prompt(
            self._context(),
            generation="g1",
            state_schema="1",
        )
        self.assertIn("fresh ACP conversation", prompt)
        self.assertIn("start implementation", prompt)
        with self.assertRaises(ACTIVATION.StaleGeneration):
            ACTIVATION.reconstruct_prompt(
                self._context("old-generation"),
                generation="g1",
                state_schema="1",
            )

    def test_retry_counter_is_bead_owned_and_generation_bound(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-context-") as raw:
            context_path = pathlib.Path(raw) / "bead.json"
            context_path.write_text(json.dumps(self._context()), encoding="utf-8")
            updated = ACTIVATION.increment_retry_counter(
                context_path,
                counter="session",
                generation="g1",
                state_schema="1",
            )
            self.assertEqual(updated["retry_counters"]["session"], 1)
            with self.assertRaises(ACTIVATION.StaleGeneration):
                ACTIVATION.increment_retry_counter(
                    context_path,
                    counter="session",
                    generation="g2",
                    state_schema="1",
                )

    def _gc_targets(self, raw: str) -> dict[str, pathlib.Path]:
        targets = {}
        for name in ACTIVATION.GC_ROOT_NAMES:
            target = pathlib.Path(raw) / name
            target.mkdir()
            targets[name] = target
        return targets

    def _closed_bead(self, bead_id: str = "bead-1") -> mock._patch:
        return mock.patch.object(
            ACTIVATION.subprocess,
            "run",
            return_value=subprocess.CompletedProcess(
                ["bd", "show", bead_id, "--json"],
                0,
                stdout=json.dumps(
                    [{"id": bead_id, "status": "closed", "metadata": {}}]
                ),
                stderr="",
            ),
        )

    def test_terminal_writer_then_cleanup_is_atomic_and_idempotent(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-writer-") as raw:
            root = pathlib.Path(raw) / "roots"
            terminal_root = pathlib.Path(raw) / "terminal"
            targets = self._gc_targets(raw)
            roots = ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                allowed_prefixes=(f"{raw}/",),
                generation="g1",
                state_schema="1",
            )
            with self._closed_bead():
                first = ACTIVATION.record_terminal_state(
                    terminal_root,
                    run_id="run-1",
                    bead_id="bead-1",
                    generation="g1",
                    state_schema="1",
                    bd_path="bd",
                )
                second = ACTIVATION.record_terminal_state(
                    terminal_root,
                    run_id="run-1",
                    bead_id="bead-1",
                    generation="g1",
                    state_schema="1",
                    bd_path="bd",
                )
            self.assertTrue(first["recorded"])
            self.assertTrue(second["recorded"])
            state_path = terminal_root / "run-1.json"
            self.assertEqual(
                set(json.loads(state_path.read_text(encoding="utf-8"))),
                {
                    "schema",
                    "run_id",
                    "bead_id",
                    "generation",
                    "state_schema",
                    "terminal_status",
                },
            )
            roots.cleanup(state_path=state_path)
            roots.cleanup(state_path=state_path)
            self.assertFalse((root / "run-1").exists())
            with self._closed_bead():
                restarted = ACTIVATION.record_terminal_state(
                    terminal_root,
                    run_id="run-1",
                    bead_id="bead-1",
                    generation="g1",
                    state_schema="1",
                    bd_path="bd",
                )
            self.assertTrue(restarted["recorded"])

    def test_terminal_writer_retains_cancelled_open_pr_and_nonterminal_runs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-retain-") as raw:
            root = pathlib.Path(raw) / "roots"
            terminal_root = pathlib.Path(raw) / "terminal"
            cancellation_root = pathlib.Path(raw) / "cancellations"
            targets = self._gc_targets(raw)
            for run_id, bead in (
                ("cancelled", {"id": "bead-1", "status": "closed", "metadata": {}}),
                (
                    "open-pr",
                    {
                        "id": "bead-2",
                        "status": "closed",
                        "metadata": {"pull_request_url": "https://example.invalid/pr/1"},
                    },
                ),
                ("open", {"id": "bead-3", "status": "in_progress", "metadata": {}}),
            ):
                ACTIVATION.ActiveRunGCRoots.create(
                    root,
                    run_id=run_id,
                    bead_id=bead["id"],
                    generation_paths=targets,
                    allowed_prefixes=(f"{raw}/",),
                    generation="g1",
                    state_schema="1",
                )
                if run_id == "cancelled":
                    cancellation_root.mkdir()
                    (cancellation_root / f"{run_id}.json").write_text(
                        json.dumps(
                            {
                                "schema": 1,
                                "run_id": run_id,
                                "reason": "operator requested cancellation",
                                "cancelled": True,
                            }
                        ),
                        encoding="utf-8",
                    )
                with mock.patch.dict(
                    os.environ,
                    {
                        "GC_PUBLISH_ENABLED": "1" if run_id == "open-pr" else "0",
                        "GC_PUBLISH_OPEN_PR": "1" if run_id == "open-pr" else "0",
                    },
                    clear=False,
                ), mock.patch.object(
                    ACTIVATION.subprocess,
                    "run",
                    return_value=subprocess.CompletedProcess(
                        ["bd", "show", bead["id"], "--json"],
                        0,
                        stdout=json.dumps([bead]),
                        stderr="",
                    ),
                ):
                    result = ACTIVATION.record_terminal_state(
                        terminal_root,
                        run_id=run_id,
                        bead_id=bead["id"],
                        generation="g1",
                        state_schema="1",
                        bd_path="bd",
                        cancellation_root=cancellation_root,
                    )
                self.assertFalse(result["recorded"])
                self.assertTrue(result["retained"])
                self.assertFalse((terminal_root / f"{run_id}.json").exists())
                self.assertTrue((root / run_id).is_dir())

    def test_terminal_writer_rejects_stale_or_forged_record(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-stale-") as raw:
            terminal_root = pathlib.Path(raw) / "terminal"
            terminal_root.mkdir()
            state_path = terminal_root / "run-1.json"
            state_path.write_text(
                json.dumps(
                    {
                        "schema": ACTIVATION.TERMINAL_RECORD_SCHEMA,
                        "run_id": "run-1",
                        "bead_id": "bead-1",
                        "generation": "old-generation",
                        "state_schema": "1",
                        "terminal_status": "closed",
                    }
                ),
                encoding="utf-8",
            )
            with self._closed_bead(), self.assertRaises(
                ACTIVATION.RootLifecycleError
            ):
                ACTIVATION.record_terminal_state(
                    terminal_root,
                    run_id="run-1",
                    bead_id="bead-1",
                    generation="g1",
                    state_schema="1",
                    bd_path="bd",
                )
            self.assertIn("old-generation", state_path.read_text(encoding="utf-8"))

    def test_gc_root_links_are_valid_store_paths(self) -> None:
        nix_store = shutil.which("nix-store")
        if nix_store is None:
            self.skipTest("a valid Nix store path is unavailable")
        store_target = pathlib.Path("/run/current-system").resolve()
        if not str(store_target).startswith("/nix/store/"):
            store_target = pathlib.Path(nix_store).resolve()
        if not str(store_target).startswith("/nix/store/"):
            self.skipTest("a valid Nix store path is unavailable")
        if not store_target.exists():
            self.skipTest("the current system store path is unavailable")
        probe = subprocess.run(
            [nix_store, "--query", "--deriver", str(store_target)],
            check=False,
            capture_output=True,
            text=True,
        )
        if probe.returncode != 0:
            self.skipTest(
                "Nix store query is unavailable in this environment: "
                + probe.stderr.strip()
            )
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-store-") as raw:
            root = pathlib.Path(raw) / "roots"
            targets = {
                name: store_target for name in ACTIVATION.GC_ROOT_NAMES
            }
            ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                generation="g1",
                state_schema="1",
            )
            for name in ACTIVATION.GC_ROOT_NAMES:
                link = root / "run-1" / name
                self.assertEqual(os.readlink(link), str(store_target))
                result = subprocess.run(
                    [nix_store, "--query", "--deriver", os.readlink(link)],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
            registrations = []
            try:
                registration_root = root / "registered"
                registration_root.mkdir()
                for name in ACTIVATION.GC_ROOT_NAMES:
                    registration = registration_root / name
                    result = subprocess.run(
                        [
                            nix_store,
                            "--add-root",
                            str(registration),
                            "--indirect",
                            "--realise",
                            str(root / "run-1" / name),
                        ],
                        check=False,
                        capture_output=True,
                        text=True,
                    )
                    if result.returncode != 0:
                        self.skipTest(
                            "Nix GC-root registration is unavailable: "
                            + result.stderr.strip()
                        )
                    registrations.append(registration)
                roots = subprocess.run(
                    [nix_store, "--gc", "--print-roots"],
                    check=False,
                    capture_output=True,
                    text=True,
                )
                if roots.returncode != 0:
                    self.skipTest(
                        "Nix GC-root query is unavailable: "
                        + roots.stderr.strip()
                    )
                for registration in registrations:
                    self.assertTrue(
                        any(
                            candidate in roots.stdout
                            for candidate in (
                                f"{registration} -> {store_target}",
                                f'"{registration}" -> {store_target}',
                            )
                        ),
                        roots.stdout,
                    )
            finally:
                for registration in registrations:
                    registration.unlink(missing_ok=True)
                subprocess.run(
                    [nix_store, "--gc", "--print-roots"],
                    check=False,
                    capture_output=True,
                    text=True,
                )

    def test_gc_root_links_are_visible_to_nix_gc_root_query(self) -> None:
        nix_store = shutil.which("nix-store")
        if nix_store is None:
            self.skipTest("nix-store is unavailable")
        store_target = pathlib.Path("/run/current-system").resolve()
        if (
            not str(store_target).startswith("/nix/store/")
            or not store_target.exists()
        ):
            self.skipTest("a valid Nix store path is unavailable")
        root_parent = pathlib.Path("/nix/var/nix/gcroots")
        if not root_parent.is_dir() or not os.access(root_parent, os.W_OK):
            self.skipTest("the production Nix GC-root hierarchy is not writable")
        root = root_parent / f".gascity-contract-{os.getpid()}"
        terminal_root = root_parent / f".gascity-terminal-{os.getpid()}"
        try:
            root.mkdir(mode=0o700)
            targets = {
                name: store_target for name in ACTIVATION.GC_ROOT_NAMES
            }
            ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                generation="g1",
                state_schema="1",
                terminal_state_path=terminal_root / "run-1.json",
            )
            roots = subprocess.run(
                [nix_store, "--gc", "--print-roots"],
                check=False,
                capture_output=True,
                text=True,
            )
            if roots.returncode != 0:
                self.skipTest(
                    "Nix GC-root query is unavailable: " + roots.stderr.strip()
                )
            for name in ACTIVATION.GC_ROOT_NAMES:
                link = root / "run-1" / name
                self.assertTrue(
                    any(
                        candidate in roots.stdout
                        for candidate in (
                            f"{link} -> {store_target}",
                            f'"{link}" -> {store_target}',
                        )
                    ),
                    roots.stdout,
                )
        finally:
            shutil.rmtree(root, ignore_errors=True)
            shutil.rmtree(terminal_root, ignore_errors=True)

    def test_gc_root_lifecycle_requires_durable_terminal_state(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-") as raw:
            root = pathlib.Path(raw) / "roots"
            targets = self._gc_targets(raw)
            roots = ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                allowed_prefixes=(f"{raw}/",),
                generation="g1",
                state_schema="1",
            )
            with self.assertRaises(ACTIVATION.RootLifecycleError):
                roots.cleanup(terminal=False)
            state_path = pathlib.Path(raw) / "terminal" / "run-1.json"
            state_path.parent.mkdir(mode=0o700)
            state_path.write_text(
                json.dumps(
                    {
                        "schema": ACTIVATION.TERMINAL_RECORD_SCHEMA,
                        "run_id": "run-1",
                        "bead_id": "bead-1",
                        "generation": "g1",
                        "state_schema": "1",
                        "terminal_status": "open",
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaises(ACTIVATION.RootLifecycleError):
                roots.cleanup(state_path=state_path)
            state_path.write_text(
                json.dumps(
                    {
                        "schema": ACTIVATION.TERMINAL_RECORD_SCHEMA,
                        "run_id": "run-1",
                        "bead_id": "bead-1",
                        "generation": "g1",
                        "state_schema": "1",
                        "terminal_status": "completed",
                    }
                ),
                encoding="utf-8",
            )
            roots.cleanup(state_path=state_path)
            self.assertFalse((root / "run-1").exists())

    def test_gc_root_lifecycle_reuses_immutable_symlinks_across_restart(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-restart-") as raw:
            root = pathlib.Path(raw) / "roots"
            targets = self._gc_targets(raw)
            first = ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                allowed_prefixes=(f"{raw}/",),
                generation="g1",
                state_schema="1",
            )
            first_links = {
                name: os.readlink(root / "run-1" / name)
                for name in ACTIVATION.GC_ROOT_NAMES
            }
            restarted = ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                allowed_prefixes=(f"{raw}/",),
                generation="g1",
                state_schema="1",
            )
            self.assertFalse(restarted.terminal)
            self.assertEqual(
                first_links,
                {
                    name: os.readlink(root / "run-1" / name)
                    for name in ACTIVATION.GC_ROOT_NAMES
                },
            )

    def test_gc_root_lifecycle_rejects_incompatible_generation(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-upgrade-") as raw:
            root = pathlib.Path(raw) / "roots"
            targets = {}
            for name in ACTIVATION.GC_ROOT_NAMES:
                target = pathlib.Path(raw) / name
                target.mkdir()
                targets[name] = target
            ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                bead_id="bead-1",
                generation_paths=targets,
                allowed_prefixes=(f"{raw}/",),
                generation="g1",
                state_schema="1",
            )
            with self.assertRaises(ACTIVATION.RootLifecycleError):
                ACTIVATION.ActiveRunGCRoots.create(
                    root,
                    run_id="run-1",
                    bead_id="bead-1",
                    generation_paths=targets,
                    allowed_prefixes=(f"{raw}/",),
                    generation="g2",
                    state_schema="1",
                )

    def test_gc_root_lifecycle_retains_cancelled_and_open_pr_runs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-terminal-") as raw:
            root = pathlib.Path(raw) / "roots"
            targets = {}
            for name in ACTIVATION.GC_ROOT_NAMES:
                target = pathlib.Path(raw) / name
                target.mkdir()
                targets[name] = target
            for index, state in enumerate(
                (
                    {"workflow_state": "cancelled", "cancelled": True},
                    {"workflow_state": "completed", "open_pr": True},
                ),
                start=1,
            ):
                run_id = f"run-{index}"
                roots = ACTIVATION.ActiveRunGCRoots.create(
                    root,
                    run_id=run_id,
                    bead_id=f"bead-{index}",
                    generation_paths=targets,
                    allowed_prefixes=(f"{raw}/",),
                    generation="g1",
                    state_schema="1",
                )
                state_path = pathlib.Path(raw) / "terminal" / f"{run_id}.json"
                state_path.parent.mkdir(exist_ok=True)
                state_path.write_text(
                    json.dumps(
                        {
                            "schema": ACTIVATION.TERMINAL_RECORD_SCHEMA,
                            "run_id": run_id,
                            "bead_id": f"bead-{index}",
                            "generation": "g1",
                            "state_schema": "1",
                            "terminal_status": "closed",
                            **state,
                        }
                    ),
                    encoding="utf-8",
                )
                with self.assertRaises(ACTIVATION.RootLifecycleError):
                    roots.cleanup(state_path=state_path)
                self.assertTrue((root / run_id).is_dir())

    def test_progress_helper_rejects_decision_operations(self) -> None:
        environment = {
            "GC_RUN_ID": "run-1",
            "GC_BEAD_ID": "bead-1",
            "GC_CITY_GENERATION": "g1",
            "GC_STATE_SCHEMA": "1",
        }
        request = GC_AGENT.make_request("progress", environment=environment)
        self.assertEqual(request["operation"], "progress")
        request["operation"] = "decision"
        with self.assertRaises(GC_AGENT.AgentChannelError):
            GC_AGENT.validate_request(request, environment=environment)


class LauncherServerHarness:
    def __init__(
        self,
        root: pathlib.Path,
        *,
        extra: tuple[str, ...] = (),
        gc_root_args: tuple[str, ...] = (),
        max_agents: int = 2,
        max_active_runs: int = 2,
    ):
        self.root = root
        self.worktree = root / "worktree"
        self.worktree.mkdir()
        self.socket_path = root / "agent.sock"
        self.process: subprocess.Popen[bytes] | None = None
        self.extra = extra
        self.gc_root_args = gc_root_args
        self.max_agents = max_agents
        self.max_active_runs = max_active_runs

    def start(self) -> None:
        command = [
            sys.executable,
            str(SCRIPT_ROOT / "agent-launcher.py"),
            "--server",
            "--socket",
            str(self.socket_path),
            "--settings-root",
            str(COPILOT_ROOT),
            "--copilot",
            sys.executable,
            "--lease-root",
            str(self.root / "leases"),
            "--runtime-root",
            str(self.root / "runtime"),
            "--sandbox-script",
            str(SCRIPT_ROOT / "agent-sandbox.py"),
            "--fdproxy-script",
            str(SCRIPT_ROOT / "fdproxy.py"),
            "--max-agents",
            str(self.max_agents),
            "--max-active-runs",
            str(self.max_active_runs),
            "--term-grace",
            "0.1",
            "--kill-grace",
            "0.1",
            "--allow-unsafe-fixture",
            "--fixture-child-script",
            str(FAKE_ACP),
        ]
        for value in self.extra:
            command.append(f"--fixture-child-arg={value}")
        command.extend(self.gc_root_args)
        environment = dict(os.environ)
        environment["GC_TEST_MODE"] = "1"
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
        )
        deadline = time.monotonic() + 5
        while not self.socket_path.exists():
            if self.process.poll() is not None:
                stderr = (
                    self.process.stderr.read().decode("utf-8", errors="replace")
                    if self.process.stderr is not None
                    else ""
                )
                raise AssertionError(
                    f"launcher server exited before binding: {stderr}"
                )
            if time.monotonic() >= deadline:
                raise AssertionError("launcher server did not bind")
            time.sleep(0.01)

    def client_command(
        self,
        *,
        profile: str = "code-luna",
        run_id: str = "run-1",
        bead_id: str = "bead-1",
        control_fd: int | None = None,
        terminal_state_path: pathlib.Path | None = None,
    ) -> list[str]:
        command = [
            sys.executable,
            str(SCRIPT_ROOT / "copilot-profile.py"),
            "--profile",
            profile,
            "--tool-policy",
            "coding" if profile == "code-luna" else "review",
            "--launcher-socket",
            str(self.socket_path),
            "--run-id",
            run_id,
            "--bead-id",
            bead_id,
            "--generation",
            "g1",
            "--state-schema",
            "1",
            "--worktree",
            str(self.worktree),
        ]
        if control_fd is not None:
            command.extend(["--control-fd", str(control_fd)])
        if terminal_state_path is not None:
            command.extend(["--terminal-state-path", str(terminal_state_path)])
        return command

    def stop(self) -> None:
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            self.process.wait(timeout=5)
        if self.process.stdout is not None:
            self.process.stdout.close()
        if self.process.stderr is not None:
            self.process.stderr.close()


class LauncherLifecycleTests(unittest.TestCase):
    def _assert_lease_available(self, root: pathlib.Path, run_id: str) -> None:
        deadline = time.monotonic() + 2
        while True:
            try:
                lease = LAUNCHER.ConcurrencyLease.acquire(
                    root / "leases",
                    run_id=run_id,
                    max_agents=2,
                    max_active_runs=2,
                )
            except LAUNCHER.LeaseBusy:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.01)
            else:
                lease.release()
                return

    def _active_run_slots(self, root: pathlib.Path) -> dict[str, dict[str, object]]:
        registry = root / "leases" / "active-runs.json"
        if not registry.exists():
            return {}
        value = json.loads(registry.read_text(encoding="utf-8"))
        slots = value.get("slots")
        return slots if isinstance(slots, dict) else {}

    def test_real_client_lifecycle_uses_ndjson_and_releases_lease(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-launcher-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(root)
            server.start()
            try:
                process = subprocess.Popen(
                    server.client_command(),
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env=dict(os.environ),
                )
                diagnostic = (
                    b'{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}\n'
                    b'{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/host/worktree"}}\n'
                    b'{"jsonrpc":"2.0","id":3,"method":"session/prompt",'
                    b'"params":{"sessionId":"fake-session","prompt":[]}}\n'
                )
                self.assertIsNotNone(process.stdin)
                process.stdin.write(diagnostic)
                process.stdin.flush()
                self.assertIsNotNone(process.stdout)
                messages = [
                    json.loads(process.stdout.readline()),
                    json.loads(process.stdout.readline()),
                    json.loads(process.stdout.readline()),
                    json.loads(process.stdout.readline()),
                ]
                process.stdin.close()
                process.wait(timeout=5)
                self.assertEqual(process.returncode, 0)
                self.assertTrue(any(message.get("method") == "session/update" for message in messages))
                self.assertTrue(
                    any(
                        message.get("result", {}).get("models", {}).get(
                            "currentModelId"
                        )
                        == "gpt-5.6-luna"
                        for message in messages
                        if isinstance(message.get("result"), dict)
                    )
                )
                self._assert_lease_available(root, "run-1")
            finally:
                server.stop()

    def test_production_admission_creates_and_reuses_gc_roots_across_restart(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-launcher-gcroots-") as raw:
            root = pathlib.Path(raw)
            targets = {}
            for name in ACTIVATION.GC_ROOT_NAMES:
                target = root / "targets" / name
                target.mkdir(parents=True)
                targets[name] = target
            gc_root_directory = root / "gc-roots"
            gc_args = (
                "--generation",
                "g1",
                "--activation-script",
                str(SCRIPT_ROOT / "service-activation.py"),
                "--gc-root-directory",
                str(gc_root_directory),
                "--gc-root-prefix",
                f"{root}/",
                "--package-path",
                str(targets["package"]),
                "--city-path",
                str(targets["city"]),
                "--pack-path",
                str(targets["pack"]),
                "--profiles-path",
                str(targets["profiles"]),
                "--instructions-path",
                str(targets["instructions"]),
            )
            server = LauncherServerHarness(root, gc_root_args=gc_args)
            terminal_state_path = root / "terminal" / "run-1.json"
            terminal_state_path.parent.mkdir(mode=0o700)
            for _restart in range(2):
                server.start()
                try:
                    process = subprocess.Popen(
                        server.client_command(
                            terminal_state_path=terminal_state_path,
                        ),
                        stdin=subprocess.PIPE,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE,
                        env=dict(os.environ),
                    )
                    self.assertIsNotNone(process.stdin)
                    process.stdin.write(
                        b'{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}\n'
                    )
                    process.stdin.flush()
                    root_path = gc_root_directory / "run-1"
                    deadline = time.monotonic() + 5
                    while not root_path.exists() and time.monotonic() < deadline:
                        time.sleep(0.01)
                    if not root_path.exists():
                        process.stdin.close()
                        process.wait(timeout=5)
                        stderr = (
                            process.stderr.read().decode("utf-8", errors="replace")
                            if process.stderr is not None
                            else ""
                        )
                        self.fail(
                            f"active-run GC roots were not created "
                            f"(returncode={process.returncode}, stderr={stderr})"
                        )
                    self.assertTrue(root_path.is_dir())
                    for name, target in targets.items():
                        self.assertTrue((root_path / name).is_symlink())
                        self.assertEqual(os.readlink(root_path / name), str(target))
                    process.stdin.close()
                    process.wait(timeout=5)
                    self.assertEqual(process.returncode, 0)
                finally:
                    server.stop()
            self.assertTrue((gc_root_directory / "run-1").is_dir())

    def test_production_terminal_state_cleans_gc_roots(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-launcher-terminal-") as raw:
            root = pathlib.Path(raw)
            targets = {}
            for name in ACTIVATION.GC_ROOT_NAMES:
                target = root / "targets" / name
                target.mkdir(parents=True)
                targets[name] = target
            gc_root_directory = root / "gc-roots"
            gc_args = (
                "--generation",
                "g1",
                "--activation-script",
                str(SCRIPT_ROOT / "service-activation.py"),
                "--gc-root-directory",
                str(gc_root_directory),
                "--gc-root-prefix",
                f"{root}/",
                "--package-path",
                str(targets["package"]),
                "--city-path",
                str(targets["city"]),
                "--pack-path",
                str(targets["pack"]),
                "--profiles-path",
                str(targets["profiles"]),
                "--instructions-path",
                str(targets["instructions"]),
            )
            terminal_state_path = root / "terminal" / "run-1.json"
            terminal_state_path.parent.mkdir(mode=0o700)
            terminal_state_path.write_text(
                json.dumps(
                    {
                        "schema": ACTIVATION.TERMINAL_RECORD_SCHEMA,
                        "run_id": "run-1",
                        "bead_id": "bead-1",
                        "generation": "g1",
                        "state_schema": "1",
                        "terminal_status": "completed",
                    }
                ),
                encoding="utf-8",
            )
            server = LauncherServerHarness(root, gc_root_args=gc_args)
            server.start()
            try:
                process = subprocess.Popen(
                    server.client_command(
                        terminal_state_path=terminal_state_path,
                    ),
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env=dict(os.environ),
                )
                self.assertIsNotNone(process.stdin)
                process.stdin.close()
                process.wait(timeout=5)
                self.assertEqual(process.returncode, 0)
                deadline = time.monotonic() + 2
                root_path = gc_root_directory / "run-1"
                while root_path.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertFalse(root_path.exists())
            finally:
                server.stop()

    def test_client_probe_preserves_initialize_session_new_and_diagnostic_prompt(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-probe-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(
                root,
                extra=("--silent-metadata",),
            )
            server.start()
            try:
                args = PROFILE._default_namespace("code-luna", "coding")
                args.launcher_socket = str(server.socket_path)
                args.run_id = "run-1"
                args.bead_id = "bead-1"
                args.generation = "g1"
                args.state_schema = "1"
                args.worktree = str(server.worktree)
                result = PROFILE.run_probe(
                    "code-luna",
                    tool_policy="coding",
                    args=args,
                    timeout=5,
                )
                self.assertEqual(result["ok"], True)
                self.assertEqual(result["model"], "gpt-5.6-luna")
                self.assertEqual(result["context"], "default")
                self.assertEqual(result["effort"], "max")
            finally:
                server.stop()

    def test_direct_fixture_probe_accepts_the_host_worktree_cwd(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-direct-probe-") as raw:
            root = pathlib.Path(raw)
            worktree = root / "worktree"
            worktree.mkdir()
            copilot = root / "copilot"
            copilot.write_text(
                f"#!{sys.executable}\n"
                f"import runpy\nrunpy.run_path({str(FAKE_ACP)!r}, run_name=\"__main__\")\n",
                encoding="utf-8",
            )
            copilot.chmod(0o755)
            args = PROFILE._default_namespace("code-luna", "coding")
            args.fixture_direct = True
            args.launcher = str(SCRIPT_ROOT / "agent-launcher.py")
            args.copilot = str(copilot)
            args.run_id = "direct-run"
            args.bead_id = "direct-bead"
            args.generation = "g1"
            args.state_schema = "1"
            args.worktree = str(worktree)
            args.lease_root = str(root / "leases")
            args.runtime_root = str(root / "runtime")
            with mock.patch.dict(os.environ, {"GC_TEST_MODE": "1"}):
                result = PROFILE.run_probe(
                    "code-luna",
                    tool_policy="coding",
                    args=args,
                    timeout=5,
                )
            self.assertEqual(result["ok"], True, result)
            self.assertEqual(result["model"], "gpt-5.6-luna")

    def test_direct_probe_classifies_exception_with_stderr(self) -> None:
        cases = (
            (BrokenPipeError("write failed"), "model unavailable", "unavailable"),
            (PROFILE.ProfileError("probe failed"), "unknown model", "unsupported"),
            (
                PROFILE.ACPMalformed("typed ACP failure"),
                "unsupported",
                "malformed",
            ),
        )

        class SynchronousThread:
            def __init__(self, target, name):
                self.target = target

            def start(self) -> None:
                self.target()

            def join(self, timeout: float | None = None) -> None:
                return None

        for error, stderr, expected_code in cases:
            with self.subTest(error=type(error).__name__, stderr=stderr):
                with tempfile.TemporaryDirectory(
                    prefix="gascity-direct-probe-error-"
                ) as raw:
                    root = pathlib.Path(raw)
                    worktree = root / "worktree"
                    worktree.mkdir()
                    child = (
                        "import sys\n"
                        f"sys.stderr.write({stderr!r})\n"
                    )
                    args = PROFILE._default_namespace("code-luna", "coding")
                    args.fixture_direct = True
                    args.launcher = None
                    args.copilot = None
                    args.worktree = str(worktree)
                    with (
                        mock.patch.dict(os.environ, {"GC_TEST_MODE": "1"}),
                        mock.patch.object(
                            PROFILE,
                            "build_launch_argv",
                            return_value=[sys.executable, "-c", child],
                        ),
                        mock.patch.object(
                            PROFILE.threading,
                            "Thread",
                            SynchronousThread,
                        ),
                        mock.patch.object(
                            PROFILE,
                            "_probe_exchange",
                            side_effect=error,
                        ),
                    ):
                        result = PROFILE.run_probe(
                            "code-luna",
                            tool_policy="coding",
                            args=args,
                            timeout=1,
                        )

                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], expected_code)
                self.assertIn(stderr, result["error"])

    def test_direct_probe_classifies_real_child_provider_stderr_after_stdout_close(
        self,
    ) -> None:
        cases = (
            ("unknown model", "unsupported"),
            ("model unavailable", "unavailable"),
        )
        for stderr, expected_code in cases:
            with self.subTest(stderr=stderr):
                with tempfile.TemporaryDirectory(
                    prefix="gascity-direct-probe-child-"
                ) as raw:
                    root = pathlib.Path(raw)
                    worktree = root / "worktree"
                    worktree.mkdir()
                    stderr_line = stderr + "\n"
                    child = (
                        "import sys\n"
                        f"sys.stderr.write({stderr_line!r})\n"
                        "sys.stderr.flush()\n"
                        "sys.stdout.close()\n"
                        "sys.stdin.buffer.read()\n"
                    )
                    args = PROFILE._default_namespace("code-luna", "coding")
                    args.fixture_direct = True
                    args.launcher = None
                    args.copilot = None
                    args.worktree = str(worktree)
                    with (
                        mock.patch.dict(os.environ, {"GC_TEST_MODE": "1"}),
                        mock.patch.object(
                            PROFILE,
                            "build_launch_argv",
                            return_value=[sys.executable, "-c", child],
                        ),
                    ):
                        result = PROFILE.run_probe(
                            "code-luna",
                            tool_policy="coding",
                            args=args,
                            timeout=2,
                        )

                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], expected_code)
                self.assertIn(stderr, result["error"])

    def test_client_eof_stops_child_and_releases_lease(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-eof-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(root, extra=("--ignore-eof",))
            server.start()
            try:
                process = subprocess.Popen(
                    server.client_command(),
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env=dict(os.environ),
                )
                stdout, _stderr = process.communicate(b"", timeout=5)
                self.assertEqual(process.returncode, 0)
                self.assertEqual(stdout, b"")
                self._assert_lease_available(root, "run-1")
            finally:
                server.stop()

    def test_closed_probe_is_not_a_fallback_success(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-closed-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(
                root,
                extra=("--close-after-initialize",),
            )
            server.start()
            try:
                args = PROFILE._default_namespace("code-luna", "coding")
                args.launcher_socket = str(server.socket_path)
                args.run_id = "run-1"
                args.bead_id = "bead-1"
                args.generation = "g1"
                args.state_schema = "1"
                args.worktree = str(server.worktree)
                result = PROFILE.run_probe(
                    "code-luna",
                    tool_policy="coding",
                    args=args,
                    timeout=5,
                )
                self.assertFalse(result["ok"])
                self.assertEqual(result["error_code"], "closed")
            finally:
                server.stop()

    def test_control_cancel_stops_only_exact_child_and_releases_lease(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-cancel-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(
                root,
                extra=("--ignore-eof",),
            )
            server.start()
            parent, child = socket.socketpair()
            try:
                environment = dict(os.environ)
                environment["GC_TEST_MODE"] = "1"
                process = subprocess.Popen(
                    server.client_command(control_fd=child.fileno()),
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    env=environment,
                    pass_fds=(child.fileno(),),
                )
                child.close()
                parent.sendall(b'{"run_id":"run-1","op":"cancel"}\n')
                self.assertIn(b'"ok":true', parent.recv(1024))
                process.wait(timeout=5)
                self.assertEqual(process.returncode, 0)
                self._assert_lease_available(root, "run-1")
            finally:
                parent.close()
                server.stop()

    def test_same_run_concurrent_beads_share_one_active_run_lease(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-same-run-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(
                root,
                extra=("--ignore-eof",),
                max_agents=3,
                max_active_runs=1,
            )
            server.start()
            clients: list[subprocess.Popen[bytes]] = []
            try:
                for index in range(2):
                    clients.append(
                        subprocess.Popen(
                            server.client_command(
                                run_id="shared-run",
                                bead_id=f"bead-{index}",
                            ),
                            stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                            env=dict(os.environ),
                        )
                    )
                deadline = time.monotonic() + 5
                while True:
                    exited = next(
                        (client for client in clients if client.poll() is not None),
                        None,
                    )
                    if exited is not None:
                        stderr = (
                            exited.stderr.read().decode("utf-8", errors="replace")
                            if exited.stderr is not None
                            else ""
                        )
                        self.fail(f"same-run client exited early: {stderr}")
                    slots = self._active_run_slots(root)
                    if (
                        slots.get("0", {}).get("run_id") == "shared-run"
                        and slots.get("0", {}).get("refcount") == 2
                    ):
                        break
                    if time.monotonic() >= deadline:
                        self.fail("same-run clients did not hold the active-run lease")
                    time.sleep(0.01)

                with self.assertRaises(LAUNCHER.LeaseBusy):
                    LAUNCHER.ConcurrencyLease.acquire(
                        root / "leases",
                        run_id="other-run",
                        bead_id="other-bead",
                        max_agents=3,
                        max_active_runs=1,
                    )

                server.process.send_signal(signal.SIGTERM)
                server.process.wait(timeout=5)
                for client in clients:
                    if client.stdin is not None:
                        client.stdin.close()
                    client.wait(timeout=5)
                    if client.returncode != 0:
                        stderr = (
                            client.stderr.read().decode("utf-8", errors="replace")
                            if client.stderr is not None
                            else ""
                        )
                        self.fail(f"same-run client failed during drain: {stderr}")

                lease = LAUNCHER.ConcurrencyLease.acquire(
                    root / "leases",
                    run_id="other-run",
                    bead_id="other-bead",
                    max_agents=3,
                    max_active_runs=1,
                )
                lease.release()
            finally:
                for client in clients:
                    if client.poll() is None:
                        client.kill()
                        client.wait()
                server.stop()

    def test_active_run_reference_count_releases_after_last_bead(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-lease-refcount-") as raw:
            root = pathlib.Path(raw)
            first = LAUNCHER.ConcurrencyLease.acquire(
                root / "leases",
                run_id="shared-run",
                bead_id="bead-1",
                max_agents=3,
                max_active_runs=1,
            )
            second = LAUNCHER.ConcurrencyLease.acquire(
                root / "leases",
                run_id="shared-run",
                bead_id="bead-2",
                max_agents=3,
                max_active_runs=1,
            )
            with self.assertRaises(LAUNCHER.LeaseBusy):
                LAUNCHER.ConcurrencyLease.acquire(
                    root / "leases",
                    run_id="other-run",
                    bead_id="other-bead",
                    max_agents=3,
                    max_active_runs=1,
                )
            first.release()
            with self.assertRaises(LAUNCHER.LeaseBusy):
                LAUNCHER.ConcurrencyLease.acquire(
                    root / "leases",
                    run_id="other-run",
                    bead_id="other-bead",
                    max_agents=3,
                    max_active_runs=1,
                )
            second.release()
            released = LAUNCHER.ConcurrencyLease.acquire(
                root / "leases",
                run_id="other-run",
                bead_id="other-bead",
                max_agents=3,
                max_active_runs=1,
            )
            released.release()

    def test_concurrent_sessions_and_service_drain_release_all_leases(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-concurrent-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(
                root,
                extra=("--ignore-eof",),
            )
            server.start()
            clients: list[subprocess.Popen[bytes]] = []
            try:
                for index in range(2):
                    clients.append(
                        subprocess.Popen(
                            server.client_command(
                                run_id=f"run-{index}",
                                bead_id=f"bead-{index}",
                            ),
                            stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                            env=dict(os.environ),
                        )
                    )
                deadline = time.monotonic() + 5
                while True:
                    exited = next((client for client in clients if client.poll() is not None), None)
                    if exited is not None:
                        stderr = (
                            exited.stderr.read().decode("utf-8", errors="replace")
                            if exited.stderr is not None
                            else ""
                        )
                        self.fail(f"launcher client exited before drain: {stderr}")
                    slots = self._active_run_slots(root)
                    if (
                        len(slots) == 2
                        and {
                            record.get("run_id") for record in slots.values()
                        }
                        == {"run-0", "run-1"}
                        and all(
                            record.get("refcount") == 1
                            for record in slots.values()
                        )
                    ):
                        break
                    if time.monotonic() >= deadline:
                        self.fail("concurrent launcher sessions did not acquire both leases")
                    time.sleep(0.01)
                server.process.send_signal(signal.SIGTERM)
                server.process.wait(timeout=5)
                for client in clients:
                    if client.stdin is not None:
                        client.stdin.close()
                    client.wait(timeout=5)
                    if client.returncode != 0:
                        stderr = (
                            client.stderr.read().decode("utf-8", errors="replace")
                            if client.stderr is not None
                            else ""
                        )
                        self.fail(f"launcher client failed during drain: {stderr}")
                for index in range(2):
                    lease = LAUNCHER.ConcurrencyLease.acquire(
                        root / "leases",
                        run_id=f"run-{index}",
                        max_agents=2,
                        max_active_runs=2,
                    )
                    lease.release()
            finally:
                for client in clients:
                    if client.poll() is None:
                        client.kill()
                        client.wait()
                server.stop()


class CheckRunnerHarness:
    def __init__(
        self,
        root: pathlib.Path,
        *,
        approved: tuple[str, ...] = ("smoke=smoke.sh",),
        timeout: float = 2.0,
        max_heavy_checks: int = 1,
    ):
        self.root = root
        self.snapshot = root / "snapshot"
        self.worktree = self.snapshot / "run-1"
        self.worktree.mkdir(parents=True)
        self.output = root / "output"
        self.store = root / "store"
        self.socket_path = root / "check.sock"
        self.egress_socket = root / "egress.sock"
        self.process: subprocess.Popen[bytes] | None = None
        self.approved = approved
        self.timeout = timeout
        self.max_heavy_checks = max_heavy_checks

    def write_check(self, name: str, body: str) -> None:
        path = self.worktree / name
        path.write_text(body, encoding="utf-8")
        path.chmod(0o700)

    def start(self) -> None:
        command = [
            sys.executable,
            str(SCRIPT_ROOT / "check-runner.py"),
            "server",
            "--socket",
            str(self.socket_path),
            "--snapshot-root",
            str(self.snapshot),
            "--store-root",
            str(self.store),
            "--output-root",
            str(self.output),
            "--proxy",
            "http://127.0.0.1:3129",
            "--listen-port",
            "3129",
            "--egress-socket",
            str(self.egress_socket),
            "--auth-token-env",
            "GC_CHECK_AUTH",
            "--allowed-uid",
            str(os.geteuid()),
            "--timeout-seconds",
            str(self.timeout),
            "--term-grace",
            "0.05",
            "--kill-grace",
            "0.05",
            "--max-heavy-checks",
            str(self.max_heavy_checks),
        ]
        for check in self.approved:
            command.extend(["--approved-check", check])
        environment = dict(os.environ)
        environment["GC_CHECK_AUTH"] = "fixture-check-auth"
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
        )
        deadline = time.monotonic() + 5
        while not self.socket_path.exists():
            if self.process.poll() is not None:
                stderr = (
                    self.process.stderr.read().decode("utf-8", errors="replace")
                    if self.process.stderr is not None
                    else ""
                )
                raise AssertionError(f"check runner exited before binding: {stderr}")
            if time.monotonic() >= deadline:
                raise AssertionError("check runner did not bind")
            time.sleep(0.01)

    def bind(self, *, auth: str = "fixture-check-auth") -> socket.socket:
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.connect(str(self.socket_path))
        request = {
            "protocol": CHECK_RUNNER.CHECK_PROTOCOL,
            "operation": "bind",
            "run_id": "run-1",
            "bead_id": "bead-1",
            "worktree": str(self.worktree),
            "auth": CHECK_RUNNER.bind_auth_token(
                auth,
                run_id="run-1",
                bead_id="bead-1",
                worktree=str(self.worktree),
            ),
        }
        client.sendall(json.dumps(request, separators=(",", ":")).encode() + b"\n")
        response = self.read_line(client)
        if response.get("ok") is not True:
            client.close()
            raise AssertionError(response)
        return client

    @staticmethod
    def read_line(client: socket.socket) -> dict[str, object]:
        data = bytearray()
        while not data.endswith(b"\n"):
            chunk = client.recv(4096)
            if not chunk:
                raise AssertionError("check runner channel closed")
            data.extend(chunk)
        return json.loads(bytes(data))

    def run(self, client: socket.socket, check: str) -> dict[str, object]:
        client.sendall(
            json.dumps(
                {
                    "protocol": CHECK_RUNNER.CHECK_PROTOCOL,
                    "operation": "run",
                    "request_id": f"request-{time.monotonic_ns()}",
                    "check": check,
                },
                separators=(",", ":"),
            ).encode()
            + b"\n"
        )
        return self.read_line(client)

    def stop(self) -> None:
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGTERM)
            self.process.wait(timeout=5)
        if self.process.stdout is not None:
            self.process.stdout.close()
        if self.process.stderr is not None:
            self.process.stderr.close()


class CheckRunnerContractTests(unittest.TestCase):
    def test_graceful_process_exit_does_not_receive_group_kill(self) -> None:
        process = mock.Mock()
        process.pid = 1234
        process.wait.return_value = 0
        with mock.patch.object(CHECK_RUNNER.os, "killpg") as killpg:
            CHECK_RUNNER._terminate_process_group(
                process,
                term_grace=0.05,
                kill_grace=0.05,
            )
        killpg.assert_called_once_with(process.pid, signal.SIGTERM)
        process.wait.assert_called_once_with(timeout=0.05)

    def test_authorized_and_unauthorized_socket_requests(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-") as raw:
            root = pathlib.Path(raw)
            server = CheckRunnerHarness(root)
            server.write_check("smoke.sh", "#!/bin/sh\nexit 0\n")
            server.start()
            try:
                authorized = server.bind()
                try:
                    self.assertTrue(server.run(authorized, "smoke")["ok"])
                finally:
                    authorized.close()
                denied = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                denied.connect(str(server.socket_path))
                request = {
                    "protocol": CHECK_RUNNER.CHECK_PROTOCOL,
                    "operation": "bind",
                    "run_id": "run-1",
                    "bead_id": "bead-1",
                    "worktree": str(server.worktree),
                    "auth": CHECK_RUNNER.bind_auth_token(
                        "wrong-auth",
                        run_id="run-1",
                        bead_id="bead-1",
                        worktree=str(server.worktree),
                    ),
                }
                denied.sendall(json.dumps(request).encode() + b"\n")
                response = server.read_line(denied)
                self.assertFalse(response["ok"])
                self.assertEqual(response["error_code"], "unauthorized")
                denied.close()
            finally:
                server.stop()

    def test_malformed_socket_requests_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-malformed-") as raw:
            root = pathlib.Path(raw)
            server = CheckRunnerHarness(root)
            server.write_check("smoke.sh", "#!/bin/sh\nexit 0\n")
            server.start()
            try:
                client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                client.connect(str(server.socket_path))
                client.sendall(b"not-json\n")
                response = server.read_line(client)
                self.assertFalse(response["ok"])
                self.assertEqual(response["error_code"], "malformed_request")
                client.close()
            finally:
                server.stop()

    def test_real_approved_command_dispatch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-dispatch-") as raw:
            root = pathlib.Path(raw)
            server = CheckRunnerHarness(root)
            server.write_check(
                "smoke.sh",
                "#!/bin/sh\nprintf dispatched > \"$GC_CHECK_OUTPUT_ROOT/result\"\n",
            )
            server.start()
            try:
                client = server.bind()
                try:
                    response = server.run(client, "smoke")
                    request_code = CHECK_RUNNER.request_check(
                        fd=client.fileno(),
                        check_name="smoke",
                    )
                finally:
                    client.close()
                self.assertEqual(
                    response,
                    {
                        "ok": True,
                        "returncode": 0,
                        "protocol": CHECK_RUNNER.CHECK_PROTOCOL,
                    },
                )
                self.assertEqual(request_code, 0)
                self.assertEqual((server.output / "result").read_text(), "dispatched")
            finally:
                server.stop()

    def test_request_client_deadline_covers_long_configured_check(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-client-deadline-") as raw:
            root = pathlib.Path(raw)
            server = CheckRunnerHarness(root, timeout=31.0)
            server.write_check("smoke.sh", "#!/bin/sh\nexit 0\n")
            server.start()
            client = server.bind()
            try:
                real_socket = socket.socket
                timeouts: list[float | None] = []

                class RecordingSocket:
                    def __init__(self, *, fileno: int):
                        self._socket = real_socket(fileno=fileno)

                    def settimeout(self, value: float | None) -> None:
                        timeouts.append(value)
                        self._socket.settimeout(value)

                    def sendall(self, data: bytes) -> None:
                        self._socket.sendall(data)

                    def recv(self, size: int) -> bytes:
                        return self._socket.recv(size)

                    def close(self) -> None:
                        self._socket.close()

                with mock.patch.object(CHECK_RUNNER.socket, "socket", RecordingSocket):
                    self.assertEqual(
                        CHECK_RUNNER.request_check(
                            fd=client.fileno(),
                            check_name="smoke",
                        ),
                        0,
                    )
                self.assertEqual(
                    timeouts,
                    [CHECK_RUNNER.CHECK_CLIENT_TIMEOUT_SECONDS],
                )
                self.assertGreater(timeouts[0], server.timeout)
            finally:
                client.close()
                server.stop()

    def test_timeout_kills_exact_process_group_and_releases_slot(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-timeout-") as raw:
            root = pathlib.Path(raw)
            server = CheckRunnerHarness(
                root,
                approved=("timeout=timeout.sh", "smoke=smoke.sh"),
                timeout=0.2,
            )
            server.write_check(
                "timeout.sh",
                "#!/bin/sh\n"
                "trap '' TERM\n"
                "(sleep 30) &\n"
                "printf '%s' \"$!\" > \"$GC_CHECK_OUTPUT_ROOT/child.pid\"\n"
                "wait\n",
            )
            server.write_check("smoke.sh", "#!/bin/sh\nexit 0\n")
            server.start()
            try:
                client = server.bind()
                try:
                    response = server.run(client, "timeout")
                    self.assertFalse(response["ok"])
                    self.assertEqual(response["error_code"], "timeout")
                    child_pid = int((server.output / "child.pid").read_text())
                    deadline = time.monotonic() + 2
                    while pathlib.Path(f"/proc/{child_pid}").exists():
                        if time.monotonic() >= deadline:
                            self.fail("timed-out check child remained alive")
                        time.sleep(0.02)
                    self.assertTrue(server.run(client, "smoke")["ok"])
                finally:
                    client.close()
            finally:
                server.stop()

    def test_configured_max_heavy_checks_serializes_requests(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-concurrency-") as raw:
            root = pathlib.Path(raw)
            server = CheckRunnerHarness(
                root,
                approved=("heavy=heavy.sh",),
                timeout=2,
                max_heavy_checks=1,
            )
            server.write_check(
                "heavy.sh",
                "#!/bin/sh\n"
                "if ! mkdir \"$GC_CHECK_OUTPUT_ROOT/active\"; then\n"
                "  printf overlap > \"$GC_CHECK_OUTPUT_ROOT/overlap\"\n"
                "fi\n"
                "trap 'rmdir \"$GC_CHECK_OUTPUT_ROOT/active\" 2>/dev/null || true' EXIT\n"
                "sleep 0.2\n",
            )
            server.start()
            clients = [server.bind(), server.bind()]
            try:
                with ThreadPoolExecutor(max_workers=2) as executor:
                    responses = list(
                        executor.map(lambda client: server.run(client, "heavy"), clients)
                    )
                self.assertEqual([response["returncode"] for response in responses], [0, 0])
                self.assertFalse((server.output / "overlap").exists())
            finally:
                for client in clients:
                    client.close()
                server.stop()


class CheckChannelLaunchContractTests(unittest.TestCase):
    def test_only_coding_launches_receive_authenticated_check_fd(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-check-launch-") as raw:
            root = pathlib.Path(raw)
            socket_path = root / "check.sock"
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(socket_path))
            listener.listen(1)
            received: list[dict[str, object]] = []
            ready = threading.Event()

            def serve_bind() -> None:
                ready.set()
                client, _address = listener.accept()
                try:
                    received.append(CheckRunnerHarness.read_line(client))
                    client.sendall(
                        json.dumps(
                            {
                                "protocol": PROFILE.CHECK_PROTOCOL,
                                "ok": True,
                                "operation": "bind",
                            }
                        ).encode()
                        + b"\n"
                    )
                    client.recv(1)
                finally:
                    client.close()

            thread = threading.Thread(target=serve_bind, daemon=True)
            thread.start()
            ready.wait(timeout=1)
            args = PROFILE._default_namespace("code-luna", "coding")
            args.run_id = "run-1"
            args.bead_id = "bead-1"
            args.generation = "g1"
            args.state_schema = "1"
            args.worktree = str(root)
            old_socket = os.environ.get("GC_CHECK_SOCKET")
            old_auth = os.environ.get("GC_CHECK_AUTH")
            os.environ["GC_CHECK_SOCKET"] = str(socket_path)
            os.environ["GC_CHECK_AUTH"] = "fixture-check-auth"
            try:
                metadata, descriptors = PROFILE._launch_metadata(
                    args,
                    profile="code-luna",
                    tool_policy="coding",
                )
                self.assertIn("check", metadata["fds"])
                self.assertEqual(len(descriptors), 1)
                os.close(descriptors.pop())
                thread.join(timeout=2)
                self.assertEqual(received[0]["operation"], "bind")
                self.assertEqual(received[0]["run_id"], "run-1")
            finally:
                if old_socket is None:
                    os.environ.pop("GC_CHECK_SOCKET", None)
                else:
                    os.environ["GC_CHECK_SOCKET"] = old_socket
                if old_auth is None:
                    os.environ.pop("GC_CHECK_AUTH", None)
                else:
                    os.environ["GC_CHECK_AUTH"] = old_auth
                listener.close()
                thread.join(timeout=2)

            review_args = PROFILE._default_namespace("review-luna", "review")
            review_args.run_id = "run-2"
            review_args.bead_id = "bead-2"
            review_args.generation = "g1"
            review_args.state_schema = "1"
            review_args.worktree = str(root)
            metadata, descriptors = PROFILE._launch_metadata(
                review_args,
                profile="review-luna",
                tool_policy="review",
            )
            self.assertNotIn("check", metadata["fds"])
            self.assertEqual(descriptors, [])


class FDProxyContractTests(unittest.TestCase):
    def setUp(self) -> None:
        self.sidecar, self.inherited = socket.socketpair()
        self.channel = FDPROXY.MultiplexedChannel(
            self.inherited.fileno(),
            auth_token="fixture-auth",
        )
        self.listener = FDPROXY._listen("127.0.0.1", 0)
        self.stop_event = threading.Event()
        self.port = self.listener.getsockname()[1]
        self.sidecar_thread = threading.Thread(target=self._sidecar, daemon=True)
        self.proxy_thread = threading.Thread(
            target=FDPROXY._serve_listener,
            args=(self.listener, self.channel),
            kwargs={"stop_event": self.stop_event},
            daemon=True,
        )
        self.sidecar_thread.start()
        self.proxy_thread.start()

    def tearDown(self) -> None:
        self.stop_event.set()
        try:
            self.listener.close()
        except OSError:
            pass
        self.inherited.close()
        self.sidecar.close()
        self.channel.close()
        self.proxy_thread.join(timeout=2)
        self.sidecar_thread.join(timeout=2)

    def _receive_line(self) -> dict[str, object]:
        data = bytearray()
        while not data.endswith(b"\n"):
            chunk = self.sidecar.recv(4096)
            if not chunk:
                raise AssertionError("sidecar channel closed")
            data.extend(chunk)
        return json.loads(bytes(data))

    def _sidecar(self) -> None:
        try:
            while True:
                request = self._receive_line()
                self.assertEqual(request["version"], "fdproxy/1")
                self.assertEqual(request["operation"], "connect")
                self.assertEqual(request["auth"], "fixture-auth")
                request_id = request["request_id"]
                host = request["host"]
                if host == "deny":
                    self.sidecar.sendall(
                        (
                            json.dumps(
                                {
                                    "version": "fdproxy/1",
                                    "request_id": request_id,
                                    "ok": False,
                                    "error": "denied",
                                },
                                separators=(",", ":"),
                            )
                            + "\n"
                        ).encode("ascii")
                    )
                    continue
                if host == "malformed":
                    local_one, passed_one = socket.socketpair()
                    local_two, passed_two = socket.socketpair()
                    try:
                        response = (
                            json.dumps(
                                {
                                    "version": "fdproxy/1",
                                    "request_id": request_id,
                                    "ok": True,
                                },
                                separators=(",", ":"),
                            )
                            + "\n"
                        ).encode("ascii")
                        self.sidecar.sendmsg(
                            [response],
                            [
                                (
                                    socket.SOL_SOCKET,
                                    socket.SCM_RIGHTS,
                                    array.array(
                                        "i",
                                        [passed_one.fileno(), passed_two.fileno()],
                                    ).tobytes(),
                                )
                            ],
                        )
                    finally:
                        local_one.close()
                        passed_one.close()
                        local_two.close()
                        passed_two.close()
                    continue
                local, passed = socket.socketpair()
                marker = f"upstream-{host}-{request_id}".encode("ascii")
                local.sendall(marker)
                response = (
                    json.dumps(
                        {
                            "version": "fdproxy/1",
                            "request_id": request_id,
                            "ok": True,
                        },
                        separators=(",", ":"),
                    )
                    + "\n"
                ).encode("ascii")
                self.sidecar.sendmsg(
                    [response],
                    [
                        (
                            socket.SOL_SOCKET,
                            socket.SCM_RIGHTS,
                            array.array("i", [passed.fileno()]).tobytes(),
                        )
                    ],
                )
                passed.close()
                local.close()
        except (OSError, AssertionError):
            return

    def _connect(self, host: str) -> bytes:
        client = socket.create_connection(("127.0.0.1", self.port))
        try:
            client.sendall(
                f"CONNECT {host}:443 HTTP/1.1\r\nHost: {host}\r\n\r\n".encode("ascii")
            )
            response = bytearray()
            while b"\r\n\r\n" not in response:
                chunk = client.recv(4096)
                self.assertTrue(chunk)
                response.extend(chunk)
            header, _separator, remainder = bytes(response).partition(b"\r\n\r\n")
            self.assertIn(b"200 Connection Established", header)
            return remainder + client.recv(4096)
        finally:
            client.close()

    def test_two_sequential_connects_receive_distinct_passed_sockets(self) -> None:
        first = self._connect("one")
        second = self._connect("two")
        self.assertIn(b"upstream-one-1", first)
        self.assertIn(b"upstream-two-2", second)
        self.assertNotEqual(first, second)

    def test_two_concurrent_connects_receive_distinct_passed_sockets(self) -> None:
        with ThreadPoolExecutor(max_workers=2) as executor:
            values = list(executor.map(self._connect, ("three", "four")))
        self.assertEqual(len(values), 2)
        self.assertNotEqual(values[0], values[1])
        self.assertTrue(any(b"upstream-three" in value for value in values))
        self.assertTrue(any(b"upstream-four" in value for value in values))

    def test_denial_fails_closed(self) -> None:
        client = socket.create_connection(("127.0.0.1", self.port))
        try:
            client.sendall(b"CONNECT deny:443 HTTP/1.1\r\n\r\n")
            self.assertIn(b"403 Forbidden", client.recv(4096))
        finally:
            client.close()

    def test_malformed_ancillary_data_fails_closed(self) -> None:
        client = socket.create_connection(("127.0.0.1", self.port))
        try:
            client.sendall(b"CONNECT malformed:443 HTTP/1.1\r\n\r\n")
            self.assertIn(b"400 Bad Request", client.recv(4096))
        finally:
            client.close()

    def test_fdproxy_never_opens_outbound_sockets(self) -> None:
        source = (SCRIPT_ROOT / "fdproxy.py").read_text(encoding="utf-8")
        self.assertNotIn("connect(", source)
        self.assertIn("SCM_RIGHTS", source)
        self.assertEqual(FDPROXY.PROTOCOL, "fdproxy/1")


if __name__ == "__main__":
    unittest.main()
