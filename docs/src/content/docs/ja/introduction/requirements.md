---
title: システム要件とサポートするハードウェア
description: eivizが必要とする環境とサポートするハードウェア
---

2026年9月現在の推奨動作環境です。技術選定の背景は[eivizについて](/eiviz/ja/introduction/about/)をご参照ください。

CPUからGPUへ映像を転送するとき、通常の低速転送（256MB単位）ではなく、CPUがGPUメモリへ直接書けることを前提にしています。  
WindowsではResizable BAR（ReBAR）、macOSではApple SiliconのUnified Memory、LinuxではVulkanのhost-visibleなVRAMがそれに当たります。

## Windows

Windows 11（x64）を対象にします。[Windows on ARMは未対応](https://github.com/MikanseiLaboratory/eiviz/issues/80)です。

**外付けGPUでResizable BARが有効であること**が必須です。内蔵GPUだけのマシンは対象外です。  
AMDでは同じ機能がSmart Access Memory（SAM）と呼ばれることがあります。

| GPU | 推奨要件 |
| --- | --- |
| NVIDIA | GeForce RTX 3000世代以降 |
| AMD | Radeon RX 6000世代以降 |
| Intel | Arc Aシリーズ（Alchemist）以降 |

DX12がサポートされているGPUであれば動作は可能です。

## macOS

**macOS 14以降のApple Silicon Mac**を対象にしています。

Intel Mac、外部GPUは対象外です。

:::note
開発チームにApple Silicon Macの常用者がいないため、実機での性能は未検証です。開発はIntel世代のMacBookで行っています。
:::

## Linux

:::caution
現在未実装のプラットフォームです。以下の内容は予告なく変更される場合があり、現在の実装の実態に沿ったものではありません。
:::

次の**両方**が必要です。

1. **Vulkan Video**動画ファイルやUSB Video CaptureをGPUに直結するための回路です。
2. **Vulkan Host-visible VRAM** CPUがVulkan経由でVRAMへデータを直接書けること。WindowsのReBARと同等の機能です。

両方を同時に満たす目安は、Windowsと同じ世代です。

| GPU | 最小要件 | ドライバの目安 |
| --- | --- | --- |
| NVIDIA | GeForce RTX 3000世代以降 | v535以降 |
| AMD | Radeon RX 6000世代以降 | Mesa RADV以降 |
| Intel | Arc Aシリーズ以降 | Mesa ANV以降 |
