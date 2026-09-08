---
title: Remote connection
description: Operate another eiviz from Eiviz.Remote
---

Windows ships `Eiviz.Host.exe` (the mixer) and `Eiviz.Remote.exe` (the operator client). macOS ships `eiviz-mac.app` and `eiviz-remote.app`. Live ops and session edits are handled by the destination `ControlService`. See [eiviz API](/eiviz/en/developers/api/) for the protocol.

## Connect

1. Enable API listen on the destination (`Eiviz.Host.exe` Preferences, or [Headless](/eiviz/en/features/headless/) `eiviz-headless run`)
2. Set a listen token
3. Launch `Eiviz.Remote.exe` (or `eiviz-remote.app`)
4. Click Connect in the top left, enter the IP, port, and token, then OK. The operator UI opens after the connection succeeds.

The connection is authenticated `ws://` on a trusted LAN or VPN. Tokens live in Windows Credential Manager / macOS Keychain. They are not stored in the session file. Host listen fields are in [Settings](/eiviz/en/introduction/settings/). When the destination is `eiviz-headless`, listen and token steps are in [Headless](/eiviz/en/features/headless/).

The Connect ▾ menu lists recent destinations. Disconnect closes the connection.

Multiple clients can stay connected; live state stays aligned through subscribe.

## Video

The main video row is either Preview and Program, or one Multiview pane. Switch the layout from the top bar. Pick the NDI or OMT source from each header menu. CPU versus GPU OMT receive is a Preferences setting on `Eiviz.Remote.exe` (or `eiviz-remote.app`). The default is CPU. Add, tile-edit, and delete Multiview layouts from Settings.

The scene list and switcher scene buttons show every Scene, collapsed. Preview/Program chrome still paints the collapsed tiles.

Adding Still/Video picks a file on the client, stores it in the destination media directory, then adds an Input.

## Settings

The Settings window sends display, performance, outputs, audio, and Web API fields to the destination session. Adding, editing tiles, and deleting Multiview layouts are also sent from Settings. Language, theme, and OMT receive belong to `Eiviz.Remote.exe` Preferences.

Session edits use `MutateSession` with `expected_revision`. A mismatched revision is rejected; reload and try again.

## Save

The Save button writes the host's current session file. Remote does not send a path or filename. The host must already have a file (GUI Save, or `eiviz-headless run --session`). If the host has never saved, the request fails. Success shows the saved path and how many in-file history entries remain. The previous document is kept inside the `.eivz` (up to 20). Export files (`.eivzx`) do not include that history.

`eivizctl save` is the same command.

## Media directory

Uploaded Still/Video files land on the host. The default when unset is:

- Windows: `%LOCALAPPDATA%\eiviz\media`
- macOS: `~/Library/Application Support/eiviz/media`
- Linux / headless: `$XDG_DATA_HOME/eiviz/media`, or `~/.local/share/eiviz/media` if that is unset

Change it in the host Preferences, with `--media-directory` / `EIVIZ_MEDIA_DIRECTORY`, or `eivizctl prefs`.
