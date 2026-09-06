---
title: Function Reference
description: eivizの関数リファレンス
---

## vMix互換API

vMix互換HTTP API（`GET /api?Function=...`）で使えるShortcutです。`Input`はSceneのフラット番号、名前、GUIDのいずれかです。  
Cut/Fadeなど`Input`とMixing Unitが関連する一部のAPI操作では`Input`クエリにSceneのみ指定可能となり、Inputが指定された場合はエラーを返します。  
Inputの値は`0`で現在のPreview、`-1`で現在のProgramを指定可能です。`Mix`を省略するか`0`にすると選択中のMixing Unit、`1`以降はMixing Unitの番号と紐づきます。

保存パス`Value`の拡張子が`.jpg`/`.jpeg`ならJPEG、それ以外（省略時を含む）はPNGです。省略時はPictures（無ければ一時ディレクトリ）へ日時付きファイルを書きます。

| Function | 引数 | 動作 |
| --- | --- | --- |
| `Cut` | `Input`, `Mix` | PreviewをProgramへ切る。`Input`があればそのSceneをProgramへ直載せし、Previewは変えない |
| `CutDirect` | `Input`（必須）, `Mix` | 指定InputをProgramへ直載せする。Previewは変えない |
| `Fade` | `Input`, `Mix`, `Duration` | Cutと同じ対象選択のあとFadeする。`Input`があればPreviewは変えずProgramへFadeする。`Duration`はミリ秒。省略時は当該Mixing UnitのFadeプリセット、無ければ1000 |
| `PreviewInput` | `Input`（必須）, `Mix` | Previewを指定Inputにする。 |
| `ActiveInput` | `Input`（必須）, `Mix` | Programを指定Inputにする。 |
| `Snapshot` | `Value`, `Mix` | 指定Mixing UnitのProgramをスクリーンショットで保存する。 |
| `SnapshotInput` | `Input`（必須）, `Value`, `Mix` | 指定Inputのスクリーンショットを保存する。 |

例:

- `http://127.0.0.1:8088/api?Function=Fade&Duration=500`
- `http://127.0.0.1:8088/api?Function=Cut&Input=3`
- `http://127.0.0.1:8088/api?Function=CutDirect&Input=3`
- `http://127.0.0.1:8088/api?Function=Snapshot&Mix=1&Value=C:/Temp/eiviz.png`
- `http://127.0.0.1:8088/api?Function=SnapshotInput&Input=3&Value=C:/Temp/scene.jpg`

## C ABIパリティ

`mixer_*`は固定幅C ABIです。制御は`ControlService`へ収束し、presentation/data-planeはネットワーク公開しません。

| 分類 | 関数 | ネットワーク | 備考 |
| --- | --- | --- | --- |
| lifecycle | `mixer_create`/`mixer_destroy`/`mixer_ping` | いいえ | readinessはComposer初期化後。destroyは世代staticをreset |
| command（ライブ） | `mixer_unit_cut`/`mixer_unit_auto`/`mixer_unit_overlay_auto`/`mixer_unit_set_state`/`mixer_unit_set_custom_wgsl` | Cut/AutoはAPI可 | C ABIもControlService経由 |
| command（セッション） | `mixer_session_replace`/`mixer_create_unit`/`mixer_define_scene`/`mixer_define_generator`/`mixer_load_still`/`mixer_video_start`/`mixer_omt_connect`/`mixer_ndi_connect`/`mixer_output_add`/`mixer_audio_bus_upsert`など | ReplaceSessionとCRUDはAPI可 | ホストはreplaceを正本にする |
| query | `mixer_unit_get_state`/`mixer_session_load`/`mixer_session_canonicalize`/`mixer_poll_events`/`mixer_copy_stats` | GetSnapshot/Subscribe | pollは有界 |
| presentation | `mixer_unit_attach_native`/`mixer_attach_monitor_native`/`mixer_resize_*`/`mixer_detach_*` | いいえ | HWND/NSView |
| data-plane | `mixer_register_source`/`mixer_push_frame`/`mixer_push_audio`/`mixer_unit_acquire_frame`/`mixer_unit_release_frame` | いいえ | 固定幅のまま |
| diagnostics | `mixer_last_error`/`mixer_take_fatal`/`mixer_copy_rebar_info` | いいえ | fatal後は同一process再起動でreset |
| 互換 | `mixer_session_publish`/`mixer_api_configure` | vMix HTTP | publishはdeprecated wrapper |

`mixer_session_replace(json, len, expected_revision)`が正本適用です。`expected_revision=0`は無条件、それ以外はrevision競合で`CONFLICT`です。

