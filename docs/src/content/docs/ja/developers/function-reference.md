---
title: Function Reference
description: eivizの関数リファレンス
---

## vMix互換API

vMix互換HTTP APIおよびTCP APIで使用できる主なFunction（Shortcut）の一覧です。

- `Input`: Sceneの通し番号、名前、またはGUIDを指定します。CutやFadeなどのバス操作では通常のInputを直接割り当てられないため、Sceneのみ指定可能です（未定義の番号も拒否されます）。`0`で現在のPreview、`-1`で現在のProgramを指定できます。
- `Mix`: 操作対象のMixing Unit番号を指定します（省略時または`0`は現在選択中のユニット、`1`以降はセッション内の順序に対応）。
- `Value`: 保存先のファイルパスを指定します（拡張子が`.jpg`/`.jpeg`ならJPEG、それ以外や省略時はPNG）。省略時はピクチャフォルダまたは一時ディレクトリに日時付きで保存されます。

| Function | 引数 | 動作 |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | PreviewとProgramをCutで切り替えます。`Input`指定時は該当Sceneを直接Programへ送り、Previewは維持します。 |
| `CutDirect` | `Input`（必須）, `Mix` | 指定したInputを直接Programへ送ります（Previewは維持）。 |
| `Fade` | `Input`, `Mix`, `Duration` | Fadeトランジションを実行します。`Duration`はミリ秒単位（省略時は該当ユニットのプリセット値、未設定時は1000ms）。 |
| `PreviewInput` | `Input`（必須）, `Mix` | 指定したInputをPreviewに設定します。 |
| `ActiveInput` | `Input`（必須）, `Mix` | 指定したInputをProgramに設定します。 |
| `Snapshot` | `Value`, `Mix` | 指定したMixing UnitのProgram映像をスクリーンショットとして保存します（`Input`は使用しません）。 |
| `SnapshotInput` | `Input`（必須）, `Value`, `Mix` | 指定したInputの映像をスクリーンショットとして保存します（通し番号はSceneが先、続いて通常のInputとなり、`0`/`-1`は対象Mixing UnitのPreview/Programを表します）。 |

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

## リプレイ

リプレイのFunctionは、プロジェクトにリプレイ入力があり、ProfessionalライセンスとProモジュールが有効な場合に使用できます。`Channel`はリプレイチャンネルを英字（`A`, `B`…）または`0`からの番号で指定します。省略または空はチャンネルA、存在しないチャンネルは別のチャンネルを再生せず拒否されます。

| Function | 引数 | 動作 |
| --- | --- | --- |
| `ReplayStartRecording` | | 全アングルの録画を開始します。 |
| `ReplayStopRecording` | | 録画を停止します。 |
| `ReplayMarkIn` | | 録画位置にIN点をマークします。 |
| `ReplayMarkOut` | `Channel`, `Value` | OUT点をマークします。`Value`はイベント名です。イベントを登録し、セッションの設定に従ってプレイリストにも追加します。 |
| `ReplayCue` | `Channel`, `Value` | チャンネルにクリップをキューします。`Value`はクリップのUIDで、省略または空ならそのチャンネルのプレイリストの最新クリップをキューします。 |
| `ReplayPlay` | `Channel`, `Value` | キュー中のクリップを再生します。`Value`は等速を1とした倍率です（既定`1`、`0.5`で半速）。 |
| `ReplayPause` | `Channel` | 現在の速度を保ったまま一時停止します。 |
| `ReplayChangeSpeed` | `Channel`, `Value` | クリップを読み直さずに速度を変更して再生します。 |
| `ReplayJumpToNow` | `Channel` | チャンネルを録画位置（最新）に戻します。 |

### リプレイの実行例

```text
# HTTP API
http://127.0.0.1:8088/api?Function=ReplayStartRecording
http://127.0.0.1:8088/api?Function=ReplayMarkIn
http://127.0.0.1:8088/api?Function=ReplayMarkOut&Channel=A
http://127.0.0.1:8088/api?Function=ReplayCue&Channel=A
http://127.0.0.1:8088/api?Function=ReplayPlay&Channel=A&Value=0.5

# TCP API
FUNCTION ReplayMarkOut Channel=A
```
