---
title: eiviz API
description: Native control API and headless operations
---

The control plane is handled by `ControlService` inside the mixer. vMix-compatible HTTP/TCP, Protobuf WebSocket, and `eivizctl` are thin adapters over the same dispatcher. vMix listen details are in [Compatibility APIs](/eiviz/en/developers/compatibility/). This page is the native Protobuf surface.

## Contract

- Protocol: `eiviz.control.v1` (`crates/eiviz-api/proto/eiviz/control/v1/control.proto`)
- WebSocket: `ws://`, subprotocol `eiviz.protobuf.v1`, one binary frame per Envelope
- Default listen is loopback port 9400. Bind address, port, token, max role, renderer, and media directory are host-owned (GUI Preferences, `eivizctl prefs`, or `eiviz-headless --bind` / `--renderer` / `EIVIZ_MEDIA_DIRECTORY`). Headless token comes from prefs. They are not stored in the session file
- This release uses authenticated `ws://` on a trusted LAN or VPN only. TLS is not provided
- Published field numbers are never reused. Removals go into `reserved`
- `SaveSession` writes the host current session file. The request is empty. A host with no current file returns `UNAVAILABLE`

Video/audio frames, GPU textures, HWND/NSView, and other presentation/data-plane surfaces are not network APIs.

## Auth and roles

The default bind is loopback. Headless tokens come from `eivizctl prefs`. GUI listen and remote-client tokens use Windows Credential Manager / macOS Keychain. Tokens are never stored in the session file. Comparison is constant-time.

Roles are `read` / `operate` / `configure` / `admin`. The server clamps the granted role to the host max role; a client cannot self-elevate. Arbitrary filesystem load/save/shutdown is admin-only. `SaveSession` writes the host current file and needs configure. Session bodies move as bytes. Non-loopback bind requires authentication.

## Commands

| Command | Role | Meaning |
| --- | --- | --- |
| `GetCapabilities` / `GetSnapshot` / `Subscribe` | read | Capabilities, Document+LiveState, event subscribe |
| `Preview` / `Cut` / `Auto` / `SetMix` / `OverlayAuto` | operate | Live Mixing Unit ops |
| `VideoPlay` / `VideoLoop` / `VideoSeek` | operate | Video inputs |
| `AudioSetInput` / `AudioSetBus` | operate | Audio |
| `SnapshotCmd` / `Discover` | operate | Still capture and discovery |
| `ReplaceSession` | configure | Replace the destination Document (`expected_revision` rejects lost updates) |
| `MutateSession` | configure | Typed document mutation (mismatched `expected_revision` is rejected; the client reloads) |
| `SaveSession` | configure | Write the host current session file |
| `BeginUpload` / `WriteChunk` / `CommitUpload` / `AbortUpload` | configure | Host-directory media upload; commit adds a Still/Video Input atomically |
| `Shutdown` | admin | Graceful stop |

A named-input Cut/Fade does not change Preview. An omitted Input takes Preview and swaps.

A lagged subscriber receives `Lag` and must take a new snapshot, then resume from `after_sequence`. Sequence gaps and server epoch changes also force snapshot resync. Subscribe is persistent: the first event after subscribe is a snapshot plus a sequence barrier.

## Remote GUI

Operator steps are in [Remote connection](/eiviz/en/features/remote/). Still/Video moves through `BeginUpload` … `CommitUpload` into the host media directory, then adds an Input. Path traversal, overwrite, and symlink/junction targets are rejected.

## Errors

`INVALID_ARGUMENT` / `NOT_FOUND` / `AMBIGUOUS` / `CONFLICT` / `UNAVAILABLE` / `PERMISSION_DENIED` / `IO` / `INTERNAL`. Duplicate names are `AMBIGUOUS`, not first-wins.

## eivizctl

```bash
eivizctl --url ws://127.0.0.1:9400 --token YOUR_TOKEN status
eivizctl mix cut --unit 1
eivizctl session save
eivizctl session show
eivizctl input edit --id 2 --name CamA
eivizctl shutdown
eivizctl prefs
eivizctl prefs get bind
eivizctl prefs set bind 127.0.0.1:9400
eivizctl --repl --url ws://127.0.0.1:9400 --token YOUR_TOKEN
```

The default is a one-shot CLI with a subcommand. Pass `--repl` for an interactive prompt, with `--url` and `--token` or `--token-file`. The REPL keeps the WebSocket open. Type `prefs` and the typed CRUD / live commands. There is no raw `mutate`.

`prefs` edits the headless listen file: `%LOCALAPPDATA%\eiviz\headless-prefs.json` on Windows, or `$XDG_CONFIG_HOME/eiviz/headless-prefs.json`. Keys are `bind`, `token`, `mediaDirectory`, `maxRole`, and `renderer`. Token display is `(set)`. Headless auth uses prefs only. Values apply on the next `eiviz-headless run`.

Partial edits send the current snapshot revision as `expected_revision`. `--force` sends revision 0. Unspecified fields stay as they are.

## Headless daemon

Operator steps for start, REPL, and Remote are in [Headless](/eiviz/en/features/headless/). This section is the daemon contract.

```bash
eiviz-headless validate --session show.eivz
eiviz-headless canonicalize --session show.eivz
eiviz-headless export --session show.eivz --output show-portable.eivzx
eiviz-headless history --session show.eivz
eiviz-headless restore --session show.eivz --index 0 --output old.eivz
eiviz-headless run --session show.eivz --bind 127.0.0.1:9400 --renderer auto
eiviz-headless run --bind 127.0.0.1:9400
```

`validate` and `canonicalize` do not initialize the GPU. `run` parses/validates the session, creates the runtime at that FPS, replace/reconciles, then waits on API readiness. Omit `--session` to create a dated default file under the OS `eiviz/sessions` directory. When `--bind` is omitted, `eivizctl prefs` bind is used, then `127.0.0.1:9400`. Renderer is `--renderer` → prefs → `auto`. An OS-unsupported value is an error. Ctrl+C/SIGTERM and API `shutdown` share one stop path: stop accept, wait connections, stop the mixer, then the runtime, with a join deadline. Stdin uses the same syntax as `eivizctl`. The GUI mixer also hosts this WebSocket when listen is enabled in Preferences (bind/token/media directory) or Settings (enable/port).

Exit codes: 2 arguments/read, 3 session, 4 GPU/runtime, 5 bind, 6 other runtime failure.

## Operations

- Rotate tokens with `eivizctl prefs set token` (headless) or the listen token in Preferences, then restart
- Non-loopback bind requires authentication. This release is authenticated `ws://` on a trusted LAN or VPN only; put TLS in front if you need it
- `--media-directory` / `EIVIZ_MEDIA_DIRECTORY` sets the host upload root. When unset, the OS local-app-data `eiviz/media` directory is used
- Logs are structured-enough text on stderr
- Inspect canonical JSON with `eiviz-headless canonicalize`. Ordinary save is `.eivz` (in-file history, no media). Export is `.eivzx` (embedded Still/Video, no history). Headless load of an export extracts media next to the file. The GUI open of an export asks for a media folder and a working `.eivz`
