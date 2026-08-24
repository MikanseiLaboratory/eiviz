# ADR-0009 Phase 0 proof-of-concept gates

- Status: Accepted
- Date: 2026-08-23

## Context

実 GPU と vendor SDK が無いクラウド環境では、Vulkan Video / DeckLink / ASIO のゼロコピー測定はできない。それでも backend 選択を凍結する必要がある。

## Decision

| 項目 | 結果 |
| --- | --- |
| Smelter 本体 | 非同梱。ライセンスゲート未通過 |
| gpu-video 0.4.0 | 非採用。wgpu 29 と単一 wgpu 25 は共有不能、stub削除（ADR-0015） |
| compositor | 明示 `CpuReference` と `Wgpu`。CI は CpuReference profile（fallback ではない） |
| GUI | egui 0.32 / exact wgpu 25.0.2; shared desktop Device/Queue (ADR-0011) |
| file media | image + Shiguredo MP4 + explicit dynamic OpenH264 2.6 profile |
| OMT | pure-Rust openmediatransport-rs/vmx-rs。実機interopはHIL |
| NDI/DeckLink/Audio | 実adapterをfeature隔離。SDK/device HIL pending |
| RTMP/SRT/fMP4 | bounded fanout/transport/recovery実装。production AVC/AAC encoder pending |
| 認定 profile | 1080p59.94 SDR / 48 kHz |

copy 回数や P99 GPU 時間は HIL で測定し、本 ADR を改訂するまでゼロコピーを前提にしない。

## Consequences

本リポジトリの default `cargo test` は SDK/GPU なしで AC-01..AC-08 の決定論部分を保証する。
