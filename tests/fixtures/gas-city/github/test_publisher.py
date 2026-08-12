#!/usr/bin/env python3
"""Runnable U5 publisher fixture.

The fixture exercises the real request validation and publication state
machine with deterministic Git/GitHub doubles.  The final test also creates a
real local Git bundle, proving that the bundle is unlinked from the worktree
and can be imported into a publisher-owned bare clone:

    python3 tests/fixtures/gas-city/github/test_publisher.py
"""

from __future__ import annotations

import importlib.util
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.error
from collections.abc import Mapping
from typing import Any
from unittest.mock import patch


ROOT = pathlib.Path(__file__).resolve().parents[4]
SCRIPT = ROOT / "nix/gas-city-contributor/pack/scripts/publish-pr.py"
FIXTURE_PRIVATE_KEY = pathlib.Path(__file__).with_name("fixture-private-key.pem")
SPEC = importlib.util.spec_from_file_location("gascity_publish_pr", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


REPOSITORY = "acme/project"
BASE = "main"
HEAD = "gascity/run-u5"
SHA = "a" * 40


def pull_request(
    *,
    state: str = "open",
    sha: str = SHA,
    repository: str = REPOSITORY,
    number: int = 7,
    merged: bool = False,
    merged_at: str | None = None,
) -> dict[str, object]:
    return {
        "number": number,
        "state": state,
        "merged": merged,
        "merged_at": merged_at,
        "html_url": f"https://github.com/{repository}/pull/{number}",
        "head": {"ref": HEAD, "sha": sha, "repo": {"full_name": repository}},
        "base": {"ref": BASE, "repo": {"full_name": repository}},
    }


class FakeHTTPResponse:
    def __init__(self, status: int, payload: object) -> None:
        self.status = status
        self.payload = payload

    def __enter__(self) -> "FakeHTTPResponse":
        return self

    def __exit__(self, *_args: object) -> None:
        return None

    def read(self, _limit: int) -> bytes:
        return MODULE.json.dumps(self.payload).encode("utf-8")


class FakeGit:
    def __init__(self) -> None:
        self.configured: list[tuple[pathlib.Path, str]] = []
        self.imports: list[tuple[pathlib.Path, int, str, str]] = []
        self.pushes: list[tuple[pathlib.Path, str, str]] = []
        self.push_plan: list[object] = []
        self.remote_plan: list[object] = []
        self.remote_sha = ""

    def configure_bare(self, repository: pathlib.Path, remote_url: str) -> None:
        self.configured.append((repository, remote_url))

    def import_bundle(
        self,
        repository: pathlib.Path,
        bundle_fd: int,
        *,
        head: str,
        expected_sha: str = "",
    ) -> str:
        self.imports.append((repository, bundle_fd, head, expected_sha))
        return SHA

    def push(self, repository: pathlib.Path, head: str, token: str = "") -> None:
        self.pushes.append((repository, head, token))
        if self.push_plan:
            outcome = self.push_plan.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
        self.remote_sha = SHA

    def remote_head(self, _repository: pathlib.Path, _head: str, _token: str = "") -> str:
        if self.remote_plan:
            outcome = self.remote_plan.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            return str(outcome)
        return self.remote_sha


class FakeGitHub:
    def __init__(self) -> None:
        self.matches: list[dict[str, object]] = []
        self.created: list[dict[str, object]] = []
        self.create_plan: list[object] = []
        self.find_count = 0
        self.identity_calls: list[str] = []
        self.installation_calls: list[str] = []
        self.merge_calls: list[object] = []

    def repository_identity(self, repository: str) -> dict[str, object]:
        self.identity_calls.append(repository)
        return {"full_name": repository}

    def installation_identity(self, repository: str) -> dict[str, object]:
        self.installation_calls.append(repository)
        return {"id": "42"}

    def installation_token(self) -> str:
        return "installation-token"

    def find_pull_requests(self, _repository: str, *, head: str, base: str) -> list[dict[str, object]]:
        self.find_count += 1
        return list(self.matches)

    def create_pull_request(
        self,
        repository: str,
        *,
        head: str,
        base: str,
        title: str,
        body: str,
    ) -> dict[str, object]:
        self.created.append(
            {
                "repository": repository,
                "head": head,
                "base": base,
                "title": title,
                "body": body,
            }
        )
        if self.create_plan:
            outcome = self.create_plan.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            return dict(outcome)
        return pull_request(number=8)


def request(**overrides: object) -> dict[str, object]:
    value: dict[str, object] = {
        "protocol": MODULE.PROTOCOL,
        "run_id": "run-u5",
        "bead_id": "root-u5",
        "repository": REPOSITORY,
        "base": BASE,
        "head": HEAD,
        "head_sha": SHA,
        "branch_namespace": "gascity/",
        "worktree_id": "worktree-u5",
        "title": "U5 publication",
        "body": "bounded body",
        "installation_id": "42",
        "app_id": "app-7",
    }
    value.update(overrides)
    return value


class PublisherFixture(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.cancel_root = pathlib.Path(self.temporary.name, "cancellations")
        self.cancel_root.mkdir()
        self.state = MODULE.PublicationStore(pathlib.Path(self.temporary.name, "state"))
        self.git = FakeGit()
        self.github = FakeGitHub()
        self.sleeps: list[float] = []
        self.publisher = MODULE.Publisher(
            state=self.state,
            git=self.git,
            github=self.github,
            repository=REPOSITORY,
            base_branch=BASE,
            branch_namespace="gascity/",
            cancellation_root=self.cancel_root,
            app_id="app-7",
            installation_id="42",
            sleep=self.sleeps.append,
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def bundle_fd(self) -> Any:
        bundle = tempfile.TemporaryFile(mode="w+b")
        bundle.write(b"fixture bundle")
        bundle.flush()
        bundle.seek(0)
        self.addCleanup(bundle.close)
        return bundle.fileno()

    def publish(self, **overrides: object) -> dict[str, object]:
        return self.publisher.publish(request(**overrides), self.bundle_fd())

    def test_exact_bundle_import_push_and_pr_creation_store_root_state(self) -> None:
        result = self.publish()
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(result["pr_url"], "https://github.com/acme/project/pull/8")
        self.assertEqual(self.git.imports[0][2:], (HEAD, SHA))
        self.assertEqual(self.git.pushes[0][1:], (HEAD, "installation-token"))
        self.assertEqual(self.github.created[0]["head"], HEAD)
        self.assertEqual(self.github.created[0]["base"], BASE)
        self.assertEqual(self.github.identity_calls, [REPOSITORY])
        self.assertEqual(self.github.installation_calls, [REPOSITORY])
        self.assertEqual(result["title"], "U5 publication")

    def test_open_and_merged_exact_prs_are_adopted_without_a_second_create(self) -> None:
        self.github.matches = [pull_request()]
        result = self.publish()
        self.assertEqual(result["pr_url"], "https://github.com/acme/project/pull/7")
        self.assertEqual(self.git.pushes, [])
        self.assertEqual(self.github.created, [])

        self.temporary.cleanup()
        self.setUp()
        self.github.matches = [
            pull_request(
                state="closed",
                merged_at="2026-08-10T12:00:00Z",
            )
        ]
        result = self.publish()
        self.assertEqual(result["pr_url"], "https://github.com/acme/project/pull/7")
        self.assertEqual(self.git.pushes, [])

    def test_closed_unmerged_multiple_cross_repository_and_divergent_matches_block(self) -> None:
        cases = [
            [pull_request(state="closed")],
            [pull_request(number=1), pull_request(number=2)],
            [pull_request(repository="other/project")],
            [pull_request(sha="b" * 40)],
            [
                {
                    **pull_request(),
                    "head": {"ref": HEAD, "sha": SHA, "repo": {"full_name": "other/project"}},
                }
            ],
        ]
        for matches in cases:
            with self.subTest(matches=matches):
                self.github.matches = matches
                with self.assertRaises(MODULE.PublicationError):
                    self.publish()

    def test_cross_repository_ref_and_branch_injection_are_rejected_before_git(self) -> None:
        with self.assertRaises(MODULE.PublicationError):
            self.publish(repository="other/project")
        with self.assertRaises(MODULE.PublicationError):
            self.publish(head="gascity/../main")
        with self.assertRaises(MODULE.PublicationError):
            self.publish(branch_namespace="other/")
        with self.assertRaises(MODULE.PublicationError):
            self.publish(base="release")
        self.assertEqual(self.git.configured, [])
        self.assertEqual(self.github.identity_calls, [])

    def test_non_force_push_retry_ceiling_and_rate_hints(self) -> None:
        self.git.push_plan = [
            MODULE.RetryableGitHubError("rate limited", retry_after=1.0),
            MODULE.RetryableGitHubError("rate limited", retry_after=2.0),
            MODULE.RetryableGitHubError("rate limited", retry_after=3.0),
        ]
        with self.assertRaises(MODULE.PublicationError):
            self.publish()
        self.assertEqual(len(self.git.pushes), MODULE.MAX_ATTEMPTS)
        self.assertEqual(self.sleeps, [1.0, 2.0])
        self.assertTrue(all("force" not in call[1] for call in self.git.pushes))

    def test_ambiguous_push_reconciles_exact_head_before_any_retry(self) -> None:
        self.git.push_plan = [MODULE.AmbiguousMutation("connection reset", retry_after=7.0)]
        self.git.remote_plan = [SHA]
        result = self.publish()
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(len(self.git.pushes), 1)
        self.assertEqual(self.sleeps, [])

    def test_remote_divergence_blocks_without_force_update(self) -> None:
        self.git.push_plan = [MODULE.AmbiguousMutation("connection reset")]
        self.git.remote_plan = ["b" * 40]
        with self.assertRaises(MODULE.PublicationError):
            self.publish()
        self.assertEqual(len(self.git.pushes), 1)
        self.assertEqual(self.sleeps, [])

    def test_ambiguous_pr_creation_reconciles_exact_match(self) -> None:
        self.github.create_plan = [
            MODULE.AmbiguousMutation("connection reset", retry_after=5.0)
        ]
        self.github.matches = []

        def find_after_ambiguity(_repository: str, *, head: str, base: str) -> list[dict[str, object]]:
            self.github.find_count += 1
            if self.github.find_count >= 3:
                return [pull_request()]
            return []

        self.github.find_pull_requests = find_after_ambiguity  # type: ignore[method-assign]
        result = self.publish()
        self.assertEqual(result["pr_url"], "https://github.com/acme/project/pull/7")
        self.assertEqual(len(self.github.created), 1)
        self.assertEqual(self.sleeps, [])

    def test_restart_reuses_immutable_identity_and_does_not_merge(self) -> None:
        self.state.write(
            {
                **request(),
                "phase": "pushed",
                "head_sha": SHA,
                "pr_url": "",
            }
        )
        self.github.matches = [pull_request()]
        result = self.publish()
        self.assertEqual(result["phase"], "complete")
        self.assertEqual(self.github.created, [])
        self.assertEqual(self.github.merge_calls, [])

        with self.assertRaises(MODULE.PublicationError):
            self.publish(title="different title")

    def test_cancellation_marker_wins_before_any_provider_or_push_mutation(self) -> None:
        marker = self.cancel_root / "run-u5.json"
        marker.write_text('{"cancelled":true}\n', encoding="utf-8")
        with self.assertRaises(MODULE.CancelledPublication):
            self.publish()
        self.assertEqual(self.github.identity_calls, [])
        self.assertEqual(self.git.configured, [])
        self.assertEqual(self.git.pushes, [])
        self.assertEqual(self.github.created, [])

    def test_cancel_and_publish_race_stops_before_pull_request_creation(self) -> None:
        lookup_started = threading.Event()
        release_lookup = threading.Event()
        outcome: list[BaseException | dict[str, object]] = []

        original_lookup = self.github.find_pull_requests

        def blocked_lookup(
            repository: str,
            *,
            head: str,
            base: str,
        ) -> list[dict[str, object]]:
            result = original_lookup(repository, head=head, base=base)
            if self.github.find_count == 2:
                lookup_started.set()
                release_lookup.wait(timeout=5)
            return result

        self.github.find_pull_requests = blocked_lookup  # type: ignore[method-assign]

        def publish() -> None:
            try:
                outcome.append(self.publish())
            except BaseException as error:
                outcome.append(error)

        worker = threading.Thread(target=publish)
        worker.start()
        self.assertTrue(lookup_started.wait(timeout=5))

        def cancel() -> None:
            with MODULE._exclusive_lock(self.cancel_root / ".lock", mode=0o660):
                (self.cancel_root / "run-u5.json").write_text(
                    '{"cancelled":true}\n',
                    encoding="utf-8",
                )

        cancellation = threading.Thread(target=cancel)
        cancellation.start()
        cancellation.join(timeout=5)
        release_lookup.set()
        worker.join(timeout=5)
        self.assertEqual(len(outcome), 1)
        self.assertIsInstance(outcome[0], MODULE.CancelledPublication)
        self.assertEqual(self.github.created, [])

    def test_git_environment_disables_global_config_hooks_helpers_and_ssh(self) -> None:
        proxy = "http://127.0.0.1:3128"
        with patch.dict(
            os.environ,
            {
                "HTTP_PROXY": proxy,
                "HTTPS_PROXY": proxy,
                "http_proxy": proxy,
                "https_proxy": proxy,
            },
            clear=True,
        ):
            environment = MODULE.GitRunner.environment()
        self.assertEqual(environment["GIT_CONFIG_NOSYSTEM"], "1")
        self.assertEqual(environment["GIT_CONFIG_GLOBAL"], "/dev/null")
        self.assertEqual(environment["GIT_CONFIG_SYSTEM"], "/dev/null")
        self.assertEqual(environment["GIT_SSH_COMMAND"], "/bin/false")
        self.assertEqual(environment["GIT_PROXY_COMMAND"], "/bin/false")
        self.assertEqual(environment["GIT_ALLOW_PROTOCOL"], "https:file")
        self.assertEqual(environment["HTTP_PROXY"], proxy)
        self.assertEqual(environment["HTTPS_PROXY"], proxy)

    def test_github_api_and_git_configuration_preserve_the_egress_proxy(self) -> None:
        proxy = "http://127.0.0.1:3128"
        with patch.dict(
            os.environ,
            {
                "HTTP_PROXY": proxy,
                "HTTPS_PROXY": proxy,
            },
            clear=True,
        ):
            api = MODULE.GitHubAPI(
                app_id="7",
                installation_id="42",
                private_key_path="/dev/null",
            )
            opener = api.opener.__self__
        handlers = [
            handler
            for handler in opener.handlers
            if isinstance(handler, MODULE.urllib.request.ProxyHandler)
        ]
        self.assertEqual(len(handlers), 1)
        self.assertEqual(handlers[0].proxies, {"http": proxy, "https": proxy})
        source = SCRIPT.read_text()
        self.assertNotIn('"http.proxy": ""', source)
        self.assertNotIn('"https.proxy": ""', source)
        self.assertNotIn('"http.proxy="', source)
        self.assertNotIn('"https.proxy="', source)
        self.assertIn("--unset-all", source)
        service = (ROOT / "nixos-modules/gas-city-contributor/service.nix").read_text()
        self.assertNotIn("PrivateNetwork = false", service)
        self.assertGreaterEqual(service.count("fdproxy-sidecar"), 2)
        self.assertIn("HTTPS_PROXY=http://127.0.0.1:3128", service)
        self.assertIn("gascity-egress-channel", service)

    def test_github_api_signs_jwt_with_packaged_openssl_under_restricted_path(self) -> None:
        openssl = os.environ.get("GC_TEST_OPENSSL") or shutil.which("openssl")
        if openssl is None:
            self.skipTest("openssl is unavailable")
        try:
            validated_openssl = MODULE._validate_openssl_path(openssl)
        except MODULE.PublicationError as error:
            self.skipTest(f"openssl is not an immutable executable fixture: {error}")

        captured: dict[str, object] = {}
        real_run = MODULE.subprocess.run

        def capture_run(*args: object, **kwargs: object) -> object:
            captured["environment"] = dict(kwargs["env"])
            return real_run(*args, **kwargs)

        with patch.dict(os.environ, {"PATH": "/definitely-not-a-command-path"}, clear=False):
            with patch.object(MODULE.subprocess, "run", side_effect=capture_run):
                api = MODULE.GitHubAPI(
                    app_id="7",
                    installation_id="42",
                    private_key_path=str(FIXTURE_PRIVATE_KEY),
                    openssl=openssl,
                )
                token = api._jwt()

        header, claims, signature = token.split(".")
        decode = lambda value: MODULE.base64.urlsafe_b64decode(
            value + "=" * (-len(value) % 4)
        )
        self.assertEqual(
            MODULE.json.loads(decode(header)),
            {"alg": "RS256", "typ": "JWT"},
        )
        self.assertIn("iss", MODULE.json.loads(decode(claims)))
        self.assertGreater(len(decode(signature)), 0)
        environment = captured["environment"]
        self.assertIsInstance(environment, dict)
        self.assertEqual(environment["PATH"], str(pathlib.Path(validated_openssl).parent))
        self.assertNotIn("GITHUB_TOKEN", environment)
        self.assertNotIn("GH_TOKEN", environment)

    def test_publisher_serve_requires_an_explicit_openssl_argument(self) -> None:
        with self.assertRaises(SystemExit):
            MODULE._parse_args(
                [
                    "serve",
                    "--socket",
                    "/run/gascity-publisher/publisher.sock",
                    "--credential",
                    str(FIXTURE_PRIVATE_KEY),
                    "--state-root",
                    "/var/lib/gascity-publisher",
                    "--repository",
                    REPOSITORY,
                    "--base-branch",
                    BASE,
                    "--app-id",
                    "7",
                    "--installation-id",
                    "42",
                ]
            )

    def test_openssl_rejects_untrusted_symlink_and_accepts_immutable_package_link(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            target = root / "openssl"
            target.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            target.chmod(0o755)
            link = root / "openssl-link"
            link.symlink_to(target)
            with self.assertRaises(MODULE.PublicationError):
                MODULE._validate_openssl_path(str(link))

        openssl = os.environ.get("GC_TEST_OPENSSL") or shutil.which("openssl")
        if openssl is None:
            self.skipTest("openssl is unavailable")
        validated = MODULE._validate_openssl_path(openssl)
        self.assertTrue(pathlib.Path(validated).is_absolute())
        self.assertTrue(pathlib.Path(validated).is_file())
        self.assertTrue(os.access(validated, os.X_OK))

    def test_github_installation_identity_uses_app_jwt_without_installation_token(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            return FakeHTTPResponse(200, {"id": 42})

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
        )
        with patch.object(api, "_jwt", return_value="app-jwt") as jwt, patch.object(
            api,
            "installation_token",
            side_effect=AssertionError("installation authentication is not allowed"),
        ) as installation_token:
            result = api.installation_identity(REPOSITORY)

        self.assertEqual(result, {"id": 42})
        self.assertEqual(
            requests[0].full_url,
            "https://api.github.com/repos/acme/project/installation",
        )
        self.assertEqual(requests[0].headers["Authorization"], "Bearer app-jwt")
        jwt.assert_called_once_with()
        installation_token.assert_not_called()

    def test_github_installation_identity_retries_rate_limited_403_and_succeeds(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []
        outcomes: list[object] = [
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                403,
                "rate limited",
                {"Retry-After": "1.5"},
                None,
            ),
            FakeHTTPResponse(200, {"id": 42}),
        ]
        sleeps: list[float] = []

        def opener(
            request: MODULE.urllib.request.Request,
            timeout: float,
        ) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            outcome = outcomes.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            self.assertIsInstance(outcome, FakeHTTPResponse)
            return outcome

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
            sleep=sleeps.append,
        )
        with patch.object(api, "_jwt", return_value="app-jwt") as jwt, patch.object(
            api,
            "installation_token",
            side_effect=AssertionError("installation authentication is not allowed"),
        ) as installation_token:
            self.assertEqual(api.installation_identity(REPOSITORY), {"id": 42})

        self.assertEqual(len(requests), 2)
        self.assertEqual(sleeps, [1.5])
        self.assertTrue(
            all(
                request.headers["Authorization"].startswith("Bearer ")
                for request in requests
            )
        )
        self.assertEqual(jwt.call_count, 2)
        installation_token.assert_not_called()

    def test_github_installation_identity_rate_limited_403_retry_ceiling_is_bounded(self) -> None:
        outcomes: list[object] = [
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                403,
                "rate limited",
                {"Retry-After": "1"},
                None,
            ),
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                403,
                "rate limited",
                {"Retry-After": "2"},
                None,
            ),
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                403,
                "rate limited",
                {"Retry-After": "3"},
                None,
            ),
        ]
        requests: list[MODULE.urllib.request.Request] = []
        sleeps: list[float] = []

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> object:
            self.assertEqual(timeout, 20)
            requests.append(request)
            outcome = outcomes.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            return outcome

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
            sleep=sleeps.append,
        )
        with patch.object(api, "_jwt", return_value="app-jwt") as jwt, patch.object(
            api,
            "installation_token",
            side_effect=AssertionError("installation authentication is not allowed"),
        ) as installation_token:
            with self.assertRaisesRegex(
                MODULE.PublicationError,
                r"\AGitHub retry ceiling reached\Z",
            ):
                api.installation_identity(REPOSITORY)

        self.assertEqual(len(requests), MODULE.MAX_ATTEMPTS)
        self.assertEqual(sleeps, [1.0, 2.0])
        self.assertEqual(jwt.call_count, MODULE.MAX_ATTEMPTS)
        self.assertTrue(
            all(
                request.headers["Authorization"].startswith("Bearer ")
                for request in requests
            )
        )
        installation_token.assert_not_called()

    def test_github_installation_identity_401_does_not_refresh_installation_token(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            return FakeHTTPResponse(401, {"message": "Bad credentials"})

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
            sleep=self.sleeps.append,
        )
        with patch.object(api, "_jwt", return_value="app-jwt"), patch.object(
            api,
            "installation_token",
            side_effect=AssertionError("installation authentication is not allowed"),
        ) as installation_token:
            with self.assertRaisesRegex(
                MODULE.PublicationError,
                r"\AGitHub authentication failed\Z",
            ):
                api.installation_identity(REPOSITORY)

        self.assertEqual(len(requests), 1)
        self.assertEqual(requests[0].headers["Authorization"], "Bearer app-jwt")
        self.assertEqual(self.sleeps, [])
        installation_token.assert_not_called()

    def test_github_installation_identity_retries_transient_read_failures(self) -> None:
        transient_errors: tuple[BaseException, ...] = (
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                429,
                "rate limited",
                {"Retry-After": "1.5"},
                None,
            ),
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                503,
                "upstream",
                {"Retry-After": "2.5"},
                None,
            ),
            urllib.error.URLError("connection reset"),
        )

        for transient_error in transient_errors:
            with self.subTest(error=type(transient_error).__name__):
                requests: list[MODULE.urllib.request.Request] = []
                outcomes: list[object] = [
                    transient_error,
                    FakeHTTPResponse(200, {"id": 42}),
                ]
                sleeps: list[float] = []

                def opener(
                    request: MODULE.urllib.request.Request,
                    timeout: float,
                ) -> FakeHTTPResponse:
                    self.assertEqual(timeout, 20)
                    requests.append(request)
                    outcome = outcomes.pop(0)
                    if isinstance(outcome, BaseException):
                        raise outcome
                    self.assertIsInstance(outcome, FakeHTTPResponse)
                    return outcome

                api = MODULE.GitHubAPI(
                    app_id="7",
                    installation_id="42",
                    private_key_path="/dev/null",
                    opener=opener,
                    sleep=sleeps.append,
                )
                with patch.object(api, "_jwt", return_value="app-jwt"), patch.object(
                    api,
                    "installation_token",
                    side_effect=AssertionError(
                        "installation authentication is not allowed"
                    ),
                ) as installation_token:
                    self.assertEqual(
                        api.installation_identity(REPOSITORY),
                        {"id": 42},
                    )

                self.assertEqual(len(requests), 2)
                self.assertEqual(len(sleeps), 1)
                self.assertTrue(
                    all(
                        request.headers["Authorization"] == "Bearer app-jwt"
                        for request in requests
                    )
                )
                installation_token.assert_not_called()

    def test_github_installation_identity_retry_ceiling_is_bounded(self) -> None:
        outcomes: list[object] = [
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                429,
                "rate limited",
                {"Retry-After": "1"},
                None,
            ),
            urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/installation",
                503,
                "upstream",
                {"Retry-After": "2"},
                None,
            ),
            urllib.error.URLError("connection reset"),
        ]
        requests: list[MODULE.urllib.request.Request] = []

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> object:
            self.assertEqual(timeout, 20)
            requests.append(request)
            outcome = outcomes.pop(0)
            if isinstance(outcome, BaseException):
                raise outcome
            return outcome

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
            sleep=self.sleeps.append,
        )
        with patch.object(api, "_jwt", return_value="app-jwt"), patch.object(
            api,
            "installation_token",
            side_effect=AssertionError("installation authentication is not allowed"),
        ) as installation_token:
            with self.assertRaisesRegex(
                MODULE.PublicationError,
                r"\AGitHub read retry ceiling reached\Z",
            ):
                api.installation_identity(REPOSITORY)

        self.assertEqual(len(requests), MODULE.MAX_ATTEMPTS)
        self.assertEqual(self.sleeps, [1.0, 2.0])
        self.assertTrue(
            all(
                request.headers["Authorization"] == "Bearer app-jwt"
                for request in requests
            )
        )
        installation_token.assert_not_called()

    def test_repository_and_pull_reads_keep_installation_token_authentication(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            if request.full_url.endswith("/repos/acme/project"):
                return FakeHTTPResponse(200, {"full_name": REPOSITORY})
            self.assertIn("/repos/acme/project/pulls?", request.full_url)
            return FakeHTTPResponse(200, [pull_request()])

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
        )
        with patch.object(
            api,
            "_jwt",
            side_effect=AssertionError("repository reads must not use App JWT"),
        ), patch.object(
            api,
            "installation_token",
            return_value="installation-token",
        ) as installation_token:
            self.assertEqual(
                api.repository_identity(REPOSITORY),
                {"full_name": REPOSITORY},
            )
            self.assertEqual(
                api.find_pull_requests(REPOSITORY, head=HEAD, base=BASE),
                [pull_request()],
            )

        self.assertEqual(installation_token.call_count, 2)
        self.assertEqual(
            [request.headers["Authorization"] for request in requests],
            ["token installation-token", "token installation-token"],
        )

    def test_github_rate_limit_permanent_and_ambiguous_responses_are_bounded(self) -> None:
        def opener(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project",
                429,
                "rate limited",
                {"Retry-After": "1.75"},
                None,
            )

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
            sleep=lambda _seconds: None,
        )
        with self.assertRaises(MODULE.RetryableGitHubError) as rate_error:
            api._request_once("GET", "/repos/acme/project")
        self.assertEqual(rate_error.exception.retry_after, 1.75)

        permanent_requests: list[MODULE.urllib.request.Request] = []
        permanent_sleeps: list[float] = []

        def permanent(_request: object, timeout: float) -> object:
            permanent_requests.append(_request)
            raise urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project",
                403,
                "forbidden",
                {},
                None,
            )

        api.opener = permanent
        api.sleep = permanent_sleeps.append
        with patch.object(api, "_jwt", return_value="app-jwt"):
            with self.assertRaisesRegex(
                MODULE.PublicationError,
                r"\AGitHub returned permanent HTTP 403\Z",
            ):
                api._request_with_app_jwt("GET", "/repos/acme/project")
        self.assertEqual(len(permanent_requests), 1)
        self.assertEqual(permanent_sleeps, [])

        def rate_limited_mutation(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/pulls",
                403,
                "rate limited",
                {"Retry-After": "2"},
                None,
            )

        api.opener = rate_limited_mutation
        with self.assertRaises(MODULE.RetryableGitHubError) as mutation_rate_error:
            api._request_once(
                "POST",
                "/repos/acme/project/pulls",
                payload={"head": HEAD, "base": BASE},
                mutating=True,
            )
        self.assertEqual(mutation_rate_error.exception.retry_after, 2.0)

        def ambiguous(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project/pulls",
                502,
                "upstream",
                {"Retry-After": "4"},
                None,
            )

        api.opener = ambiguous
        with self.assertRaises(MODULE.AmbiguousMutation) as ambiguous_error:
            api._request_once(
                "POST",
                "/repos/acme/project/pulls",
                payload={"head": HEAD, "base": BASE},
                mutating=True,
            )
        self.assertEqual(ambiguous_error.exception.retry_after, 4.0)

    def test_github_rate_limited_403_uses_bounded_reset_hint(self) -> None:
        def opener(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project",
                403,
                "rate limited",
                {
                    "X-RateLimit-Remaining": "0",
                    "X-RateLimit-Reset": "450",
                },
                None,
            )

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
        )
        with patch.object(MODULE.time, "time", return_value=100.0):
            with self.assertRaises(MODULE.RetryableGitHubError) as rate_error:
                api._request_once("GET", "/repos/acme/project")
        self.assertEqual(rate_error.exception.retry_after, MODULE.MAX_RETRY_AFTER_SECONDS)

    def test_github_installation_token_refreshes_before_expiration(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []
        responses = [
            FakeHTTPResponse(
                201,
                {
                    "token": "first-token",
                    "expires_at": "1970-01-02T00:00:00Z",
                },
            ),
            FakeHTTPResponse(
                201,
                {
                    "token": "second-token",
                    "expires_at": "1970-01-03T00:00:00Z",
                },
            ),
        ]

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            return responses.pop(0)

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
        )
        with patch.object(api, "_jwt", side_effect=["jwt-1", "jwt-2"]), patch.object(
            MODULE.time,
            "time",
            side_effect=[0.0, 86_350.0],
        ):
            self.assertEqual(api.installation_token(), "first-token")
            self.assertEqual(api.installation_token(), "second-token")
        self.assertEqual(len(requests), 2)
        self.assertEqual(api._installation_token_expires_at, 172_800.0)

    def test_github_auth_failure_forces_one_refresh_and_retries_once(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []
        responses = [
            FakeHTTPResponse(
                201,
                {
                    "token": "first-token",
                    "expires_at": "2099-01-02T00:00:00Z",
                },
            ),
            FakeHTTPResponse(401, {"message": "Bad credentials"}),
            FakeHTTPResponse(
                201,
                {
                    "token": "second-token",
                    "expires_at": "2099-01-03T00:00:00Z",
                },
            ),
            FakeHTTPResponse(200, {"full_name": REPOSITORY}),
        ]

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            return responses.pop(0)

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
        )
        with patch.object(api, "_jwt", side_effect=["jwt-1", "jwt-2"]):
            result = api.request("GET", f"/repos/{REPOSITORY}")
        self.assertEqual(result, {"full_name": REPOSITORY})
        self.assertEqual(len(requests), 4)
        self.assertEqual(requests[1].headers["Authorization"], "token first-token")
        self.assertEqual(requests[3].headers["Authorization"], "token second-token")

    def test_github_auth_failure_does_not_force_refresh_again(self) -> None:
        requests: list[MODULE.urllib.request.Request] = []
        responses = [
            FakeHTTPResponse(
                201,
                {
                    "token": "first-token",
                    "expires_at": "2099-01-02T00:00:00Z",
                },
            ),
            FakeHTTPResponse(401, {}),
            FakeHTTPResponse(
                201,
                {
                    "token": "second-token",
                    "expires_at": "2099-01-03T00:00:00Z",
                },
            ),
            FakeHTTPResponse(401, {}),
        ]

        def opener(request: MODULE.urllib.request.Request, timeout: float) -> FakeHTTPResponse:
            self.assertEqual(timeout, 20)
            requests.append(request)
            return responses.pop(0)

        api = MODULE.GitHubAPI(
            app_id="7",
            installation_id="42",
            private_key_path="/dev/null",
            opener=opener,
        )
        with patch.object(api, "_jwt", side_effect=["jwt-1", "jwt-2"]):
            with self.assertRaises(MODULE.PublicationError):
                api.request("GET", f"/repos/{REPOSITORY}")
        self.assertEqual(len(requests), 4)
        self.assertEqual(
            sum(request.headers.get("Authorization", "").startswith("Bearer ") for request in requests),
            2,
        )

    def test_publisher_rpc_timeout_covers_the_bounded_provider_budget(self) -> None:
        class FakeSocket:
            def __init__(self) -> None:
                self.timeout: float | None = None
                self.closed = False

            def settimeout(self, timeout: float) -> None:
                self.timeout = timeout

            def connect(self, _path: str) -> None:
                return None

            def sendmsg(self, buffers: list[bytes], _ancillary: list[object]) -> int:
                return len(buffers[0])

            def recv(self, _limit: int) -> bytes:
                return b'{"ok":true,"result":{}}\n'

            def sendall(self, _payload: bytes) -> None:
                return None

            def close(self) -> None:
                self.closed = True

        connection = FakeSocket()
        with tempfile.TemporaryFile() as bundle:
            with patch.object(MODULE.socket, "socket", return_value=connection):
                result = MODULE._rpc_with_fd(
                    "/run/gascity-publisher/publisher.sock",
                    {"operation": "publish"},
                    bundle.fileno(),
                )
        self.assertEqual(result, {"ok": True, "result": {}})
        self.assertEqual(connection.timeout, MODULE.RPC_TIMEOUT_SECONDS)
        self.assertGreaterEqual(
            MODULE.RPC_TIMEOUT_SECONDS,
            MODULE.PUBLISHER_OPERATION_BUDGET_SECONDS,
        )
        self.assertTrue(connection.closed)

    def test_real_unlinked_bundle_imports_one_exact_head_into_bare_clone(self) -> None:
        git = shutil.which("git")
        if git is None:
            self.skipTest("git is unavailable")
        root = pathlib.Path(self.temporary.name, "real")
        remote = root / "remote.git"
        worktree = root / "worktree"
        publisher_repo = root / "publisher.git"
        remote.parent.mkdir(parents=True)
        run = lambda *arguments: subprocess.run(
            [git, *arguments],
            cwd=worktree if worktree.exists() else None,
            check=True,
            capture_output=True,
            text=True,
        )
        run("init", "--bare", str(remote))
        run("init", str(worktree))
        run("-C", str(worktree), "config", "user.email", "fixture@example.invalid")
        run("-C", str(worktree), "config", "user.name", "Fixture")
        (worktree / "README").write_text("base\n", encoding="utf-8")
        run("-C", str(worktree), "add", "README")
        run("-C", str(worktree), "commit", "-m", "base")
        run("-C", str(worktree), "branch", "-M", BASE)
        run("-C", str(worktree), "remote", "add", "origin", str(remote))
        run("-C", str(worktree), "push", "origin", BASE)
        run("-C", str(worktree), "checkout", "-b", HEAD)
        (worktree / "README").write_text("feature\n", encoding="utf-8")
        run("-C", str(worktree), "commit", "-am", "feature")
        bundle, head_sha = MODULE.create_unlinked_bundle(
            worktree=str(worktree),
            head=HEAD,
            base=BASE,
            branch_namespace="gascity/",
            git=git,
        )
        self.addCleanup(bundle.close)
        runner = MODULE.GitRunner()
        runner.configure_bare(publisher_repo, "https://github.com/acme/project.git")
        imported = runner.import_bundle(
            publisher_repo,
            bundle.fileno(),
            head=HEAD,
            expected_sha=head_sha,
        )
        self.assertEqual(imported, head_sha)
        heads = runner.checked(
            ["-C", str(publisher_repo), "show-ref", f"refs/heads/{HEAD}"]
        )
        self.assertIn(head_sha, heads)


if __name__ == "__main__":
    unittest.main()
