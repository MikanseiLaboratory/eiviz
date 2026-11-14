#!/usr/bin/env python3
"""Checks that every host sees the same internal C ABI.

The macOS header and the Windows P/Invoke declarations are written by hand against
the Rust exports. A function that exists only in Rust is invisible to a host, and a
host declaration with the wrong arity fails at runtime, not at compile time. This
compares all three: every export must be declared for each host (or listed below
with a reason), and a name that both sides declare must take the same number of
arguments.

Platform-only exports are listed below with the reason they are not in a host. The
point of the list is to make every gap a decision rather than an accident.

This is a structural check, not a type checker: it compares names and parameter
counts, not the types inside them. Type drift is still reviewed by hand.
"""
import re
import sys
from pathlib import Path
from typing import Optional

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
    "mixer_unit_acquire_frame": "native surface path does not use frame acquisition",
    "mixer_unit_attach_output": "macOS uses mixer_unit_attach_native",
    "mixer_unit_detach_output": "macOS uses mixer_unit_detach_native",
    "mixer_unit_release_frame": "native surface path does not use frame acquisition",
    "mixer_unit_resize_output": "macOS uses mixer_unit_resize_native",
}

# Exports the Windows host does not call.
WINDOWS_UNUSED = {
    "mixer_attach_monitor_native": "Windows uses mixer_attach_monitor",
    "mixer_audio_unit_bus_get": "the Windows host reads unit buses from the session document",
    "mixer_pro_load": "used by the headless/Pro test entry point, not a host function",
    "mixer_secret_clear": "no Windows caller yet",
    "mixer_session_copy_json": "macOS keeps its own copy of the document; it polls the revision",
    "mixer_session_revision": "macOS keeps its own copy of the document; it polls the revision",
    "mixer_set_master_fps": "no host caller yet",
    "mixer_stream_destination_add": "Windows starts RTMP through the session document",
    "mixer_stream_destination_remove": "Windows stops RTMP through the session document",
    "mixer_stream_encoder_add": "Windows starts RTMP through the session document",
    "mixer_stream_encoder_remove": "Windows stops RTMP through the session document",
    "mixer_vt_encoder_destroy": "macOS Pro RTMP resolves the host VideoToolbox encoder service",
    "mixer_vt_encoder_headers": "macOS Pro RTMP resolves the host VideoToolbox encoder service",
    "mixer_vt_encoder_encode": "macOS Pro RTMP resolves the host VideoToolbox encoder service",
    "mixer_vt_encoder_create": "macOS Pro RTMP resolves the host VideoToolbox encoder service",
    "mixer_unit_acquire_frame": "Windows surface path publishes frames, it does not acquire them",
    "mixer_unit_attach_native": "Windows uses mixer_unit_attach_output",
    "mixer_unit_detach_native": "Windows uses mixer_unit_detach_output",
    "mixer_unit_release_frame": "Windows surface path publishes frames, it does not acquire them",
    "mixer_unit_resize_native": "Windows uses mixer_unit_resize_output",
}


def strip_comments(text: str) -> str:
    """Removes C/C++ and C# comments so a commented-out prototype cannot count."""
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def param_count(params: str) -> int:
    """Number of top-level parameters, with `(void)` and empty lists counting as zero."""
    depth = 0
    count = 0
    current = []
    for char in params:
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        if char == "," and depth == 0:
            if "".join(current).strip():
                count += 1
            current = []
            continue
        current.append(char)
    if "".join(current).strip() and "".join(current).strip() != "void":
        count += 1
    return count


def paren_body(text: str, open_index: int) -> Optional[str]:
    """Text between the parenthesis at `open_index` and its match."""
    depth = 0
    index = open_index
    while index < len(text):
        if text[index] == "(":
            depth += 1
        elif text[index] == ")":
            depth -= 1
            if depth == 0:
                return text[open_index + 1:index]
        index += 1
    return None


def rust_exports() -> dict[str, int]:
    arity: dict[str, int] = {}
    for directory in RUST_DIRS:
        for path in directory.rglob("*.rs"):
            text = strip_comments(path.read_text(encoding="utf-8"))
            for match in re.finditer(
                r'pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(mixer_[A-Za-z0-9_]+)\s*\(',
                text,
            ):
                body = paren_body(text, match.end() - 1)
                if body is not None:
                    arity[match.group(1)] = param_count(body)
    return arity


def header_declarations() -> dict[str, int]:
    text = strip_comments(HEADER.read_text(encoding="utf-8"))
    arity: dict[str, int] = {}
    for match in re.finditer(r"\b(mixer_[A-Za-z0-9_]+)\s*\(", text):
        body = paren_body(text, match.end() - 1)
        if body is not None:
            arity[match.group(1)] = param_count(body)
    return arity


def win_declarations() -> dict[str, int]:
    text = strip_comments(WIN.read_text(encoding="utf-8"))
    arity: dict[str, int] = {}
    for match in re.finditer(r'EntryPoint\s*=\s*"(mixer_[A-Za-z0-9_]+)"', text):
        open_index = text.find("(", match.end())
        if open_index < 0:
            continue
        body = paren_body(text, open_index)
        if body is not None:
            arity[match.group(1)] = param_count(body)
    return arity


def main() -> int:
    exports = rust_exports()
    header = header_declarations()
    windows = win_declarations()
    problems = []

    if not exports:
        problems.append("no Rust mixer_ exports found")
    unknown_header = sorted(set(header) - set(exports))
    if unknown_header:
        problems.append("macOS header names that Rust does not export: " + ", ".join(unknown_header))
    unknown_win = sorted(set(windows) - set(exports))
    if unknown_win:
        problems.append("Windows EntryPoints that Rust does not export: " + ", ".join(unknown_win))
    missing_header = sorted(set(exports) - set(header) - set(MACOS_UNUSED))
    if missing_header:
        problems.append("Rust exports missing from the macOS header: " + ", ".join(missing_header))
    missing_win = sorted(set(exports) - set(windows) - set(WINDOWS_UNUSED))
    if missing_win:
        problems.append("Rust exports missing from the Windows host: " + ", ".join(missing_win))

    for name in sorted(set(exports) & set(header)):
        if exports[name] != header[name]:
            problems.append(
                f"{name}: Rust takes {exports[name]} arguments, macOS header declares {header[name]}"
            )
    for name in sorted(set(exports) & set(windows)):
        if exports[name] != windows[name]:
            problems.append(
                f"{name}: Rust takes {exports[name]} arguments, Windows host declares {windows[name]}"
            )

    if problems:
        print("C ABI drift:", file=sys.stderr)
        for line in problems:
            print(f"  {line}", file=sys.stderr)
        return 1
    print(
        "C ABI matches "
        f"({len(exports)} Rust exports, {len(header)} macOS declarations, "
        f"{len(windows)} Windows imports, {len(MACOS_UNUSED) + len(WINDOWS_UNUSED)} platform-only allowlisted)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
