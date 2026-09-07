---
title: Headless
description: Run the mixer without a GUI, then operate it from eivizctl or Remote
---

`eiviz-headless` is the mixer daemon with no host UI. It loads a session JSON, runs compose, and opens a Protobuf WebSocket. The default is loopback port 9400.

Linux ships in this form today. Windows and macOS releases include the same binaries. Operate it with `eivizctl` on the same machine, or with `Eiviz.Remote.exe` / `eiviz-remote.app` on another PC. Protocol details are in [eiviz API](/eiviz/en/developers/api/). Remote UI steps are in [Remote connection](/eiviz/en/features/remote/).

Read Start through Connect from Remote once. Come back to the listen keys and REPL commands when you need them.

## What it hosts

Compose, inputs/outputs, and session apply match the GUI host. The control surface it opens is the Protobuf WebSocket only. It does not open vMix-compatible HTTP (8088) or vMix-compatible TCP (8099).

Host Preview/Program windows, Input Preview, and scene thumbnails are for Host. To see headless video in Remote, enable an NDI or OMT output on the destination and pick that source from the Remote header menus.

## Start

Use the `eiviz-headless` and `eivizctl` binaries from a release, or build them in Release:

```bash
cargo build -p eiviz-headless --locked --release --bins
```

The binaries land at `target/release/eiviz-headless` and `target/release/eivizctl`. An example session is `headless/tests/fixtures/bars.eiviz.json`.

```bash
eiviz-headless validate --session show.eiviz.json
eiviz-headless canonicalize --session show.eiviz.json
eiviz-headless run --session show.eiviz.json --bind 127.0.0.1:9400
```

`validate` and `canonicalize` do not initialize the GPU. `run` validates the session, creates the runtime at that FPS, then waits on the WebSocket accept. When ready it prints `eiviz-headless ready ws=` on stderr. Stop with Ctrl+C (and SIGTERM on Unix).

When `--bind` is omitted, the `eivizctl prefs` bind is used, then `127.0.0.1:9400`. Non-loopback bind requires a token. This release is authenticated `ws://` on a trusted LAN or VPN only; TLS is not included.

## Listen

Values that match GUI Preferences are host-owned. They are not stored in session JSON.

| Key | Meaning |
| --- | --- |
| `bind` | WebSocket listen, e.g. `127.0.0.1:9400`, or `0.0.0.0:9400` on a LAN |
| `token` | Connection token. Display is `(set)` |
| `mediaDirectory` | Where Still/Video uploaded from Remote is stored |
| `maxRole` | Role cap: `read` / `operate` / `configure` / `admin` |

The file is `%LOCALAPPDATA%\eiviz\headless-prefs.json` on Windows, or `$XDG_CONFIG_HOME/eiviz/headless-prefs.json` (falling back to `~/.config/eiviz/headless-prefs.json`). Values apply on the next `eiviz-headless run`.

Leftmost wins:

- bind: `--bind` → prefs `bind` → `127.0.0.1:9400`
- token: `EIVIZ_API_TOKEN` or `EIVIZ_API_TOKEN_FILE` → prefs `token`
- media directory: `--media-directory` or `EIVIZ_MEDIA_DIRECTORY` → prefs `mediaDirectory` → OS local-app-data `eiviz/media`
- max role: prefs `maxRole` → `EIVIZ_API_ROLE` → `admin` if a token is set, otherwise `read`

The default media directory when unset is `%LOCALAPPDATA%\eiviz\media` on Windows, `~/Library/Application Support/eiviz/media` on macOS, and `$XDG_DATA_HOME/eiviz/media` on Linux (`~/.local/share/eiviz/media` if that is unset).

## Interactive CLI

The default is a one-shot CLI with a subcommand. Pass `--repl` for an interactive prompt. The prompt is `eiviz>`. Type `exit` or `quit` to leave. The destination is `--url`, default `ws://127.0.0.1:9400`. Pass the client token with `--token` or `--token-file`. The REPL keeps one WebSocket open and does not disconnect after each command.

```bash
eivizctl --url ws://127.0.0.1:9400 --token YOUR_TOKEN cut --unit 1
eivizctl --repl --url ws://127.0.0.1:9400 --token YOUR_TOKEN
```

Listen edits and live mixer ops are separate.

`prefs` writes the local listen file. That works while the daemon is stopped. Live ops and `mutate` need `eiviz-headless run` to have printed `ready`.

```text
eiviz> prefs
eiviz> prefs get bind
eiviz> prefs set bind 0.0.0.0:9400
eiviz> prefs set token YOUR_TOKEN
eiviz> prefs set mediaDirectory /var/lib/eiviz/media
eiviz> prefs set maxRole configure
```

An empty value clears that key. The token value is never printed.

The same commands work as subcommands:

```bash
eivizctl prefs
eivizctl prefs get bind
eivizctl prefs set bind 127.0.0.1:9400
```

Live ops are one line. Roles are in [eiviz API](/eiviz/en/developers/api/).

```text
eiviz> status
eiviz> snapshot
eiviz> preview --unit 1 --scene 2
eiviz> cut --unit 1
eiviz> auto --unit 1 --duration-ms 1000
eiviz> replace --session show.eiviz.json
eiviz> shutdown
```

Partial session edits use `mutate`. JSON `kind` is camelCase. `expected_revision` 0 skips the conflict check. The REPL `mutate` always sends 0.

```text
eiviz> mutate {"kind":"deleteInput","id":2}
```

Settings-window fields use `"kind":"setSettings"`. That payload needs `settings`, `outputs`, and `buses` together. A partial object is rejected, so take a `snapshot` first, or send the same change from the Remote Settings window.

## Connect from Remote

The WebSocket `eiviz-headless` opens is the same `eiviz.control.v1` as the GUI host. The subprotocol is `eiviz.protobuf.v1`.

1. Set a token on the destination. For LAN access, set `bind` to an address clients can reach, such as `0.0.0.0:9400`
2. Start `eiviz-headless run --session show.eiviz.json` and wait for `ready ws=`
3. On the operator PC, launch `Eiviz.Remote.exe` (or `eiviz-remote.app`)
4. Click Connect in the top left, enter the headless IP, port (default 9400), and the same token, then OK

Tokens live in Windows Credential Manager / macOS Keychain. They are not stored in session JSON. The Connect ▾ menu lists recent destinations.

Multiple clients can stay connected. `eivizctl` and Remote can share the same daemon. Live state stays aligned through subscribe.

Preview and Program live video come from an NDI or OMT output on the destination; pick it from the Remote header menus. Multiview shows live video when that layout has exactly one enabled NDI or OMT output. UI details are in [Remote connection](/eiviz/en/features/remote/).

## Environment

`headless/eiviz-headless.example.env` is the template. Do not put tokens on the command line or in session JSON.

| Variable | Use |
| --- | --- |
| `EIVIZ_API_TOKEN` | Token for the server |
| `EIVIZ_API_TOKEN_FILE` | Read the server token from a file |
| `EIVIZ_API_REQUIRE_AUTH` | Anything other than `0` requires auth. Unset means required when a token is set |
| `EIVIZ_API_ROLE` | Server max role |
| `EIVIZ_MEDIA_DIRECTORY` | Upload root |

To rotate a token, change the env var or `eivizctl prefs set token`, then restart `eiviz-headless`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 2 | Arguments or file read |
| 3 | Session validation |
| 4 | GPU / runtime |
| 5 | Bind |
| 6 | Other runtime failure |

Logs are on stderr. Canonical session JSON for backups comes from `eiviz-headless canonicalize`.
