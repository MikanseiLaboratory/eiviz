#!/usr/bin/env python3

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
DATA_TOOL = ROOT / "packaging/scripts/eiviz-data-migration.py"


class PackagingPolicyTests(unittest.TestCase):
    def test_manifest_and_feature_profiles_validate(self) -> None:
        subprocess.run(
            [sys.executable, ROOT / "scripts/validate-package.py"],
            cwd=ROOT,
            check=True,
        )

    def test_data_upgrade_can_commit_and_explicitly_rollback(self) -> None:
        with tempfile.TemporaryDirectory(prefix="eiviz-migration-test-") as temporary:
            temporary_root = pathlib.Path(temporary)
            data = temporary_root / "data"
            state = temporary_root / "state"
            data.mkdir()
            (data / "config.json").write_text('{"schema_version": 1}\n')
            (data / "project.json").write_text('{"name": "before"}\n')

            prepared = subprocess.run(
                [
                    sys.executable,
                    DATA_TOOL,
                    "prepare",
                    "--data-dir",
                    data,
                    "--state-dir",
                    state,
                    "--from-version",
                    "0.1.0",
                    "--to-version",
                    "0.2.0",
                ],
                check=True,
                text=True,
                capture_output=True,
            )
            transaction = prepared.stdout.strip()
            (data / "config.json").write_text('{"schema_version": 2}\n')
            subprocess.run(
                [
                    sys.executable,
                    DATA_TOOL,
                    "commit",
                    "--state-dir",
                    state,
                    "--transaction",
                    transaction,
                ],
                check=True,
            )
            subprocess.run(
                [
                    sys.executable,
                    DATA_TOOL,
                    "rollback",
                    "--state-dir",
                    state,
                    "--transaction",
                    transaction,
                    "--confirm",
                    "RESTORE",
                ],
                check=True,
            )
            self.assertEqual(
                json.loads((data / "config.json").read_text())["schema_version"], 1
            )
            manifest = json.loads(
                (state / transaction / "migration.json").read_text(encoding="utf-8")
            )
            self.assertEqual(manifest["status"], "rolled-back")
            self.assertTrue((state / transaction / "pre-rollback-data").is_dir())

    def test_rollback_requires_explicit_confirmation(self) -> None:
        result = subprocess.run(
            [
                sys.executable,
                DATA_TOOL,
                "rollback",
                "--state-dir",
                "/nonexistent",
                "--transaction",
                "none",
                "--confirm",
                "NO",
            ],
            text=True,
            capture_output=True,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("--confirm RESTORE", result.stderr)


if __name__ == "__main__":
    unittest.main()
