#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""
Rebuild the realistic sample collection from end to end: the FLAC files first,
then the database that indexes them.

Either half can be run on its own — the audio only changes when the definition
gains or loses a track, while the database is worth rebuilding often, since the
play log is stored relative to the moment it is generated. This script is for
when you want both.

It has no dependencies of its own; the two scripts it calls declare theirs.
"""

import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
STEPS = ("generate_collection.py", "generate_database.py")


def generate_all() -> None:
    for step in STEPS:
        print(f"\n━━━ {step} ━━━\n")
        # Each step is a uv script in its own right, run through its shebang
        # exactly as it would be by hand.
        result = subprocess.run([str(SCRIPT_DIR / step)], check=False)
        if result.returncode != 0:
            sys.exit(f"\n{step} exited {result.returncode}; stopping.")


if __name__ == "__main__":
    generate_all()
