---
title: Function Reference
description: eivizの関数リファレンス
---

## vMix互換API

vMix互換HTTP APIおよびTCP APIで使用できる主なFunction（Shortcut）の一覧です。

- `Input`: Scene番号、名前、またはGUIDを指定します。CutやFadeなどMixing Unitを伴う操作ではSceneのみ指定可能です。`0`で現在のPreview、`-1`で現在のProgramを指定できます。
- `Mix`: 操作対象のMixing Unit番号を指定します（省略時または`0`は選択中のユニット）。
- `Value`: 保存先ファイルパスを指定します（拡張子が`.jpg`/`.jpeg`ならJPEG、それ以外はPNG）。省略時はピクチャフォルダまたは一時ディレクトリに日時付きで保存されます。

| Function | 引数 | 動作 |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | PreviewとProgramをCutで切り替えます。`Input`指定時は該当Sceneを直接Programへ送り、Previewは維持します。 |
| `CutDirect` | `Input`（必須）, `Mix` | 指定したInputを直接Programへ送ります（Previewは維持）。 |
| `Fade` | `Input`, `Mix`, `Duration` | Fadeトランジションを実行します。`Duration`はミリ秒単位（省略時は該当ユニットのプリセット値、未設定時は1000ms）。 |
| `PreviewInput` | `Input`（必須）, `Mix` | 指定したInputをPreviewに設定します。 |
| `ActiveInput` | `Input`（必須）, `Mix` | 指定したInputをProgramに設定します。 |
| `Snapshot` | `Value`, `Mix` | 指定Mixing UnitのProgram映像をスクリーンショットとして保存します。 |
| `SnapshotInput` | `Input`（必須）, `Value`, `Mix` | 指定したInputの映像をスクリーンショットとして保存します。 |

### 実行例

```text
# HTTP API
http://127.0.0.1:8088/api?Function=Fade&Duration=500
http://127.0.0.1:8088/api?Function=Cut&Input=3
http://127.0.0.1:8088/api?Function=Snapshot&Mix=1&Value=C:/Temp/eiviz.png

# TCP API
FUNCTION Fade Duration=500
FUNCTION Cut Input=3
```
