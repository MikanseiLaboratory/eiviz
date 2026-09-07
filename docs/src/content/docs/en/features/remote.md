---
title: Remote connection
description: Operate another eiviz from Eiviz.Remote
---

Windows ships `Eiviz.Host.exe` (the mixer) and `Eiviz.Remote.exe` (the operator client). macOS ships `eiviz-mac.app` and `eiviz-remote.app`. Live ops and session edits are handled by the destination `ControlService`. See [eiviz API](/eiviz/en/developers/api/) for the protocol.

## Connect

1. Enable API listen on the destination (`Eiviz.Host.exe` Preferences, or `eiviz-headless run --bind`)
2. Set a listen token
3. Launch `Eiviz.Remote.exe` (or `eiviz-remote.app`) and click Connect in the top left
4. Enter the WebSocket URL and token, then OK

The connection is authenticated `ws://` on a trusted LAN or VPN. Tokens live in Windows Credential Manager / macOS Keychain. They are not stored in session JSON. Host listen fields are in [Settings](/eiviz/en/introduction/settings/).

The Connect ▾ menu lists recent URLs.

Multiple clients can stay connected; live state stays aligned through subscribe.

## Video

Preview and Program use the same PRV/PGM frames as Host. Pick the NDI or OMT source from each header menu. Multiview shows live video when the destination has exactly one enabled NDI or OMT output for that layout. Input Preview is for Host.

The scene list and switcher scene buttons show every Scene, collapsed. Thumbnails are for Host.

Adding Still/Video picks a file on the client, stores it in the destination media directory, then adds an Input.

## Settings

Display, performance, outputs, audio, and Web API stay on the destination Settings window for review. Adding, opening, editing tiles, and deleting Multiview layouts are sent from the client. Language and theme belong to `Eiviz.Remote.exe` Preferences.

Session edits use `MutateSession` with `expected_revision`. A mismatched revision is rejected; reload and try again.

## Media directory

Uploaded Still/Video files land on the host. The default when unset is:

- Windows: `%LOCALAPPDATA%\eiviz\media`
- macOS: `~/Library/Application Support/eiviz/media`
- Linux / headless: `$XDG_DATA_HOME/eiviz/media`, or `~/.local/share/eiviz/media` if that is unset

Change it in the host Preferences or with `--media-directory` / `EIVIZ_MEDIA_DIRECTORY`.
