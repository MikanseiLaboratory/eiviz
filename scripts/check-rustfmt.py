#!/usr/bin/env python3
"""Checks that every workspace crate is rustfmt-clean.

CI formats each crate with its own `cargo fmt -p ...` list, so a crate that is missing
from that list can drift for a long time without anything noticing: the replay work left
`mixer` and `eiviz-pro-api` unformatted while CI stayed green. This asks rustfmt itself
which files it would change (`cargo fmt --all --check`), so the answer cannot fall behind
the crate list in `Cargo.toml`.
"""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    result = subprocess.run(
        ["cargo", "fmt", "--all", "--", "--check"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode == 0:
        print("every workspace crate is rustfmt-clean")
        return 0
    # rustfmt prints one `Diff in <path>:<line>` header per hunk. Name each file once,
    # so the failure reads as a list of crates to fix rather than a wall of diff.
    files = []
    for line in result.stdout.splitlines():
        if not line.startswith("Diff in "):
            continue
        location = line[len("Diff in ") :].removeprefix("\\\\?\\")
        path = location.rsplit(":", 1)[0]
        if path not in files:
            files.append(path)
    print("crates rustfmt would rewrite:", file=sys.stderr)
    for path in files:
        print(f"  {path}", file=sys.stderr)
    print("run `cargo fmt --all` to fix", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
