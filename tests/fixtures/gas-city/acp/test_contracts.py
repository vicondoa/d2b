"""Hermetic contract coverage for the U3 ACP and inherited-channel boundaries."""

from __future__ import annotations

import array
import importlib.util
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor


ROOT = pathlib.Path(__file__).resolve().parents[4]
SCRIPT_ROOT = ROOT / "nix/gas-city-contributor/pack/scripts"
COPILOT_ROOT = ROOT / "nix/gas-city-contributor/copilot"
FAKE_ACP = pathlib.Path(__file__).with_name("fake_acp.py")


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
    def __init__(self, root: pathlib.Path, *, extra: tuple[str, ...] = ()):
        self.root = root
        self.worktree = root / "worktree"
        self.worktree.mkdir()
        self.socket_path = root / "agent.sock"
        self.process: subprocess.Popen[bytes] | None = None
        self.extra = extra

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
            "2",
            "--max-active-runs",
            "2",
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
                time.sleep(0.1)
                server.process.send_signal(signal.SIGTERM)
                server.process.wait(timeout=5)
                for client in clients:
                    if client.stdin is not None:
                        client.stdin.close()
                    client.wait(timeout=5)
                    self.assertEqual(client.returncode, 0)
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
