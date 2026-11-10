#!/usr/bin/env python3
"""Checks that every host sees the same internal C ABI.

The macOS header and the Windows P/Invoke declarations are written by hand against
the Rust exports. A function that exists only in Rust is invisible to a host, and a
host declaration that was renamed fails at runtime, not at compile time. This
compares all three and fails on drift.

Platform-only exports are listed below with the reason they are not in the macOS
header. Adding a new export means adding it to the header (or, when it is genuinely
Windows-only, to this list).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST_DIRS = [ROOT / "mixer/src"]
HEADER = ROOT / "hosts/macos/Sources/EivizMixer/eiviz_mixer.h"
WIN = ROOT / "hosts/win32/Interop/MixerNative.cs"

# Exports the macOS host does not call. Keep the reason with the name: the point of
# this list is to make every gap a decision rather than an accident.
MACOS_UNUSED = {
    "mixer_attach_monitor": "macOS uses mixer_attach_monitor_native",
    "mixer_copy_monitor_audio": "monitor audio is copied by the host meter path",
    "mixer_pro_load": "used by the headless/Pro test entry point, not a host function",
    "mixer_secret_clear": "no macOS caller yet",
    "mixer_set_master_fps": "no host caller yet",
    "mixer_stream_destination_add": "streaming ABI is not wired to a host yet",
    "mixer_stream_destination_remove": "streaming ABI is not wired to a host yet",
    "mixer_stream_destination_stats": "streaming ABI is not wired to a host yet",
    "mixer_stream_encoder_add": "streaming ABI is not wired to a host yet",
    "mixer_stream_encoder_remove": "streaming ABI is not wired to a host yet",
    "mixer_unit_acquire_frame": "native surface path does not use frame acquisition",
    "mixer_unit_attach_output": "macOS uses mixer_unit_attach_native",
    "mixer_unit_detach_output": "macOS uses mixer_unit_detach_native",
    "mixer_unit_release_frame": "native surface path does not use frame acquisition",
    "mixer_unit_resize_output": "macOS uses mixer_unit_resize_native",
}


def rust_exports() -> set[str]:
    names = set()
    for directory in RUST_DIRS:
        for path in directory.rglob("*.rs"):
            text = path.read_text(encoding="utf-8")
            names.update(
                re.findall(r'pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(mixer_[A-Za-z0-9_]+)', text)
            )
    return names


def header_names() -> set[str]:
    text = HEADER.read_text(encoding="utf-8")
    return set(re.findall(r"\b(mixer_[A-Za-z0-9_]+)\s*\(", text))


def win_names() -> set[str]:
    text = WIN.read_text(encoding="utf-8")
    return set(re.findall(r'EntryPoint\s*=\s*"(mixer_[A-Za-z0-9_]+)"', text))


def main() -> int:
    exports = rust_exports()
    header = header_names()
    windows = win_names()
    problems = []

    if not exports:
        problems.append("no Rust mixer_ exports found")
    unknown_header = sorted(header - exports)
    if unknown_header:
        problems.append("macOS header names that Rust does not export: " + ", ".join(unknown_header))
    unknown_win = sorted(windows - exports)
    if unknown_win:
        problems.append("Windows EntryPoints that Rust does not export: " + ", ".join(unknown_win))
    missing_header = sorted(exports - header - set(MACOS_UNUSED))
    if missing_header:
        problems.append("Rust exports missing from the macOS header: " + ", ".join(missing_header))

    if problems:
        print("C ABI drift:", file=sys.stderr)
        for line in problems:
            print(f"  {line}", file=sys.stderr)
        return 1
    print(
        "C ABI matches "
        f"({len(exports)} Rust exports, {len(header)} macOS declarations, "
        f"{len(windows)} Windows imports, {len(MACOS_UNUSED)} platform-only allowlisted)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
