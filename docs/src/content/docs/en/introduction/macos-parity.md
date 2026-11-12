---
title: macOS UI parity
description: Windows screens and their macOS equivalents, gaps, and Pro-only scope
---

# macOS UI parity

This table maps the Windows screens and actions to macOS. **It does not mean that Windows NVIDIA DX12 and macOS Metal have the same performance.** GPU, memory bandwidth, cooling, and driver differences are evaluated separately from feature parity.

## Status

- **Equivalent**: the same action is available on macOS
- **Pro only**: shown only with a license and the Pro module
- **Unsupported**: not provided on macOS and not shown in the UI
- **Verifying**: still being exercised by the GUI smoke test

## Screens

| Windows | macOS | Status |
|---|---|---|
| Main / Inputs / Scenes / Mixing Units | ContentView | Equivalent (checked on device) |
| Add Input: Colours / Still / Video | AddInputView | Equivalent (checked on device) |
| Add Input: OMT / NDI / UVC | AddInputView | Equivalent (checked on device) |
| Add Input: Audio | AddInputView | Equivalent; microphone permission is requested first |
| Add Input: Mix | AddInputView | Equivalent |
| Add Input: DeckLink | Hidden | Unsupported; the macOS DeckLink headers do not fit |
| Add Input: Replay | Pro only | Pro only |
| Scene Editor / Scene Animation | CanvasEditors / SceneAnimView | Equivalent |
| Overlay | OverlayEditor | Equivalent |
| Mixing Unit | Dialogs | Equivalent |
| Multiview / Multiview Slots | Dialogs / LayoutPresetMosaic | Equivalent |
| Audio Input / Audio Aux | AudioInputSettings / Headphone | Equivalent |
| Switcher | SwitcherWindow | Equivalent |
| Preferences | PreferencesView | Equivalent; Cmd-, is wired |
| Settings: Display | SettingsView display | Equivalent; default unit size was added |
| Settings: Performance | SettingsView performance | Equivalent (Metal/UMA) |
| Settings: Outputs | SettingsView outputs | Equivalent (OMT/NDI; RTMP with Pro) |
| Settings: Multiview | SettingsView multiview | Equivalent |
| Settings: Audio Aux / Headphone | SettingsView headphone | Equivalent |
| Settings: Web API | SettingsView webApi | Equivalent |
| Settings: Advanced / License | SettingsView advanced/license | Equivalent |
| Resource Monitor | ResourcesView | Equivalent |
| Log | LogsView | Equivalent |
| Connect | ConnectView | Equivalent |
| Input Preview | InputPreview | Equivalent |
| Color Picker | ColorPicker / ColorPick | Equivalent |
| Custom WGSL | CustomWgslEditor | Equivalent |
| Replay Window | ReplayView | Pro only |
| Replay Settings | ReplaySettingsView | Pro only |
| Replay Export | ReplayExportRunner | Pro only |
| RTMP output | SettingsView outputs | Pro only |
| DeckLink I/O | Hidden | Unsupported |

## Windows-only settings

These are specific to the Windows GPU/driver stack and are not shown on macOS:

- Direct3D 12 / Vulkan renderer selection
- ReBAR / DXVA / Media Foundation state
- Windows output-encode and stream lists
- Internal color format (UYVY/BGRA): macOS is fixed to UYVY; the Windows BGRA switch is not shown.

macOS uses Metal, VideoToolbox, and unified memory. The settings screen shows only the macOS-relevant controls.

## GUI smoke

`scripts/macos-ui-smoke.swift` records the accessibility tree, window list, and screenshots of a running app.

```bash
swift scripts/macos-ui-smoke.swift "$(pgrep -f eiviz-mac | head -1)" target/ui-smoke
swift scripts/macos-ui-smoke.swift "$(pgrep -f eiviz-mac | head -1)" --press "設定"
```

It is not a CI test because it needs WindowServer and TCC microphone approval. After approval, add an Audio Input and check the meter, L/R, and device changes.
