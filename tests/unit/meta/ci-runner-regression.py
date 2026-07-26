#!/usr/bin/env python3
"""Regression coverage for the CI shell and local Layer-1 runner."""

from __future__ import annotations

import importlib.util
import os
import pathlib
import shlex
import shutil
import subprocess
import tempfile
import types
import unittest


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

if __name__ == "__main__":
    unittest.main()
