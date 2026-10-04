---
title: システム要件とサポートするハードウェア
description: eivizが必要とする環境とサポートするハードウェア
---

eivizの動作環境とサポートハードウェアの一覧です。

本ソフトウェアは、CPUからGPUメモリ（VRAM）への直接書き込みが可能なハードウェア構成を前提としています。WindowsのResizable BAR（ReBAR）、macOSのUnified Memory、LinuxのVulkan host-visible VRAMがこれに該当します。

## Windows

- OS: Windows 11（x64）※[Windows on ARMは未対応](https://github.com/MikanseiLaboratory/eiviz/issues/80)
- GPU: **Resizable BARが有効なディスクリートGPU**（内蔵GPU環境は非推奨）
  - AMD環境では「Smart Access Memory（SAM）」と表記される場合があります。
  - Windows 11 24H2以前の一部環境では、GPU upload heapsに非対応のため最適化が機能しない場合があります。

| GPUベンダー | 推奨要件 |
| --- | --- |
| NVIDIA | GeForce RTX 3000シリーズ以降 |
| AMD | Radeon RX 6000シリーズ以降 |
| Intel | Arc Aシリーズ（Alchemist）以降 |

Direct3D 12対応GPUであれば基本動作は可能ですが、快適な運用のために上記推奨環境での使用をお勧めします。

## macOS

- OS: **macOS 14以降**
- 対応ハードウェア: **Apple Silicon（Mシリーズ）搭載Mac**
- Intel Macおよび外付けGPU（eGPU）は非対応です。

:::note
開発チームの機材都合により、現在Intel Macでのビルド・動作確認を主として進めており、Apple Silicon実機での最適化は順次進めています。
:::

## Linux（実験的）

:::caution
現在開発中のため、要件や仕様は今後変更される可能性があります。
:::

以下の2点を満たす環境を対象としています。

1. **Vulkan Video**: 動画デコードやカメラ入力をGPU上で処理するためのハードウェア機能
2. **Vulkan host-visible VRAM**: CPUからVRAM領域への直接アクセス（WindowsのReBAR相当）

| GPU | 最小要件 | ドライバの目安 |
| --- | --- | --- |
| NVIDIA | GeForce RTX 3000シリーズ以降 | ドライバv535以降 |
| AMD | Radeon RX 6000シリーズ以降 | Mesa RADV |
| Intel | Arc Aシリーズ以降 | Mesa ANV |
