# ADR-0002 Smelter versus gpu-video

- Status: Accepted
- Date: 2026-08-23

## Context

Issue #1 は Smelter と gpu-video の利用を求める。Smelter 本体の LICENSE はリアルタイム処理と製品組込みに商用条項がある。gpu-video 0.4.0 は MIT で、wgpu 29 / Vulkan Video に結合する。公開 crate と master で wgpu 世代が異なる。

## Decision

1. 既定配布物に Smelter 本体を同梱しない。
2. 合成は eiviz 独自 compositor。backend は `CpuReference` と `Wgpu` を **明示選択** し、暗黙切替しない（ADR-0010）。
3. gpu-video の capability stub は削除する。wgpu 25 と型・所有権を共有できる実装が
   存在しないため、optional capability として表示しない（ADR-0015）。
4. 商用ライセンスを取得した場合のみ、Smelter を別プロセス IPC として再評価する。
5. 既定 wgpu は GUI と共有する exact 25.0.2（ADR-0011）。将来 codec adapter
   を追加する場合は compositor と GUI を同一 wgpu 世代へ更新し、実adapterと
   HILを同時に提出する。

## Consequences

「Smelter を使いたい」はライセンス適合時の拡張点として残る。現行製品は
**明示的な** software/external codec profile を使う。gpu-video選択肢自体を
公開しないため、software encodeへの暗黙fallbackも発生しない。
