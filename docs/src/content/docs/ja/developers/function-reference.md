---
title: Function Reference
description: eivizの関数リファレンス
---

## vMix互換API

vMix互換HTTP API（`GET /api?Function=...`）で使えるShortcutです。`Input`はSceneのフラット番号、名前、GUIDのいずれかです。  
Cut/Fadeなど`Input`とMixing Unitが関連する一部のAPI操作では`Input`クエリにSceneのみ指定可能となり、Inputが指定された場合はエラーを返します。  
Inputの値は `0`で現在のPreview、`-1`で現在のProgramを指定可能です。`Mix`を省略するか`0`にすると選択中のMixing Unit、`1`以降はMixing Unitの番号と紐づきます。

保存パス`Value`の拡張子が`.jpg`/`.jpeg`ならJPEG、それ以外（省略時を含む）はPNGです。省略時はPictures（無ければ一時ディレクトリ）へ日時付きファイルを書きます。

| Function | 引数 | 動作 |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | PreviewをProgramへ切る。`Input`があれば先にPreviewへ載せてからCutする |
| `CutDirect` | `Input`（必須）, `Mix` | 指定InputをProgramへ直載せする。 |
| `Fade` | `Input`, `Mix`, `Duration` | Cutと同じ対象選択のあとFadeする。`Duration`はミリ秒。省略時は当該Mixing UnitのFadeプリセット、無ければ1000 |
| `PreviewInput` | `Input`（必須）, `Mix` | Previewを指定Inputにする。 |
| `ActiveInput` | `Input`（必須）, `Mix` | Programを指定Inputにする。 |
| `Snapshot` | `Value`, `Mix` | 指定Mixing UnitのProgramをスクリーンショットで保存する。 |
| `SnapshotInput` | `Input`（必須）, `Value`, `Mix` | 指定Inputのスクリーンショットを保存する。 |

例:

- `http://127.0.0.1:8088/api?Function=Fade&Duration=500`
- `http://127.0.0.1:8088/api?Function=CutDirect&Input=3`
- `http://127.0.0.1:8088/api?Function=Snapshot&Mix=1&Value=C:/Temp/eiviz.png`
- `http://127.0.0.1:8088/api?Function=SnapshotInput&Input=3&Value=C:/Temp/scene.jpg`
