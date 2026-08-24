#!/usr/bin/env python3
"""Explicit, cross-platform install data backup and rollback helper.

Installers must not rewrite projects or optional SDK configuration. This tool
only snapshots/restores a user-selected data directory. Project schema changes
are performed by eiviz in memory and require an explicit Save.
"""

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import shutil
import sys
import uuid


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inventory(root: pathlib.Path) -> list[dict[str, object]]:
    entries: list[dict[str, object]] = []
    if not root.exists():
        return entries
    for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise ValueError(f"refusing to snapshot symlink: {relative}")
        if path.is_file():
            entries.append(
                {"path": relative, "size": path.stat().st_size, "sha256": sha256(path)}
            )
    return entries


def write_json_atomic(path: pathlib.Path, value: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    temporary.write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    os.replace(temporary, path)


def copy_data(source: pathlib.Path, destination: pathlib.Path) -> None:
    if source.exists():
        shutil.copytree(source, destination, copy_function=shutil.copy2)
    else:
        destination.mkdir(parents=True)


def assert_separate(data_dir: pathlib.Path, state_dir: pathlib.Path) -> None:
    data = data_dir.resolve()
    state = state_dir.resolve()
    if data == state or data in state.parents or state in data.parents:
        raise ValueError("--data-dir and --state-dir must not contain each other")


def prepare(args: argparse.Namespace) -> None:
    data_dir = args.data_dir.resolve()
    state_dir = args.state_dir.resolve()
    assert_separate(data_dir, state_dir)
    transaction = (
        f"{datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')}-"
        f"{uuid.uuid4().hex[:12]}"
    )
    staging = state_dir / f".{transaction}.tmp"
    final = state_dir / transaction
    staging.mkdir(parents=True, exist_ok=False)
    copy_data(data_dir, staging / "data")
    manifest: dict[str, object] = {
        "schema_version": 1,
        "transaction": transaction,
        "status": "prepared",
        "from_version": args.from_version,
        "to_version": args.to_version,
        "data_dir": str(data_dir),
        "files": inventory(staging / "data"),
    }
    write_json_atomic(staging / "migration.json", manifest)
    os.replace(staging, final)
    print(transaction)


def load_transaction(args: argparse.Namespace) -> tuple[pathlib.Path, dict[str, object]]:
    transaction_dir = args.state_dir.resolve() / args.transaction
    manifest_path = transaction_dir / "migration.json"
    if not manifest_path.is_file():
        raise ValueError(f"unknown transaction: {args.transaction}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("transaction") != args.transaction:
        raise ValueError("transaction manifest identifier mismatch")
    return transaction_dir, manifest


def commit(args: argparse.Namespace) -> None:
    transaction_dir, manifest = load_transaction(args)
    if manifest.get("status") != "prepared":
        raise ValueError(f"transaction is {manifest.get('status')}, expected prepared")
    manifest["status"] = "committed"
    write_json_atomic(transaction_dir / "migration.json", manifest)
    print(args.transaction)


def rollback(args: argparse.Namespace) -> None:
    if args.confirm != "RESTORE":
        raise ValueError("rollback requires --confirm RESTORE")
    transaction_dir, manifest = load_transaction(args)
    if manifest.get("status") not in {"prepared", "committed"}:
        raise ValueError(f"transaction is {manifest.get('status')}, cannot restore")
    data_dir = pathlib.Path(str(manifest["data_dir"]))
    state_dir = args.state_dir.resolve()
    assert_separate(data_dir, state_dir)

    rescue = transaction_dir / "pre-rollback-data"
    if rescue.exists():
        raise ValueError("pre-rollback rescue snapshot already exists")
    copy_data(data_dir, rescue)
    expected = manifest.get("files")
    if inventory(transaction_dir / "data") != expected:
        raise ValueError("backup integrity check failed")

    replacement = data_dir.with_name(f".{data_dir.name}.restore-{uuid.uuid4().hex}")
    copy_data(transaction_dir / "data", replacement)
    previous = data_dir.with_name(f".{data_dir.name}.previous-{uuid.uuid4().hex}")
    if data_dir.exists():
        os.replace(data_dir, previous)
    try:
        os.replace(replacement, data_dir)
    except BaseException:
        if previous.exists() and not data_dir.exists():
            os.replace(previous, data_dir)
        raise
    if previous.exists():
        shutil.rmtree(previous)
    manifest["status"] = "rolled-back"
    write_json_atomic(transaction_dir / "migration.json", manifest)
    print(args.transaction)


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    commands = result.add_subparsers(dest="command", required=True)

    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--data-dir", type=pathlib.Path, required=True)
    prepare_parser.add_argument("--state-dir", type=pathlib.Path, required=True)
    prepare_parser.add_argument("--from-version", required=True)
    prepare_parser.add_argument("--to-version", required=True)
    prepare_parser.set_defaults(function=prepare)

    for name, function in (("commit", commit), ("rollback", rollback)):
        command_parser = commands.add_parser(name)
        command_parser.add_argument("--state-dir", type=pathlib.Path, required=True)
        command_parser.add_argument("--transaction", required=True)
        if name == "rollback":
            command_parser.add_argument("--confirm", required=True)
        command_parser.set_defaults(function=function)
    return result


def main() -> None:
    args = parser().parse_args()
    try:
        args.function(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"eiviz-data-migration: {error}", file=sys.stderr)
        raise SystemExit(2) from error


if __name__ == "__main__":
    main()
