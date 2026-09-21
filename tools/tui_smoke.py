#!/usr/bin/env python3
"""Run synthetic TUI checks: python3 tools/tui_smoke.py [--binary PATH]."""

import argparse
from pathlib import Path

from mock_subsonic import Fixture
from tui_harness import Tui, baseline


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path,
                        default=Path(__file__).resolve().parents[1] / "target/debug/ratune")
    args = parser.parse_args()
    with Fixture() as fixture, Tui(args.binary, fixture) as tui:
        baseline(tui)
    print("PASS: isolated startup, navigation, queue restoration and clean exit")
