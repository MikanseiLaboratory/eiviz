---
title: eiviz API
description: Native control API and headless operations
---

The control plane lives in Mixer as `ControlService`. vMix-compatible HTTP, Protobuf WebSocket/TCP, and `eivizctl` are thin adapters over the same dispatcher.

## Contract

- Protocol: `eiviz.control.v1` (`crates/eiviz-api/proto/eiviz/control/v1/control.proto`)
- WebSocket: `ws://127.0.0.1:9400`, subprotocol `eiviz.protobuf.v1`, one binary frame per Envelope
- TCP (off by default): `EIVZ` + version + 4-byte big-endian length + Protobuf Envelope
- Published field numbers are never reused. Removals go into `reserved`

Video/audio frames, GPU textures, HWND/NSView, and other presentation/data-plane surfaces are not network APIs.

## Auth and roles

The default bind is loopback. Tokens come from `EIVIZ_API_TOKEN` or `EIVIZ_API_TOKEN_FILE`, never from argv or session JSON. Comparison is constant-time.

Roles are `read` / `operate` / `configure` / `admin`. Arbitrary filesystem load/save/shutdown is admin-only. Session bodies move as bytes, not server paths. Remote bind requires authentication.

## Commands

| Command | Role | Meaning |
| --- | --- | --- |
| `GetCapabilities` / `GetSnapshot` / `Subscribe` | read | Capabilities, Document+LiveState, event subscribe |
| `Preview` / `Cut` / `Auto` / `SetMix` | operate | Live Mixing Unit ops |
| `VideoPlay` / `VideoLoop` / `VideoSeek` | operate | Video inputs |
| `AudioSetInput` / `AudioSetBus` | operate | Audio |
| `SnapshotCmd` / `Discover` | operate | Still capture and discovery |
| `ReplaceSession` | configure | Canonical Document replace (`expected_revision` rejects lost updates) |
| `Shutdown` | admin | Graceful stop |

A named-input Cut/Fade does not change Preview. An omitted Input takes Preview and swaps.

A lagged subscriber receives `Lag` and must `GetSnapshot`, then resume from `after_sequence`.

## Errors

`INVALID_ARGUMENT` / `NOT_FOUND` / `AMBIGUOUS` / `CONFLICT` / `UNAVAILABLE` / `PERMISSION_DENIED` / `IO` / `INTERNAL`. Duplicate names are `AMBIGUOUS`, not first-wins.

## eivizctl

```bash
eivizctl --url ws://127.0.0.1:9400 status
eivizctl cut --unit 1
eivizctl snapshot
eivizctl shutdown
```

## Headless daemon

```bash
eiviz-headless validate --session show.eiviz.json
eiviz-headless canonicalize --session show.eiviz.json
eiviz-headless run --session show.eiviz.json --bind 127.0.0.1:9400
```

`validate` and `canonicalize` do not initialize the GPU. `run` parses/validates the session, creates the runtime at that FPS, replace/reconciles, then waits on API readiness. Ctrl+C/SIGTERM stops accept, then workers, inputs/outputs, then render, with a join deadline.

Exit codes: 2 arguments/read, 3 session, 4 GPU/runtime, 5 bind, 6 other runtime failure.

## Operations

- Rotate tokens by changing `EIVIZ_API_TOKEN` and restarting the daemon
- Remote bind requires auth and TLS termination
- Logs are structured-enough text on stderr
- Back up canonical session JSON from `eiviz-headless canonicalize`
- If a GUI later starts headless, spawn the shipped `eiviz-headless` binary; do not duplicate lifecycle in C#/Swift
