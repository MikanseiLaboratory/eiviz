#!/usr/bin/env python3
"""Generate host tables from checked-in preset data; --check verifies outputs."""

import argparse
import csv
import json
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
DATA = "hosts/shared/stream-platforms.tsv"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    with (ROOT / DATA).open(encoding="utf-8", newline="") as source:
        presets = list(csv.reader(source, delimiter="\t"))
    for line, row in enumerate(presets, 1):
        if len(row) != 2 or not all(row):
            raise SystemExit(f"{DATA}:{line}: expected name and URL")
        name, url = row
        if any(ord(char) < 32 for char in name + url):
            raise SystemExit(f"{DATA}:{line}: control characters are not allowed")
        parsed = urlsplit(url)
        if parsed.scheme not in ("rtmp", "rtmps") or not parsed.hostname:
            raise SystemExit(f"{DATA}:{line}: expected an RTMP(S) ingest URL")
    names = [name.casefold() for name, _ in presets]
    if len(set(names)) != len(names):
        raise SystemExit("Duplicate service names")
    if names[:2] != ["youtube", "twitch"] or names[2:] != sorted(names[2:]):
        raise SystemExit("Expected YouTube, Twitch, then services sorted by name")

    def quote(value):
        return json.dumps(value, ensure_ascii=False)

    header = f"// Generated from {DATA} by scripts/generate-stream-platform-presets.py; do not edit.\n"
    cs = header + "namespace Eiviz.Host.Dialogs;\n\ninternal static class StreamPlatformPresets\n{\n"
    cs += "    internal static readonly (string Name, string Url)[] All =\n    [\n"
    cs += "".join(f"        ({quote(name)}, {quote(url)}),\n" for name, url in presets)
    cs += "    ];\n}\n"
    swift = header + "enum StreamPlatformPresets {\n"
    swift += "    static let all: [(name: String, url: String)] = [\n"
    swift += "".join(f"        ({quote(name)}, {quote(url)}),\n" for name, url in presets)
    swift += "    ]\n}\n"
    outputs = {
        ROOT / "hosts/win32/Dialogs/StreamPlatformPresets.g.cs": cs,
        ROOT / "hosts/macos/Sources/EivizMac/StreamPlatformPresets.swift": swift,
    }
    for path, content in outputs.items():
        if args.check:
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                raise SystemExit(f"Out of date: {path.relative_to(ROOT)}")
        else:
            path.write_text(content, encoding="utf-8", newline="\n")
    print(f"{len(presets)} stream platform presets")


if __name__ == "__main__":
    main()
