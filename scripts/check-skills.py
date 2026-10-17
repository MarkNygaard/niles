#!/usr/bin/env python3
"""Fail when .agents/skills and .claude/skills differ.

The skills live in .agents/skills, where Codex and Hermes look, and a
copy sits in .claude/skills, where Claude Code looks. The repository
has no symlinks (Windows checkouts run with core.symlinks off), so this
keeps the copy honest. Files are compared by the blob hash git would
store for them, which is the same for a CRLF checkout on Windows and an
LF one on Linux.

    python scripts/check-skills.py
"""

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CANONICAL = ".agents/skills"
COPY = ".claude/skills"


def blobs(base: str) -> dict[str, str]:
    files = sorted(p for p in (ROOT / base).rglob("*") if p.is_file())
    if not files:
        return {}
    paths = [p.relative_to(ROOT).as_posix() for p in files]
    hashes = subprocess.run(
        ["git", "hash-object", "--stdin-paths"],
        input="\n".join(paths) + "\n",
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    return {p.relative_to(ROOT / base).as_posix(): h for p, h in zip(files, hashes)}


def main() -> int:
    canonical = blobs(CANONICAL)
    copy = blobs(COPY)
    if not canonical:
        print(f"{CANONICAL} is empty or missing", file=sys.stderr)
        return 1
    problems = []
    for path in sorted(canonical.keys() | copy.keys()):
        if path not in copy:
            problems.append(f"only in {CANONICAL}: {path}")
        elif path not in canonical:
            problems.append(f"only in {COPY}: {path}")
        elif canonical[path] != copy[path]:
            problems.append(f"differs: {path}")
    if problems:
        print(f"{COPY} is not a copy of {CANONICAL}:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(f"Edit {CANONICAL}, then copy it over {COPY}.", file=sys.stderr)
        return 1
    print(f"{COPY} matches {CANONICAL} ({len(canonical)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
