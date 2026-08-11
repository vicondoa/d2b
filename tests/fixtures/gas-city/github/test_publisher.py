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
) -> dict[str, object]:
    return {
        "number": number,
        "state": state,
        "merged": merged,
        "html_url": f"https://github.com/{repository}/pull/{number}",
        "head": {"ref": HEAD, "sha": sha, "repo": {"full_name": repository}},
        "base": {"ref": BASE, "repo": {"full_name": repository}},
    }


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
        self.github.matches = [pull_request(state="closed", merged=True)]
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

        def permanent(_request: object, timeout: float) -> object:
            raise urllib.error.HTTPError(
                "https://api.github.com/repos/acme/project",
                403,
                "forbidden",
                {},
                None,
            )

        api.opener = permanent
        with self.assertRaises(MODULE.PublicationError):
            api._request_once("GET", "/repos/acme/project")

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
        heads = subprocess.run(
            [git, "-C", str(publisher_repo), "show-ref", f"refs/heads/{HEAD}"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertIn(head_sha, heads)


if __name__ == "__main__":
    unittest.main()
