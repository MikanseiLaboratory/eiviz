#!/usr/bin/env python3
"""Write deterministic external metadata after artifact signing."""

import argparse
import hashlib
import json
import os
import pathlib
import subprocess


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifact", type=pathlib.Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--signature", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()

    root = pathlib.Path(__file__).resolve().parents[1]
    artifact = args.artifact.resolve(strict=True)
    signature = args.signature.resolve(strict=True) if args.signature else None
    epoch = int(
        os.environ.get("SOURCE_DATE_EPOCH")
        or subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", "HEAD"], cwd=root, text=True
        ).strip()
    )
    value = {
        "schema_version": 1,
        "product": "eiviz",
        "target": args.target,
        "source_revision": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip(),
        "source_date_epoch": epoch,
        "signed": True,
        "artifact": {
            "name": artifact.name,
            "size": artifact.stat().st_size,
            "sha256": sha256(artifact),
        },
        "signature": (
            {
                "name": signature.name,
                "size": signature.stat().st_size,
                "sha256": sha256(signature),
            }
            if signature
            else {"embedded": True}
        ),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(args.output)


if __name__ == "__main__":
    main()
