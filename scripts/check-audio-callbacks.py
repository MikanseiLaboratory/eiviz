#!/usr/bin/env python3
"""Reject allocation, blocking, I/O, logging, and panic primitives in callbacks."""

from pathlib import Path
import re
import sys

SOURCE = Path("crates/eiviz-io-audio/src/native.rs")
START = re.compile(r"realtime-callback-start: ([a-z0-9-]+)")
END = re.compile(r"realtime-callback-end: ([a-z0-9-]+)")
FORBIDDEN = {
    ".lock(": "blocking lock",
    "Mutex": "mutex",
    "Vec": "heap vector",
    "String": "heap string",
    "format!": "format allocation",
    "Box": "heap box",
    "std::fs": "file I/O",
    "std::net": "network I/O",
    "sleep(": "blocking sleep",
    "tracing::": "synchronous logging",
    "println!": "synchronous logging",
    "eprintln!": "synchronous logging",
    "panic!": "panic",
    ".expect(": "panic",
    ".unwrap(": "panic",
}


def callback_blocks(source: str) -> list[tuple[str, int, str]]:
    active: tuple[str, int] | None = None
    blocks: list[tuple[str, int, str]] = []
    lines = source.splitlines()
    for line_number, line in enumerate(lines, 1):
        start = START.search(line)
        end = END.search(line)
        if start:
            if active is not None:
                raise ValueError(f"nested callback marker at line {line_number}")
            active = (start.group(1), line_number)
        if end:
            if active is None or active[0] != end.group(1):
                raise ValueError(f"unmatched callback marker at line {line_number}")
            name, first = active
            blocks.append((name, first, "\n".join(lines[first:line_number - 1])))
            active = None
    if active is not None:
        raise ValueError(f"unterminated callback marker {active[0]!r}")
    return blocks


def main() -> int:
    source = SOURCE.read_text(encoding="utf-8")
    blocks = callback_blocks(source)
    if not blocks:
        print(f"{SOURCE}: no realtime callback markers", file=sys.stderr)
        return 1
    failures: list[str] = []
    for name, first, body in blocks:
        for token, reason in FORBIDDEN.items():
            if token in body:
                failures.append(
                    f"{SOURCE}:{first}: {name}: forbidden {reason} token {token!r}"
                )
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"checked {len(blocks)} realtime callback regions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
