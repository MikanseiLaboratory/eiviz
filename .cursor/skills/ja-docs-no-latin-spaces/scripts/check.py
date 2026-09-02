#!/usr/bin/env python3
"""Fail if Japanese docs put half-width spaces on 和文/欧文 boundaries."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
DEFAULT_DIR = ROOT / "docs/src/content/docs/ja"

JP = r"[ぁ-んァ-ヶｱ-ﾝﾞﾟー一-龯]"
ASCII_RUN = r"[A-Za-z0-9]"

CHECKS: list[tuple[str, str]] = [
    (rf"{ASCII_RUN} {JP}", "ASCIIの直後に和文"),
    (rf"{JP} {ASCII_RUN}", "和文の直後にASCII"),
    (rf"{JP} /", "和文の直後に /"),
    (rf"/ {JP}", "/ の直後に和文"),
    (rf"` {JP}", "インラインコードの直後に和文"),
    (rf"{JP} `", "和文の直後にインラインコード"),
    (rf"\*\* {JP}", "強調の直後に和文"),
    (rf"{JP} \*\*", "和文の直後に強調"),
    (rf"\) {JP}", "リンク閉じの直後に和文"),
]


def iter_files(paths: list[str]) -> list[Path]:
    if not paths:
        return sorted(DEFAULT_DIR.rglob("*.md"))
    files: list[Path] = []
    for raw in paths:
        path = Path(raw)
        if path.is_dir():
            files.extend(sorted(path.rglob("*.md")))
        else:
            files.append(path)
    return files


def check_file(path: Path) -> list[str]:
    hits: list[str] = []
    in_fence = False
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = line.lstrip()
        if stripped.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence or stripped.startswith("|"):
            continue
        if re.search(JP, line) and " / " in line:
            hits.append(f"{path}:{lineno}: 和文行の / 前後スペース: {line.strip()}")
            continue
        for pattern, label in CHECKS:
            if re.search(pattern, line):
                hits.append(f"{path}:{lineno}: {label}: {line.strip()}")
                break
    return hits


def main() -> int:
    hits: list[str] = []
    for path in iter_files(sys.argv[1:]):
        hits.extend(check_file(path))
    if hits:
        print("和欧間スペースがあります。半角語の前後のスペースを削除してください:\n")
        print("\n".join(hits))
        return 1
    print("OK: 和欧間スペースなし")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
