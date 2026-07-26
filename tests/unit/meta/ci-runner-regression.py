#!/usr/bin/env python3
"""Regression coverage for the CI shell and local Layer-1 runner."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import os
import pathlib
import shlex
import shutil
import subprocess
import tempfile
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
SCRATCH = ROOT / ".scratch"
LAYER1_JOBS = ROOT / "tests" / "tools" / "layer1-jobs.py"


def load_layer1_jobs() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("d2b_layer1_jobs", LAYER1_JOBS)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load the Layer-1 runner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CiRunnerRegressionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        SCRATCH.mkdir(exist_ok=True)

    def setUp(self) -> None:
        self.scratch = pathlib.Path(
            tempfile.mkdtemp(prefix="ci-runner-regression.", dir=SCRATCH)
        )

    def tearDown(self) -> None:
        shutil.rmtree(self.scratch)

    def test_shell_bootstrap_rejects_bash_startup_poison(self) -> None:
        layer1_jobs = load_layer1_jobs()
        bootstrap = shlex.split(layer1_jobs.SCRUBBED_BASH)
        self.assertEqual(bootstrap[-1], "{0}")

        poison_marker = self.scratch / "bash-env-reached-bootstrap"
        step_marker = self.scratch / "step-ran"
        bash_env = self.scratch / "bash-env"
        step = self.scratch / "step.sh"
        bash_env.write_text(
            ': > "${D2B_POISON_MARKER:?}"\n',
            encoding="utf-8",
        )
        step.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
[ -z "${BASH_ENV+x}" ]
! declare -F poisoned_helper >/dev/null
! declare -F dirname >/dev/null
: > "${D2B_STEP_MARKER:?}"
""",
            encoding="utf-8",
        )

        env = os.environ.copy()
        env.update(
            {
                "BASH_ENV": str(bash_env),
                "BASH_FUNC_poisoned_helper%%": "() { return 41; }",
                "BASH_FUNC_dirname%%": (
                    "() { printf '%s\\n' '/corrupted-bootstrap'; }"
                ),
                "D2B_POISON_MARKER": str(poison_marker),
                "D2B_STEP_MARKER": str(step_marker),
            }
        )
        command = [*bootstrap[:-1], str(step)]
        proc = subprocess.run(
            command,
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )

        failures = []
        if poison_marker.exists():
            failures.append("BASH_ENV was sourced before the environment scrub")
        if proc.returncode != 0:
            failures.append(
                f"scrubbed shell exited {proc.returncode}; stderr:\n{proc.stderr}"
            )
        if not step_marker.exists():
            failures.append("the protected step did not run")
        self.assertEqual(
            failures,
            [],
            msg="\n".join(failures),
        )

    def test_extra_target_failure_is_identified_and_redacted(self) -> None:
        layer1_jobs = load_layer1_jobs()
        log_dir = self.scratch / "retained-log"
        calls: list[str] = []

        def make_log_dir(*_args: object, **_kwargs: object) -> str:
            log_dir.mkdir()
            return str(log_dir)

        def run_make(
            argv: list[str],
            **kwargs: object,
        ) -> types.SimpleNamespace:
            target = argv[1]
            calls.append(target)
            output = kwargs["stdout"]
            if target == "check-tier0":
                return types.SimpleNamespace(returncode=0)
            output.write(
                (
                    f"repo failure: {ROOT}/private/output\n"
                    f"home failure: {pathlib.Path.home()}/private/output\n"
                    "system failure: /nix/store/private-output\n"
                ).encode()
            )
            return types.SimpleNamespace(returncode=37)

        job = {
            "displayName": "Tier 0 first-pass gate",
            "makeTarget": "check-tier0",
            "extraMakeTargets": [
                {
                    "makeTarget": "test-ci-coverage",
                    "displayName": "CI coverage structural guard",
                }
            ],
        }
        stdout = io.StringIO()
        stderr = io.StringIO()
        with (
            mock.patch.object(layer1_jobs.tempfile, "mkdtemp", make_log_dir),
            mock.patch.object(layer1_jobs.subprocess, "run", run_make),
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            returncode = layer1_jobs.run_job("tier0", job)

        diagnostic = stderr.getvalue()
        self.assertEqual(returncode, 37)
        self.assertEqual(calls, ["check-tier0", "test-ci-coverage"])
        self.assertIn("FAIL: test-ci-coverage", diagnostic)
        self.assertIn("Layer-1 job tier0", diagnostic)
        self.assertNotIn(str(log_dir), diagnostic)
        self.assertNotIn(str(ROOT), diagnostic)
        self.assertNotIn(str(pathlib.Path.home()), diagnostic)
        self.assertNotIn("/nix/store", diagnostic)
        self.assertIn("<repo>/private/output", diagnostic)
        self.assertIn("<home>/private/output", diagnostic)
        self.assertIn("<path>", diagnostic)


if __name__ == "__main__":
    unittest.main()
