# 技術スタック（固定版）

本ファイルが依存関係の source of truth です。版変更は ADR を先に更新します。

| コンポーネント | 版 | 備考 |
|---|---|---|
| Rust | 1.97.0 MSRV / CI 1.98 + MSRV check | edition 2024 |
| wgpu（eiviz-gpu直接依存） | =25.0.2 | eframe render_state と同一 Device/Queue。headless/HIL のみ別 device（ADR-0011） |
| egui / eframe | 0.32.3 | `egui-wgpu`のnative texture registryでcompositor outputを直接表示 |
| serde / serde_json | 1.0.219 / 1.0.143 | 永続化・制御面 |
| uuid | 1.18.1 | v7 IDs |
| image | 0.25.6 | PNG/JPEG 入力 |
| shiguredo_mp4 | 2026.4.0 exact | MP4/fMP4 demux、Apache-2.0 |
| openh264-sys2 | 0.9.8 exact | `libloading`のみ。Cisco 2.6.0外部binary、source build禁止 |
| libloading | 0.9.x | 明示pathのlicense-reviewed FDK AAC C ABI dynamic adapter。FDK source/binary非同梱、patent grantなし |
| yuv | 0.8.17 exact | 固定profileのlimited-range BT.709 I420→RGBA変換。MP4 color metadata検証は未認定 |
| zip | 4.3.x | portable `.eiviz` |
| tiny_http / tungstenite | 0.12.0 / 0.30.0 | authenticated HTTP + WebSocket control API |
| midir | 0.11.0 | optional `midi` feature。WinMM/CoreMIDI/ALSA native input only |
| gpu-video | 0.4.0 | **optional feature only**, MIT, wgpu 29 |
| Smelter | 非同梱 | ADR-0002 |
| openmediatransport-rs | git rev 2a0a9d31 | Pure Rust OMT、MIT、HIL pending |
| grafton-ndi | 1.0.0 exact | Apache-2.0 wrapper。NDI 6 SDK headers/runtimeは別途必要。`ndi` feature、HIL pending |
| rml_rtmp | 0.8.0 exact | pure Rust RTMP session、MIT、upstream MSRV未宣言、Rust 1.97確認 |
| srt-tokio | 0.4.4 exact | pure safe Rust SRT caller、Apache-2.0、upstream MSRV未宣言、Rust 1.97確認、HIL pending |
| DeckLink / ASIO | feature + HIL | ADR-0007 |

認定プロファイル: 1920×1080p, `60000/1001` fps, SDR BT.709 8-bit, 48 kHz。
