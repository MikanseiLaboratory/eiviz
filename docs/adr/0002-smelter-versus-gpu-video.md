# ADR-0002 Smelter versus gpu-video

- Status: Accepted
- Date: 2026-08-23

## Context

Issue #1 は Smelter と gpu-video の利用を求める。Smelter 本体の LICENSE はリアルタイム処理と製品組込みに商用条項がある。gpu-video 0.4.0 は MIT で、wgpu 29 / Vulkan Video に結合する。公開 crate と master で wgpu 世代が異なる。

## Decision

1. 既定配布物に Smelter 本体を同梱しない。
2. 合成は eiviz 独自 compositor。backend は `CpuReference` と `Wgpu` を **明示選択** し、暗黙切替しない（ADR-0010）。
3. gpu-video は `eiviz-codec-gpu-video` からのみ import する optional adapter とする。
4. 商用ライセンスを取得した場合のみ、Smelter を別プロセス IPC として再評価する。
5. 既定 wgpu は GUI と共有する exact 25.0.2（ADR-0011）。wgpu 29 の gpu-video は隔離 feature のままとし、有効化する互換単位へ進むときは compositor と GUI を同一 PR で更新する。

## Consequences

「Smelter を使いたい」はライセンス適合時の拡張点として残る。Vulkan Video 非対応 GPU と macOS は **明示的な** software/OS codec profile を使う。gpu-video が要求されているのに software encode へ黙って落とすことはしない。
