# ADR-0009 Phase 0 proof-of-concept gates

- Status: Accepted
- Date: 2026-08-23

## Context

実 GPU と vendor SDK が無いクラウド環境では、Vulkan Video / DeckLink / ASIO のゼロコピー測定はできない。それでも backend 選択を凍結する必要がある。

## Decision

| 項目 | 結果 |
| --- | --- |
| Smelter 本体 | 非同梱。ライセンスゲート未通過 |
| gpu-video 0.4.0 | optional adapter。既定実行経路にしない |
| compositor | CPU reference + wgpu。CI は CPU |
| GUI | egui 0.32 / wgpu 24 |
| file media | image crate + software frame source |
| NDI/OMT/DeckLink/ASIO | feature + mock。実機は HIL |
| RTMP/SRT/MP4 | 内蔵 muxer。GStreamer は後続 feature |
| 認定 profile | 1080p59.94 SDR / 48 kHz |

copy 回数や P99 GPU 時間は HIL で測定し、本 ADR を改訂するまでゼロコピーを前提にしない。

## Consequences

本リポジトリの default `cargo test` は SDK/GPU なしで AC-01..AC-08 の決定論部分を保証する。
