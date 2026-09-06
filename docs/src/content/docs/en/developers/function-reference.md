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

## C ABI parity

`mixer_*` stays a fixed-width C ABI. Control converges on `ControlService`. Presentation and data-plane symbols are not networked.

| Class | Functions | Network | Notes |
| --- | --- | --- | --- |
| lifecycle | `mixer_create` / `mixer_destroy` / `mixer_ping` | no | Readiness follows Composer init. Destroy resets generation statics |
| command (live) | `mixer_unit_cut` / `mixer_unit_auto` / `mixer_unit_overlay_auto` / `mixer_unit_set_state` / `mixer_unit_set_custom_wgsl` | Cut/Auto yes | C ABI also goes through ControlService |
| command (session) | `mixer_session_replace` / `mixer_create_unit` / `mixer_define_scene` / `mixer_define_generator` / `mixer_load_still` / `mixer_video_start` / `mixer_omt_connect` / `mixer_ndi_connect` / `mixer_output_add` / `mixer_audio_bus_upsert` | ReplaceSession and CRUD yes | Hosts treat replace as canonical apply |
| query | `mixer_unit_get_state` / `mixer_session_load` / `mixer_session_canonicalize` / `mixer_poll_events` / `mixer_copy_stats` | GetSnapshot/Subscribe | Poll is bounded |
| presentation | `mixer_unit_attach_native` / `mixer_attach_monitor_native` / `mixer_resize_*` / `mixer_detach_*` | no | HWND/NSView |
| data-plane | `mixer_register_source` / `mixer_push_frame` / `mixer_push_audio` / `mixer_unit_acquire_frame` / `mixer_unit_release_frame` | no | Stay fixed-width |
| diagnostics | `mixer_last_error` / `mixer_take_fatal` / `mixer_copy_rebar_info` | no | Fatal flags reset on in-process restart |
| compat | `mixer_session_publish` / `mixer_api_configure` | vMix HTTP | publish is a deprecated wrapper |

`mixer_session_replace(json, len, expected_revision)` is the canonical apply. `expected_revision=0` skips the check; anything else returns `CONFLICT` on mismatch.

