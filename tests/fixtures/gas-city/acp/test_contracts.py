"""Hermetic contract coverage for the U3 ACP and inherited-channel boundaries."""

from __future__ import annotations

import array
import fcntl
import importlib.util
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


class ProfileContractTests(unittest.TestCase):
    def test_settings_have_only_persistent_authority(self) -> None:
        for profile in ("review-sol", "review-luna", "code-luna"):
            path = COPILOT_ROOT / profile / "settings.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(set(value), {"model", "contextTier"})
            self.assertEqual(PROFILE.load_profile(profile), value)

    def test_effort_is_an_acp_startup_argument(self) -> None:
        expected = {
            "review-sol": "xhigh",
            "review-luna": "max",
            "code-luna": "max",
        }
        for profile, effort in expected.items():
            argv = PROFILE.child_argv(
                profile,
                tool_policy="coding" if profile == "code-luna" else "review",
            )
            effort_index = argv.index("--effort")
            self.assertEqual(argv[effort_index + 1], effort)
            self.assertNotIn("--model", argv)
            self.assertNotIn("--context", argv)

    def test_model_override_is_rejected_and_effort_must_match(self) -> None:
        with self.assertRaises(LAUNCHER.LauncherError):
            LAUNCHER.validate_child_arguments(["--acp", "--model", "gpt-4"])
        with self.assertRaises(LAUNCHER.LauncherError):
            LAUNCHER.validate_child_arguments(
                ["--acp", "--effort", "xhigh"],
                profile="code-luna",
            )
        self.assertEqual(
            LAUNCHER.validate_child_arguments(["--acp"], profile="code-luna")[-2:],
            ["--effort", "max"],
        )

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
    def test_sol_success_selects_sol_without_fallback_probe(self) -> None:
        calls: list[str] = []

        def probe(profile: str):
            calls.append(profile)
            return successful_probe(profile)

        status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
        self.assertTrue(status["ready"])
        self.assertEqual(status["effective_profiles"]["review"], "review-sol")
        self.assertEqual(calls, ["code-luna", "review-sol"])

    def test_only_unsupported_or_unavailable_sol_failures_fallback(self) -> None:
        calls: list[str] = []

        def probe(profile: str):
            calls.append(profile)
            if profile == "review-sol":
                return {"profile": profile, "ok": False, "error_code": "unavailable"}
            return successful_probe(profile)

        status = ACTIVATION.select_profiles(probe, generation="g1", state_schema="1")
        self.assertTrue(status["ready"])
        self.assertEqual(status["effective_profiles"]["review"], "review-luna")
        self.assertEqual(calls, ["code-luna", "review-sol", "review-luna"])

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

    def test_gc_root_lifecycle_removes_only_terminal_run_roots(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-gcroots-") as raw:
            root = pathlib.Path(raw) / "roots"
            targets = {}
            for name in ACTIVATION.GC_ROOT_NAMES:
                target = pathlib.Path(raw) / name
                target.mkdir()
                targets[name] = target
            roots = ACTIVATION.ActiveRunGCRoots.create(
                root,
                run_id="run-1",
                generation_paths=targets,
                allowed_prefixes=(f"{raw}/",),
            )
            with self.assertRaises(ACTIVATION.RootLifecycleError):
                roots.cleanup(terminal=False)
            roots.cleanup(terminal=True)
            self.assertFalse((root / "run-1").exists())

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
        max_agents: int = 2,
        max_active_runs: int = 2,
    ):
        self.root = root
        self.worktree = root / "worktree"
        self.worktree.mkdir()
        self.socket_path = root / "agent.sock"
        self.process: subprocess.Popen[bytes] | None = None
        self.extra = extra
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
                    b'{"jsonrpc":"2.0","id":2,"method":"session/new","params":{}}\n'
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
                        message.get("result", {}).get("effectiveModel") == "gpt-5.6-luna"
                        for message in messages
                        if isinstance(message.get("result"), dict)
                    )
                )
                self._assert_lease_available(root, "run-1")
            finally:
                server.stop()

    def test_client_probe_preserves_initialize_session_new_and_diagnostic_prompt(self) -> None:
        with tempfile.TemporaryDirectory(prefix="gascity-probe-") as raw:
            root = pathlib.Path(raw)
            server = LauncherServerHarness(root)
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
                    try:
                        probe = LAUNCHER.ConcurrencyLease.acquire(
                            root / "leases",
                            run_id="other-run",
                            bead_id="other-bead",
                            max_agents=3,
                            max_active_runs=1,
                        )
                    except LAUNCHER.LeaseBusy:
                        slots = self._active_run_slots(root)
                        if (
                            slots.get("0", {}).get("run_id") == "shared-run"
                            and slots.get("0", {}).get("refcount") == 2
                        ):
                            break
                    else:
                        probe.release()
                    if time.monotonic() >= deadline:
                        self.fail("same-run clients did not hold the active-run lease")
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
