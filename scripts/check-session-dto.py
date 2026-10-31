#!/usr/bin/env python3
"""Checks that the Windows host's session DTOs carry every field the core defines.

A publish replaces the whole session document in the core, so a field the host
forgets is silently reset there: the scene camera was missing that way, and every
replay edit put the operator's framing back to default. The host DTOs are written
by hand against the Rust definitions, so this compares the two and fails when one
side has a field the other does not.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RUST = ROOT / "crates/eiviz-control/src/session/mod.rs"
CS_FILES = [ROOT / "hosts/win32/SessionStore.cs", ROOT / "hosts/win32/Session.cs"]

# Rust struct -> C# class. The C# side names some types differently.
PAIRS = [
    ("Document", "Document"),
    ("InputDto", "InputDto"),
    ("SceneDto", "SceneDto"),
    ("UnitDto", "UnitDto"),
    ("MultiviewDto", "MultiviewDto"),
    ("SceneLayer", "SceneLayer"),
    ("SceneState", "SceneState"),
    ("SceneSequence", "SceneSequence"),
    ("SessionSettings", "SessionSettings"),
    ("OverlaySlot", "OverlaySlot"),
    ("TransitionPreset", "TransitionPreset"),
    ("SceneLayoutPreset", "SceneLayoutPreset"),
    ("SceneCamera", "SceneCamera"),
]


def norm(name: str) -> str:
    return name.replace("_", "").lower()


def rust_fields(text: str, struct: str) -> list[str]:
    match = re.search(rf"pub struct {struct}\b[^{{]*\{{", text)
    if not match:
        return []
    body = text[match.end():]
    depth = 1
    fields = []
    for line in body.splitlines():
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        depth += stripped.count("{") - stripped.count("}")
        if depth <= 0:
            break
        field = re.match(r"(?:pub\s+)?([a-z_0-9]+)\s*:", stripped)
        if field:
            fields.append(norm(field.group(1)))
    return fields


def cs_fields(text: str, klass: str) -> list[str]:
    match = re.search(rf"class {klass}\b[^{{]*\{{", text)
    if not match:
        return []
    body = text[match.end():]
    depth = 1
    fields = []
    for line in body.splitlines():
        stripped = line.strip()
        if stripped.startswith("//"):
            continue
        # Properties, not fields: the DTOs use `public T Name { get; set; }`, and a
        # computed one (`=> ...`) still reaches the JSON as a value.
        prop = re.match(
            r"public\s+[\w<>\[\],\?\.\s]+?\s+([A-Z][A-Za-z0-9]*)\s*(?:\{\s*(?:get|set)|=>)",
            stripped,
        )
        if prop:
            fields.append(norm(prop.group(1)))
        depth += stripped.count("{") - stripped.count("}")
        if depth <= 0:
            break
    return fields


# Fields the host deliberately does not send. `renderer` is a machine capability: the
# core keeps its own value on a settings update, and an exported session resets it to
# `Auto` on purpose, so a host publish saying `Auto` matches what the core does anyway.
ALLOWED = {
    "SessionSettings": {"renderer"},
}


def main() -> int:
    rust_text = RUST.read_text(encoding="utf-8")
    cs_text = "\n".join(path.read_text(encoding="utf-8-sig") for path in CS_FILES)
    problems = []
    for rust_name, cs_name in PAIRS:
        rust = rust_fields(rust_text, rust_name)
        cs = set(cs_fields(cs_text, cs_name))
        if not rust:
            problems.append(f"{rust_name}: not found in {RUST.name}")
            continue
        if not cs:
            problems.append(f"{cs_name}: not found in the host DTOs")
            continue
        allowed = ALLOWED.get(rust_name, set())
        missing = [field for field in rust if field not in cs and field not in allowed]
        if missing:
            problems.append(f"{rust_name} -> {cs_name}: host is missing {', '.join(missing)}")
    if problems:
        print("session DTO drift:", file=sys.stderr)
        for line in problems:
            print(f"  {line}", file=sys.stderr)
        return 1
    print(f"session DTOs match the core ({len(PAIRS)} structs checked)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
