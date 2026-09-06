---
title: Function Reference
description: eiviz function reference
---

## vMix-compatible API

Shortcuts available on the vMix-compatible HTTP API (`GET /api?Function=...`). `Input` is a Scene's flat number, name, or GUID. Cut/Fade and other bus Functions reject a raw Input because a Mixing Unit cannot take it. Unknown numbers are also rejected. `0` is the current Preview, `-1` is the current Program. Omit `Mix` or use `0` for the selected Mixing Unit; `1` and up index Mixing Units in session order.

`Value` is the destination path. `.jpg` / `.jpeg` saves JPEG; anything else (including an omitted path) saves PNG. If `Value` is omitted, a timestamped file is written under Pictures, or the temp directory.

| Function | Parameters | Action |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | Cuts Preview to Program. If `Input` is set, that Scene goes onto Program and Preview is left as-is |
| `CutDirect` | `Input` (required), `Mix` | Puts the input on Program. Preview does not change |
| `Fade` | `Input`, `Mix`, `Duration` | Same target as Cut, then Fade. If `Input` is set, Preview is left as-is. `Duration` is milliseconds. If omitted, the Mixing Unit Fade preset is used, else 1000 |
| `PreviewInput` | `Input` (required), `Mix` | Sets Preview to the input |
| `ActiveInput` | `Input` (required), `Mix` | Sets Program to the input. Preview does not change |
| `Snapshot` | `Value`, `Mix` | Saves Program of the Mixing Unit. `Input` is not used |
| `SnapshotInput` | `Input` (required), `Value`, `Mix` | Saves that Input. Flat numbers are Scenes first, then raw Inputs. `0` / `-1` are Preview / Program of the Mixing Unit |

Examples:

- `http://127.0.0.1:8088/api?Function=Fade&Duration=500`
- `http://127.0.0.1:8088/api?Function=Cut&Input=3`
- `http://127.0.0.1:8088/api?Function=CutDirect&Input=3`
- `http://127.0.0.1:8088/api?Function=Snapshot&Mix=1&Value=C:/Temp/eiviz.png`
- `http://127.0.0.1:8088/api?Function=SnapshotInput&Input=3&Value=C:/Temp/scene.jpg`
