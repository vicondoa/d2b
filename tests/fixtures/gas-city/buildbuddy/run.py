#!/usr/bin/env python3
"""Run the hermetic BuildBuddy fixture suite."""

from __future__ import annotations

import pathlib
import unittest


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.discover(
        str(pathlib.Path(__file__).parent),
        pattern="test_*.py",
    )
    raise SystemExit(0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1)
