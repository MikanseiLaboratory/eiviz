---
title: eivizについて
description: 開発動機と技術選定
---

## 開発の背景

eivizは有志コミュニティおよび個人開発者によって開発・保守されているソフトウェアスイッチャーです。[PolyForm Shield 1.0.0](https://github.com/MikanseiLaboratory/eiviz/blob/main/LICENSE)ライセンスのもと、商用利用を含めて無料で利用できます。ソースコードは[GitHub](https://github.com/MikanseiLaboratory/eiviz)で公開されています。

本プロジェクトは、vMixやOBS Studioなどの既存ソフトウェアを直接置き換えるものではなく、モダンな技術スタックを用いた新しいアーキテクチャや機能の可能性を検証・提示することを目的としています。現時点では実験的な実装が多く含まれるため、**本番配信環境での利用は非推奨**です。検証環境やテスト用途での利用を前提としています。

## 技術選定

高いパフォーマンス、操作性、クロスプラットフォーム対応の両立を目指して技術選定を行っています。

映像合成の中核処理はMixer（コア）に集約し、各OS向けのUIホストからC ABI経由で呼び出します。GPUパイプラインはwgpuを基盤としつつ、OSごとのネイティブAPIを活用して最適化しています。

| レイヤー | 採用技術 |
| --- | --- |
| Mixer（コア） | Rust、wgpu |
| Windows UI | .NET 10、C# 14、WPF |
| Windows GPU | Direct3D 12、Resizable BAR |
| macOS UI | Swift 6、SwiftUI |
| macOS GPU | Metal |
| Linux（実験的） | Rust、GTK 4（gtk4-rs）、Vulkan |

### Mixer

映像合成と音声処理、入出力管理を担うコアエンジンです。Rustとwgpuを用いてGPUを活用したリアルタイム処理を行います。セッションデータや外部制御API（vMix互換HTTP/TCP、Protobuf WebSocket）もMixer側で管理され、OS間で共通のセッションを扱えます。

### Windows: .NET 10 / C# 14 / WPF / D3D12

Windows向けUIホストはWPFで実装されています。GPUへの映像フレーム転送にはResizable BAR（ReBAR）を活用し、VRAM領域へ直接書き込むことで低遅延・高パフォーマンスを実現します。

※ReBAR非対応環境ではパフォーマンスが大幅に低下する場合があります。また、Windows on ARMには未対応です（[#80](https://github.com/MikanseiLaboratory/eiviz/issues/80)）。Windows 11 24H2以前の一部環境ではGPU upload heapsに対応していないため、この最適化を利用できません。

### macOS: Swift 6 / SwiftUI / Metal

macOS向けUIホストはSwiftUIで実装され、描画にはMetalを使用します。Apple SiliconのUnified Memoryアーキテクチャを活用し、メモリコピーのオーバーヘッドを削減しています。

:::note
現在、実機検証はIntel世代のMacBookを中心に行っており、Apple Silicon環境での動作確認を進めています。ディスクリートGPU搭載Macのサポートは予定していません。
:::

### Linux（実験的）: Rust/GTK 4/Vulkan

:::caution
開発中の実験的プラットフォームです。
:::

GTK 4とVulkanを用いた実装を進めています。Vulkanのhost-visibleメモリを活用し、WindowsのReBARと同様の効率的な転送経路を目指しています。
