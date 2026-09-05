---
title: Function Reference
description: eivizの関数リファレンス
---

## vMix互換API

vMix互換HTTP API（`GET /api?Function=...`）で使えるShortcutです。`Input`はSceneのフラット番号、名前、GUIDのいずれかです。Input単体はMixing Unitに載せられないので拒否します。存在しない番号も拒否します。`0`は現在のPreview、`-1`は現在のProgramです。`Mix`を省略するか`0`にすると選択中のMixing Unit、`1`始まりはMixing Unitの並びです。

| Function | 引数 | 動作 |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | PreviewをProgramへ切る。`Input`があれば先にPreviewへ載せてからCutする |
| `CutDirect` | `Input`（必須）, `Mix` | 指定InputをProgramへ直載せする。Previewは変えない |
| `Fade` | `Input`, `Mix`, `Duration` | Cutと同じ対象選択のあとFadeする。`Duration`はミリ秒。省略時は当該Mixing UnitのFadeプリセット、無ければ1000 |
| `PreviewInput` | `Input`（必須）, `Mix` | Previewを指定Inputにする |
| `ActiveInput` | `Input`（必須）, `Mix` | Programを指定Inputにする。Previewは変えない |
| `Snapshot` | `Value`, `Mix` | 選択中Mixing UnitのProgramをPNGとして保存する。`Value`は保存先パス。省略時はPictures（無ければ一時ディレクトリ）へ日時付きファイルを書く |

例:

- `http://127.0.0.1:8088/api?Function=Fade&Duration=500`
- `http://127.0.0.1:8088/api?Function=CutDirect&Input=3`
- `http://127.0.0.1:8088/api?Function=Snapshot&Value=C:/Temp/eiviz.png`
