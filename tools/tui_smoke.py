#!/usr/bin/env python3
"""Run synthetic TUI checks: python3 tools/tui_smoke.py [--binary PATH]."""

import argparse
from pathlib import Path

from mock_subsonic import Fixture
from tui_harness import Tui, baseline
from tui_mix import instant_mix
from tui_gallery import gallery


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path,
                        default=Path(__file__).resolve().parents[1] / "target/debug/ratune")
    parser.add_argument("--scenario", choices=["baseline", "mix", "gallery"], default="baseline")
    args = parser.parse_args()
    with Fixture(gallery=args.scenario == "gallery") as fixture, Tui(args.binary, fixture) as tui:
        {"baseline": baseline, "mix": instant_mix, "gallery": gallery}[args.scenario](tui)
    print(f"PASS: isolated {args.scenario} TUI scenario")
