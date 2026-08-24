#!/usr/bin/env python3
"""Generate a deterministic Cargo dependency notice inventory."""

import argparse
import json
import pathlib
import subprocess


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default="target/notices/THIRD-PARTY-NOTICES.txt")
    args = parser.parse_args()

    root = pathlib.Path(__file__).resolve().parents[1]
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            cwd=root,
            text=True,
        )
    )
    workspace = set(metadata["workspace_members"])
    packages = sorted(
        (package for package in metadata["packages"] if package["id"] not in workspace),
        key=lambda package: (package["name"].casefold(), package["version"], package["id"]),
    )

    lines = [
        "eiviz third-party dependency notices",
        "====================================",
        "",
        "Generated from Cargo.lock. License expressions are package metadata;",
        "the corresponding license texts and upstream terms remain authoritative.",
        "See NOTICE for native SDK and codec distribution restrictions.",
        "",
    ]
    for package in packages:
        source = package.get("repository") or package.get("source") or "not declared"
        lines.extend(
            [
                f"{package['name']} {package['version']}",
                f"  License: {package.get('license') or 'NOT DECLARED'}",
                f"  Source: {source}",
            ]
        )
        if package.get("license_file"):
            lines.append(f"  License file: {package['license_file']}")
        lines.append("")

    output = root / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    print(output)


if __name__ == "__main__":
    main()
