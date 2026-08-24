#!/usr/bin/env python3
"""Verify native release signatures using platform trust tooling."""

import argparse
import hashlib
import json
import pathlib
import subprocess


def run(command: list[str]) -> None:
    subprocess.run(command, check=True)


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--target",
        choices=["windows-x86_64", "macos-aarch64", "linux-x86_64"],
        required=True,
    )
    parser.add_argument("--artifact", type=pathlib.Path, required=True)
    parser.add_argument("--manifest", type=pathlib.Path, required=True)
    parser.add_argument("--signature", type=pathlib.Path)
    parser.add_argument("--gpg-keyring", type=pathlib.Path)
    parser.add_argument("--assess-notarization", action="store_true")
    args = parser.parse_args()

    artifact = args.artifact.resolve(strict=True)
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    if manifest.get("signed") is not True:
        raise ValueError("release manifest does not declare a signed artifact")
    if manifest["artifact"]["name"] != artifact.name:
        raise ValueError("release manifest artifact name mismatch")
    if (
        manifest["artifact"]["size"] != artifact.stat().st_size
        or manifest["artifact"]["sha256"] != sha256(artifact)
    ):
        raise ValueError("release manifest artifact hash mismatch")
    if args.signature:
        signature = args.signature.resolve(strict=True)
        declared = manifest.get("signature", {})
        if (
            declared.get("name") != signature.name
            or declared.get("size") != signature.stat().st_size
            or declared.get("sha256") != sha256(signature)
        ):
            raise ValueError("release manifest signature hash mismatch")

    if args.target == "windows-x86_64":
        run(["signtool", "verify", "/pa", "/all", "/v", str(artifact)])
    elif args.target == "macos-aarch64":
        run(["pkgutil", "--check-signature", str(artifact)])
        if args.assess_notarization:
            run(["spctl", "--assess", "--verbose=2", "--type", "install", str(artifact)])
    else:
        if not args.signature:
            parser.error("Linux verification requires --signature")
        command = ["gpg", "--batch"]
        if args.gpg_keyring:
            command.extend(["--no-default-keyring", "--keyring", str(args.gpg_keyring)])
        command.extend(["--verify", str(signature), str(artifact)])
        run(command)
    print("signature verification passed")


if __name__ == "__main__":
    main()
