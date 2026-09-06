---
title: eiviz API
description: Native control API and headless operations
---

The control plane lives in Mixer as `ControlService`. vMix-compatible HTTP/TCP, Protobuf WebSocket, and `eivizctl` are thin adapters over the same dispatcher. vMix listen details are in [Compatibility APIs](/eiviz/en/developers/compatibility/). This page is the native Protobuf surface.

## Contract

- Protocol: `eiviz.control.v1` (`crates/eiviz-api/proto/eiviz/control/v1/control.proto`)
- WebSocket: `ws://`, subprotocol `eiviz.protobuf.v1`, one binary frame per Envelope
- Default listen is loopback port 9400. Bind address, port, token, max role, and media directory are host-owned (GUI Preferences, or `eiviz-headless --bind` / `EIVIZ_API_TOKEN` / `EIVIZ_MEDIA_DIRECTORY`). They are not stored in session JSON
- This release uses authenticated `ws://` on a trusted LAN or VPN only. TLS is not provided
- Published field numbers are never reused. Removals go into `reserved`

Video/audio frames, GPU textures, HWND/NSView, and other presentation/data-plane surfaces are not network APIs.

## Auth and roles

The default bind is loopback. Headless tokens come from `EIVIZ_API_TOKEN` or `EIVIZ_API_TOKEN_FILE`. GUI listen and remote-client tokens use Windows Credential Manager / macOS Keychain. Tokens are never stored in session JSON. Comparison is constant-time.

Roles are `read` / `operate` / `configure` / `admin`. The server clamps the granted role to the host max role; a client cannot self-elevate. Arbitrary filesystem load/save/shutdown is admin-only. Session bodies move as bytes, not server paths. Non-loopback bind requires authentication.

## Commands

| Command | Role | Meaning |
| --- | --- | --- |
| `GetCapabilities` / `GetSnapshot` / `Subscribe` | read | Capabilities, Document+LiveState, event subscribe |
| `Preview` / `Cut` / `Auto` / `SetMix` / `OverlayAuto` | operate | Live Mixing Unit ops |
| `VideoPlay` / `VideoLoop` / `VideoSeek` | operate | Video inputs |
| `AudioSetInput` / `AudioSetBus` | operate | Audio |
| `SnapshotCmd` / `Discover` | operate | Still capture and discovery |
| `ReplaceSession` | configure | Canonical Document replace (`expected_revision` rejects lost updates) |
| `MutateSession` | configure | Typed document mutation (`expected_revision` rejects lost updates; no silent merge) |
| `BeginUpload` / `WriteChunk` / `CommitUpload` / `AbortUpload` | configure | Host-directory media upload; commit adds a Still/Video Input atomically |
| `Shutdown` | admin | Graceful stop |

A named-input Cut/Fade does not change Preview. An omitted Input takes Preview and swaps.

A lagged subscriber receives `Lag` and must take a new snapshot, then resume from `after_sequence`. Sequence gaps and server epoch changes also force snapshot resync. Subscribe is persistent: the first event after subscribe is a snapshot plus a sequence barrier.

## Remote GUI

Windows and macOS can connect as a remote client from Preferences. The remote host session is the source of truth. Settings on the client is view-only except client-local Preferences (language, theme, connection). Input Preview and scene thumbnails are off. Preview/Program/Multiview video uses only already-enabled NDI or OMT outputs named as `MuPreview` / `MuProgram` / `Multiview`. eiviz does not create extra outputs. If none exists, or more than one matches, the surface shows Unavailable.

Upload of Still/Video picks a file on the client, stores it in the host media directory, then adds an Input. Path traversal, overwrite, and symlink/junction targets are rejected.

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

`validate` and `canonicalize` do not initialize the GPU. `run` parses/validates the session, creates the runtime at that FPS, replace/reconciles, then waits on API readiness. Ctrl+C/SIGTERM stops accept, then workers, inputs/outputs, then render, with a join deadline. The GUI mixer also hosts this WebSocket when listen is enabled in Preferences (bind/token/media directory) or Settings (enable/port).

Exit codes: 2 arguments/read, 3 session, 4 GPU/runtime, 5 bind, 6 other runtime failure.

## Operations

- Rotate tokens by changing `EIVIZ_API_TOKEN` (headless) or the listen token in Preferences, then restart
- Non-loopback bind requires authentication. This release is authenticated `ws://` on a trusted LAN or VPN only; put TLS in front if you need it
- `--media-directory` / `EIVIZ_MEDIA_DIRECTORY` sets the host upload root
- Logs are structured-enough text on stderr
- Back up canonical session JSON from `eiviz-headless canonicalize`
