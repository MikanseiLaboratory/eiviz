---
title: Remote connection
description: Operate another eiviz from Eiviz.Remote
---

Windows ships `Eiviz.Host.exe` (the mixer) and `Eiviz.Remote.exe` (the operator client). macOS ships `eiviz-mac.app` and `eiviz-remote.app`. Live ops and session edits are handled by the destination `ControlService`. See [eiviz API](/eiviz/en/developers/api/) for the protocol.

## Connect

1. Enable API listen on the destination (`Eiviz.Host.exe` Preferences, or `eiviz-headless run --bind`)
2. Set a listen token
3. Launch `Eiviz.Remote.exe` (or `eiviz-remote.app`) and enter the WebSocket URL and token in Preferences

The connection is authenticated `ws://` on a trusted LAN or VPN. Tokens live in Windows Credential Manager / macOS Keychain. They are not stored in session JSON. Host listen fields are in [Settings](/eiviz/en/introduction/settings/).

Multiple clients can stay connected; live state stays aligned through subscribe.

## Video

Preview, Program, and Multiview receive the destination's already-enabled NDI or OMT outputs. Live video appears when exactly one output matches `SourceKind` MuPreview, MuProgram, or Multiview for that Mixing Unit or Multiview. Other counts show Unavailable.

Scene lists and switcher scene buttons are placeholders. Input Preview is for `Eiviz.Host.exe`.

Adding Still/Video picks a file on the client, stores it in the destination media directory, then adds an Input.

## Settings

Display, performance, outputs, audio, and Web API stay on the destination Settings window for review. Adding, opening, editing tiles, and deleting Multiview layouts are sent from the client. Language, theme, and the connection URL belong to `Eiviz.Remote.exe` Preferences.

Session edits use `MutateSession` with `expected_revision`. A mismatched revision is rejected; reload and try again.

## Media directory

Uploaded Still/Video files land on the host. The default when unset is:

- Windows: `%LOCALAPPDATA%\eiviz\media`
- macOS: `~/Library/Application Support/eiviz/media`
- Linux / headless: `$XDG_DATA_HOME/eiviz/media`, or `~/.local/share/eiviz/media` if that is unset

Change it in the host Preferences or with `--media-directory` / `EIVIZ_MEDIA_DIRECTORY`.
