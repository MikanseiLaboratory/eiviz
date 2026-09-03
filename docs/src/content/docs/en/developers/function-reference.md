---
title: Function Reference
description: eiviz function reference
---

Shortcuts available on the vMix-compatible HTTP API (`GET /api?Function=...`). `Input` is a Scene's flat number, name, or GUID. A raw Input is rejected because a Mixing Unit cannot take it. Unknown numbers are also rejected. `0` is the current Preview, `-1` is the current Program. Omit `Mix` or use `0` for the selected Mixing Unit; `1` and up index Mixing Units in session order.

| Function | Parameters | Action |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | Cuts Preview to Program. If `Input` is set, that input is placed on Preview first |
| `CutDirect` | `Input` (required), `Mix` | Puts the input on Program. Preview does not change |
| `Fade` | `Input`, `Mix`, `Duration` | Same target as Cut, then Fade. `Duration` is milliseconds. If omitted, the Mixing Unit Fade preset is used, else 1000 |
| `PreviewInput` | `Input` (required), `Mix` | Sets Preview to the input |
| `ActiveInput` | `Input` (required), `Mix` | Sets Program to the input. Preview does not change |

Examples:

- `http://127.0.0.1:8088/api?Function=Fade&Duration=500`
- `http://127.0.0.1:8088/api?Function=CutDirect&Input=3`
