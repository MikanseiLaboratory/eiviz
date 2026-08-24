#!/usr/bin/env python3
"""Generate deterministic SPDX 2.3 and CycloneDX 1.5 JSON from Cargo metadata."""

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import subprocess
import uuid


def identifier(package_id: str) -> str:
    digest = hashlib.sha256(package_id.encode()).hexdigest()[:16]
    return f"SPDXRef-Package-{digest}"


def timestamp() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    instant = (
        datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc)
        if epoch
        else datetime.datetime.now(datetime.timezone.utc)
    )
    return instant.replace(microsecond=0).isoformat().replace("+00:00", "Z")


def purl(package: dict) -> str:
    name = package["name"]
    version = package["version"]
    return f"pkg:cargo/{name}@{version}"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default="target/sbom")
    args = parser.parse_args()
    root = pathlib.Path(__file__).resolve().parents[1]
    output = root / args.output
    output.mkdir(parents=True, exist_ok=True)
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            cwd=root,
            text=True,
        )
    )
    packages = sorted(metadata["packages"], key=lambda item: item["id"])
    package_by_id = {package["id"]: package for package in packages}
    lock_hash = hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest()
    created = timestamp()

    spdx_packages = []
    for package in packages:
        declared = package.get("license") or "NOASSERTION"
        entry = {
            "SPDXID": identifier(package["id"]),
            "name": package["name"],
            "versionInfo": package["version"],
            "downloadLocation": package.get("source") or "NOASSERTION",
            "filesAnalyzed": False,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": declared,
            "copyrightText": "NOASSERTION",
            "externalRefs": [
                {
                    "referenceCategory": "PACKAGE-MANAGER",
                    "referenceType": "purl",
                    "referenceLocator": purl(package),
                }
            ],
        }
        spdx_packages.append(entry)
    spdx_relationships = []
    for node in metadata["resolve"]["nodes"]:
        if node["id"] not in package_by_id:
            continue
        for dependency in sorted(node["dependencies"]):
            if dependency in package_by_id:
                spdx_relationships.append(
                    {
                        "spdxElementId": identifier(node["id"]),
                        "relationshipType": "DEPENDS_ON",
                        "relatedSpdxElement": identifier(dependency),
                    }
                )
    spdx = {
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "eiviz",
        "documentNamespace": f"https://github.com/mikanseilaboratory/eiviz/sbom/{lock_hash}",
        "creationInfo": {"created": created, "creators": ["Tool: scripts/generate-sbom.py"]},
        "packages": spdx_packages,
        "relationships": spdx_relationships,
    }

    components = []
    for package in packages:
        component = {
            "type": "library",
            "bom-ref": package["id"],
            "name": package["name"],
            "version": package["version"],
            "purl": purl(package),
        }
        if package.get("license"):
            component["licenses"] = [{"expression": package["license"]}]
        components.append(component)
    dependencies = []
    for node in sorted(metadata["resolve"]["nodes"], key=lambda item: item["id"]):
        dependencies.append(
            {
                "ref": node["id"],
                "dependsOn": sorted(
                    dependency
                    for dependency in node["dependencies"]
                    if dependency in package_by_id
                ),
            }
        )
    cyclone = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{uuid.uuid5(uuid.NAMESPACE_URL, lock_hash)}",
        "version": 1,
        "metadata": {
            "timestamp": created,
            "tools": {
                "components": [
                    {
                        "type": "application",
                        "name": "eiviz Cargo metadata SBOM generator",
                        "version": "1",
                    }
                ]
            },
            "component": {
                "type": "application",
                "bom-ref": "eiviz-workspace",
                "name": "eiviz",
                "version": package_by_id[metadata["workspace_members"][0]]["version"],
            },
        },
        "components": components,
        "dependencies": dependencies,
    }

    (output / "eiviz.spdx.json").write_text(
        json.dumps(spdx, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (output / "eiviz.cyclonedx.json").write_text(
        json.dumps(cyclone, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(output / "eiviz.spdx.json")
    print(output / "eiviz.cyclonedx.json")


if __name__ == "__main__":
    main()
