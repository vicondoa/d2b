#!/usr/bin/env python3
"""Run the hermetic ACP fixture suite without repository-wide gate wiring."""

from __future__ import annotations

import pathlib
import sys
import unittest


def main() -> int:
    suite = unittest.defaultTestLoader.discover(
        str(pathlib.Path(__file__).parent),
        pattern="test_*.py",
    )
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
