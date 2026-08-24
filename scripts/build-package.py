#!/usr/bin/env python3
"""Build the native unsigned package declared by packaging/release.json."""

import argparse
import fnmatch
import hashlib
import json
import os
import pathlib
import shutil
import struct
import subprocess
import xml.sax.saxutils
import zlib


ROOT = pathlib.Path(__file__).resolve().parents[1]
CONFIG = json.loads((ROOT / "packaging/release.json").read_text(encoding="utf-8"))


def digest(path: pathlib.Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def copy(source: pathlib.Path, destination: pathlib.Path, executable: bool = False) -> None:
    if not source.is_file():
        raise FileNotFoundError(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    destination.chmod(0o755 if executable else 0o644)


def png(path: pathlib.Path, width: int, height: int) -> None:
    """Write a deterministic RGBA icon without image-tool dependencies."""
    path.parent.mkdir(parents=True, exist_ok=True)
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            border = min(x, y, width - x - 1, height - y - 1)
            if border < max(1, width // 16):
                rows.extend((22, 28, 45, 255))
            else:
                rows.extend((36, 154, 166, 255))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(rows), level=9))
        + chunk(b"IEND", b"")
    )


def source_epoch() -> int:
    configured = os.environ.get("SOURCE_DATE_EPOCH")
    if configured:
        return int(configured)
    return int(
        subprocess.check_output(
            ["git", "show", "-s", "--format=%ct", "HEAD"], cwd=ROOT, text=True
        ).strip()
    )


def source_revision() -> str:
    return subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()


def normalize_tree(root: pathlib.Path, epoch: int) -> None:
    for path in sorted(root.rglob("*"), reverse=True):
        os.utime(path, (epoch, epoch), follow_symlinks=False)
    os.utime(root, (epoch, epoch), follow_symlinks=False)


def legal_payload(destination: pathlib.Path) -> None:
    copy(ROOT / "LICENSE", destination / "LICENSE")
    copy(ROOT / "NOTICE", destination / "NOTICE")
    copy(
        ROOT / "target/notices/THIRD-PARTY-NOTICES.txt",
        destination / "THIRD-PARTY-NOTICES.txt",
    )


def sbom_payload(destination: pathlib.Path) -> None:
    copy(ROOT / "target/sbom/eiviz.spdx.json", destination / "eiviz.spdx.json")
    copy(
        ROOT / "target/sbom/eiviz.cyclonedx.json",
        destination / "eiviz.cyclonedx.json",
    )


def payload_manifest(root: pathlib.Path, destination: pathlib.Path, target: str) -> None:
    files = []
    for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()):
        if path.is_file() and path != destination and "DEBIAN" not in path.parts:
            files.append(
                {
                    "path": path.relative_to(root).as_posix(),
                    "size": path.stat().st_size,
                    "sha256": digest(path),
                }
            )
    value = {
        "schema_version": 1,
        "product": CONFIG["product"]["name"],
        "version": workspace_version(),
        "target": target,
        "features": CONFIG["targets"][target]["features"],
        "source_revision": source_revision(),
        "source_date_epoch": source_epoch(),
        "signed": False,
        "state": "unsigned-build-input",
        "optional_sdk_payloads": [],
        "files": files,
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )


def assert_no_optional_sdks(root: pathlib.Path) -> None:
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        name = path.name.casefold()
        for pattern in CONFIG["forbidden_bundle_globs"]:
            if fnmatch.fnmatch(name, pattern.casefold()):
                raise ValueError(f"forbidden optional SDK payload: {path.relative_to(root)}")


def workspace_version() -> str:
    for line in (ROOT / "Cargo.toml").read_text(encoding="utf-8").splitlines():
        if line.startswith("version = "):
            return line.split('"', 2)[1]
    raise ValueError("workspace package version not found")


def four_part_version(version: str) -> str:
    values = version.split("-", 1)[0].split(".")
    if len(values) != 3 or not all(value.isdigit() for value in values):
        raise ValueError(f"MSIX requires a numeric semver version, found {version}")
    numbers = [int(value) for value in values] + [0]
    if any(value > 65535 for value in numbers):
        raise ValueError("MSIX version component exceeds 65535")
    return ".".join(str(value) for value in numbers)


def bundle_version(version: str) -> str:
    return ".".join(version.split("-", 1)[0].split("."))


def stage_windows(
    stage: pathlib.Path,
    binary: pathlib.Path,
    migrator: pathlib.Path,
    publisher: str,
) -> pathlib.Path:
    copy(binary, stage / "eiviz.exe", executable=True)
    copy(migrator, stage / "eiviz-project-migrate.exe", executable=True)
    legal_payload(stage / "Legal")
    sbom_payload(stage / "SBOM")
    copy(
        ROOT / "packaging/scripts/eiviz-data-migration.py",
        stage / "Support/eiviz-data-migration.py",
    )
    template = (ROOT / "packaging/windows/AppxManifest.xml.in").read_text(
        encoding="utf-8"
    )
    (stage / "AppxManifest.xml").write_text(
        template.replace("@VERSION@", four_part_version(workspace_version())).replace(
            "CN=eiviz unsigned test package", xml.sax.saxutils.escape(publisher)
        ),
        encoding="utf-8",
        newline="\n",
    )
    png(stage / "Assets/StoreLogo.png", 50, 50)
    png(stage / "Assets/Square44x44Logo.png", 44, 44)
    png(stage / "Assets/Square150x150Logo.png", 150, 150)
    payload_manifest(stage, stage / "payload-manifest.json", "windows-x86_64")
    return stage


def stage_macos(
    stage: pathlib.Path, binary: pathlib.Path, migrator: pathlib.Path
) -> pathlib.Path:
    app = stage / "eiviz.app"
    copy(binary, app / "Contents/MacOS/eiviz", executable=True)
    copy(
        migrator,
        app / "Contents/MacOS/eiviz-project-migrate",
        executable=True,
    )
    legal_payload(app / "Contents/Resources/Legal")
    sbom_payload(app / "Contents/Resources/SBOM")
    copy(
        ROOT / "packaging/scripts/eiviz-data-migration.py",
        app / "Contents/Resources/Support/eiviz-data-migration.py",
        executable=True,
    )
    template = (ROOT / "packaging/macos/Info.plist.in").read_text(encoding="utf-8")
    (app / "Contents/Info.plist").write_text(
        template.replace("@VERSION@", workspace_version()).replace(
            "@BUNDLE_VERSION@", bundle_version(workspace_version())
        ),
        encoding="utf-8",
        newline="\n",
    )
    payload_manifest(
        app,
        app / "Contents/Resources/payload-manifest.json",
        "macos-aarch64",
    )
    return app


def stage_linux(
    stage: pathlib.Path, binary: pathlib.Path, migrator: pathlib.Path
) -> pathlib.Path:
    package = stage / "root"
    copy(binary, package / "usr/bin/eiviz", executable=True)
    copy(
        migrator,
        package / "usr/bin/eiviz-project-migrate",
        executable=True,
    )
    legal_payload(package / "usr/share/doc/eiviz")
    sbom_payload(package / "usr/share/eiviz/sbom")
    copy(
        ROOT / "packaging/scripts/eiviz-data-migration.py",
        package / "usr/lib/eiviz/maintenance/eiviz-data-migration.py",
        executable=True,
    )
    copy(
        ROOT / "packaging/linux/eiviz.desktop",
        package / "usr/share/applications/eiviz.desktop",
    )
    png(package / "usr/share/icons/hicolor/256x256/apps/eiviz.png", 256, 256)
    control = package / "DEBIAN/control"
    control.parent.mkdir(parents=True)
    control.write_text(
        "\n".join(
            [
                "Package: eiviz",
                f"Version: {workspace_version()}",
                "Section: video",
                "Priority: optional",
                "Architecture: amd64",
                "Maintainer: eiviz contributors",
                "Depends: libasound2, libx11-6, libxkbcommon0, libwayland-client0, libvulkan1",
                "Description: Cross-platform GPU vision mixer",
                " eiviz native desktop capability profile.",
                "",
            ]
        ),
        encoding="utf-8",
        newline="\n",
    )
    payload_manifest(
        package,
        package / "usr/share/eiviz/payload-manifest.json",
        "linux-x86_64",
    )
    return package


def command(args: list[str], env: dict[str, str] | None = None) -> None:
    subprocess.run(args, cwd=ROOT, env=env, check=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", choices=CONFIG["targets"], required=True)
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--project-migrator", type=pathlib.Path, required=True)
    parser.add_argument(
        "--msix-publisher",
        default="CN=eiviz unsigned test package",
        help="must exactly match the signing certificate subject for signed MSIX builds",
    )
    parser.add_argument("--output-dir", type=pathlib.Path, default=ROOT / "target/packages")
    parser.add_argument(
        "--staging-dir", type=pathlib.Path, default=ROOT / "target/package-stage"
    )
    args = parser.parse_args()

    output = args.output_dir.resolve()
    stage = (args.staging_dir / args.target).resolve()
    shutil.rmtree(stage, ignore_errors=True)
    stage.mkdir(parents=True)
    output.mkdir(parents=True, exist_ok=True)
    epoch = source_epoch()
    environment = os.environ.copy()
    environment["SOURCE_DATE_EPOCH"] = str(epoch)

    if args.target == "windows-x86_64":
        package_root = stage_windows(
            stage, args.binary, args.project_migrator, args.msix_publisher
        )
        assert_no_optional_sdks(package_root)
        normalize_tree(package_root, epoch)
        artifact = output / f"eiviz-{workspace_version()}-windows-x86_64-unsigned.msix"
        command(["makeappx", "pack", "/o", "/d", str(package_root), "/p", str(artifact)])
    elif args.target == "macos-aarch64":
        app = stage_macos(stage, args.binary, args.project_migrator)
        assert_no_optional_sdks(app)
        normalize_tree(app, epoch)
        artifact = output / f"eiviz-{workspace_version()}-macos-aarch64-unsigned.pkg"
        command(
            [
                "pkgbuild",
                "--component",
                str(app),
                "--install-location",
                "/Applications",
                "--identifier",
                CONFIG["product"]["identifier"],
                "--version",
                workspace_version(),
                str(artifact),
            ],
            environment,
        )
    else:
        package_root = stage_linux(stage, args.binary, args.project_migrator)
        assert_no_optional_sdks(package_root)
        normalize_tree(package_root, epoch)
        artifact = output / f"eiviz_{workspace_version()}_amd64_unsigned.deb"
        command(
            [
                "dpkg-deb",
                "--root-owner-group",
                "--build",
                str(package_root),
                str(artifact),
            ],
            environment,
        )

    checksum = output / f"{artifact.name}.sha256"
    checksum.write_text(
        f"{digest(artifact)}  {artifact.name}\n", encoding="utf-8", newline="\n"
    )
    print(artifact)
    print(checksum)


if __name__ == "__main__":
    main()
