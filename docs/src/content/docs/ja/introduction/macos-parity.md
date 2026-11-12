---
title: macOS UI パリティ
description: Windows版の画面とmacOS版の対応、未対応、Pro限定の範囲
---

# macOS UI パリティ

この表はWindows版の画面・操作とmacOS版の対応を整理するためのものです。**Windows NVIDIA GPU DX12とmacOS Metalが同じ性能になるという意味ではありません。** 性能差はGPU、メモリ帯域、冷却、ドライバの違いであり、機能パリティとは別に評価します。

## 状態

- **同等**: macOSで同じ操作ができる
- **Pro限定**: ライセンスとProモジュールがある場合だけ表示される
- **非対応**: macOSでは提供しない。UIにも出さない
- **要確認**: 実機スモークテストで確認中

## 画面

| Windows | macOS | 状態 |
|---|---|---|
| メイン / Inputs / Scenes / Mixing Units | ContentView | 同等（実機確認済み） |
| Add Input: Colours / Still / Video | AddInputView | 同等（実機確認済み） |
| Add Input: OMT / NDI / UVC | AddInputView | 同等（実機確認済み） |
| Add Input: Audio | AddInputView | 同等。マイク権限の事前要求を追加 |
| Add Input: Mix | AddInputView | 同等 |
| Add Input: DeckLink | 非表示 | 非対応。Pro読込時も非表示を実機確認 |
| Add Input: Replay | Pro時のみ | Pro限定 |
| Scene Editor / Scene Animation | CanvasEditors / SceneAnimView | 同等 |
| Overlay | OverlayEditor | 同等 |
| Mixing Unit | Dialogs | 同等 |
| Multiview / Multiview Slots | Dialogs / LayoutPresetMosaic | 同等 |
| Audio Input / Audio Aux | AudioInputSettings / Headphone | 同等 |
| Switcher | SwitcherWindow | 同等 |
| Preferences | PreferencesView | 同等。Cmd-,を接続済み |
| Settings: Display | SettingsView display | 同等。Default unit sizeを追加 |
| Settings: Performance | SettingsView performance | 同等（Metal/UMA向け） |
| Settings: Outputs | SettingsView outputs | 同等（OMT/NDI、Pro時RTMP） |
| Settings: Multiview | SettingsView multiview | 同等 |
| Settings: Audio Aux / Headphone | SettingsView headphone | 同等 |
| Settings: Web API | SettingsView webApi | 同等 |
| Settings: Advanced / License | SettingsView advanced/license | 同等 |
| Resource Monitor | ResourcesView | 同等 |
| Log | LogsView | 同等 |
| Connect | ConnectView | 同等 |
| Input Preview | InputPreview | 同等 |
| Color Picker | ColorPicker / ColorPick | 同等 |
| Custom WGSL | CustomWgslEditor | 同等 |
| Replay Window | ReplayView | Pro限定。Proテストモジュールで起動確認 |
| Replay Settings | ReplaySettingsView | Pro限定 |
| Replay Export | ReplayExportRunner | Pro限定 |
| RTMP出力 | SettingsView outputs | Pro限定。Pro時にtransportへ表示されることを確認 |
| DeckLink入出力 | 非表示 | 非対応。Pro時も非表示を実機確認 |

## Windows固有の設定

次はWindowsのGPU/ドライバ構成に固有で、macOSでは意味が異なるため表示しない。

- Direct3D 12 / Vulkan のレンダラー選択
- ReBAR / DXVA / Media Foundation の状態
- Windowsの出力エンコード・ストリーム一覧
- Internal color format（UYVY/BGRA）: macOSはUYVY固定で、WindowsのBGRA切替は表示しない。

macOSではMetal、VideoToolbox、UMAを使う。設定画面ではmacOSで有効な項目だけを表示する。

## 実機スモーク

`scripts/macos-ui-smoke.swift` は起動中のアプリのAXツリー、ウィンドウ一覧、スクリーンショットを保存する。

```bash
swift scripts/macos-ui-smoke.swift "$(pgrep -f eiviz-mac | head -1)" target/ui-smoke
swift scripts/macos-ui-smoke.swift "$(pgrep -f eiviz-mac | head -1)" --press "設定"
```

TCCのマイク許可とWindowServerが必要なためCIでは実行しない。許可後はAudio Inputの追加、メーター、L/R、デバイス変更を確認する。
