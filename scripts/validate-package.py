#!/usr/bin/env python3
"""Validate packaging policy and smoke-test one native artifact."""

import argparse
import fnmatch
import hashlib
import json
import pathlib
import shutil
import subprocess
import tempfile
import tomllib
import zipfile


ROOT = pathlib.Path(__file__).resolve().parents[1]


def sha256(path: pathlib.Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate_policy() -> dict:
    config = json.loads((ROOT / "packaging/release.json").read_text(encoding="utf-8"))
    if config["schema_version"] != 1:
        raise ValueError("unsupported packaging schema")
    cargo = tomllib.loads(
        (ROOT / "apps/eiviz-desktop/Cargo.toml").read_text(encoding="utf-8")
    )
    available = set(cargo["features"])
    forbidden_features = {"ndi", "decklink", "audio-asio"}
    for target, profile in config["targets"].items():
        features = set(profile["features"])
        unknown = features - available
        if unknown:
            raise ValueError(f"{target} contains unknown Cargo features: {sorted(unknown)}")
        prohibited = features & forbidden_features
        if prohibited:
            raise ValueError(
                f"{target} silently enables separately licensed SDKs: {sorted(prohibited)}"
            )
    if config["signing"]["unsigned_ci"] is not True:
        raise ValueError("ordinary CI must be declared unsigned")
    workflow = ROOT / config["signing"]["workflow"]
    if not workflow.is_file():
        raise ValueError(f"missing signing workflow: {workflow}")
    return config


def verify_checksum(artifact: pathlib.Path) -> None:
    checksum = artifact.with_name(f"{artifact.name}.sha256")
    if not checksum.is_file():
        raise ValueError(f"missing checksum sidecar: {checksum}")
    values = checksum.read_text(encoding="utf-8").strip().split()
    if len(values) != 2 or values[1] != artifact.name or values[0] != sha256(artifact):
        raise ValueError("artifact checksum sidecar mismatch")


def extract(artifact: pathlib.Path, target: str, destination: pathlib.Path) -> None:
    if target == "windows-x86_64":
        with zipfile.ZipFile(artifact) as package:
            package.extractall(destination)
    elif target == "linux-x86_64":
        subprocess.run(["dpkg-deb", "-x", artifact, destination], check=True)
    else:
        subprocess.run(["pkgutil", "--expand-full", artifact, destination], check=True)


def locate(root: pathlib.Path, basename: str) -> pathlib.Path:
    matches = [path for path in root.rglob(basename) if path.is_file()]
    if len(matches) != 1:
        raise ValueError(f"expected one {basename}, found {len(matches)}")
    return matches[0]


def verify_payload_hashes(target: str, manifest_path: pathlib.Path, manifest: dict) -> None:
    if target == "windows-x86_64":
        payload_root = manifest_path.parent
    elif target == "macos-aarch64":
        payload_root = manifest_path.parents[2]
    else:
        payload_root = manifest_path.parents[3]
    for entry in manifest["files"]:
        path = payload_root / entry["path"]
        if not path.is_file():
            raise ValueError(f"manifest payload is missing: {entry['path']}")
        if path.stat().st_size != entry["size"] or sha256(path) != entry["sha256"]:
            raise ValueError(f"manifest payload hash mismatch: {entry['path']}")


def validate_artifact(config: dict, target: str, artifact: pathlib.Path) -> None:
    if not artifact.is_file() or artifact.stat().st_size == 0:
        raise ValueError(f"missing or empty package: {artifact}")
    if "unsigned" not in artifact.name:
        raise ValueError("CI smoke validation only accepts explicitly unsigned artifact names")
    verify_checksum(artifact)
    with tempfile.TemporaryDirectory(prefix="eiviz-package-") as temporary:
        expanded = pathlib.Path(temporary) / "expanded"
        extract(artifact, target, expanded)
        for basename in config["required_payloads"]:
            locate(expanded, basename)
        executable = "eiviz.exe" if target == "windows-x86_64" else "eiviz"
        locate(expanded, executable)
        migrator = (
            "eiviz-project-migrate.exe"
            if target == "windows-x86_64"
            else "eiviz-project-migrate"
        )
        locate(expanded, migrator)
        manifest_path = locate(expanded, "payload-manifest.json")
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        if manifest["target"] != target or manifest["signed"] is not False:
            raise ValueError("payload manifest target/signing state mismatch")
        if manifest["features"] != config["targets"][target]["features"]:
            raise ValueError("payload manifest feature profile mismatch")
        if manifest["optional_sdk_payloads"]:
            raise ValueError("optional SDK payload list must remain empty")
        verify_payload_hashes(target, manifest_path, manifest)
        for path in expanded.rglob("*"):
            if not path.is_file():
                continue
            for pattern in config["forbidden_bundle_globs"]:
                if fnmatch.fnmatch(path.name.casefold(), pattern.casefold()):
                    raise ValueError(f"forbidden optional SDK payload: {path}")
        if target == "windows-x86_64":
            locate(expanded, "AppxManifest.xml")
        elif target == "macos-aarch64":
            locate(expanded, "Info.plist")
        else:
            locate(expanded, "eiviz.desktop")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=["windows-x86_64", "macos-aarch64", "linux-x86_64"])
    parser.add_argument("--artifact", type=pathlib.Path)
    args = parser.parse_args()
    config = validate_policy()
    if bool(args.target) != bool(args.artifact):
        parser.error("--target and --artifact must be supplied together")
    if args.artifact:
        validate_artifact(config, args.target, args.artifact.resolve())
    print("packaging validation passed")


if __name__ == "__main__":
    main()
