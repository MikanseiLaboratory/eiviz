---
title: システム要件とサポートするハードウェア
description: eivizが必要とする環境とサポートするハードウェア
---

2026年9月現在の想定環境です。SKUの網羅リストではなく、アップロード経路が成立する世代の下限です。なぜこの経路が必要かは[システムアーキテクチャ](/introduction/architecture/)を、技術選定は[eivizについて](/introduction/about/)を見てください。

本番配信での利用は非推奨です。

CPUから合成用テクスチャへフレームを載せるとき、通常のステージングではなく、CPUがGPUメモリへ直接書けることを前提にしています。WindowsではResizable BAR（ReBAR）、macOSではApple SiliconのUnified Memory、LinuxではVulkanのhost-visibleなVRAMがそれに当たります。足りない環境でも起動できることはありますが、想定性能にはなりません。

## Windows

実装済みです。64ビットのWindows 11（x64）を対象にします。[Windows on ARMは未対応](https://github.com/MikanseiLaboratory/eiviz/issues/80)です。

**外付けGPUでResizable BARが有効であること**が必須です。内蔵GPUだけのマシンは対象外です。AMDでは同じ機能がSmart Access Memory（SAM）と呼ばれることがあります。

| GPU | 下限（これ以降） |
| --- | --- |
| NVIDIA | GeForce RTX 30（Ampere）。40/50世代を含む |
| AMD | Radeon RX 6000（RDNA 2）。7000/9000世代を含む |
| Intel | Arc Aシリーズ（Alchemist）。Bシリーズを含む |

公式には乗らない例です。GeForce GTX 10、GTX 16、RTX 20はReBAR非対応です。ノートPCは、メーカーがReBARを出している機種に限ります。

プラットフォーム側も揃える必要があります。

- UEFI起動（CSM/レガシーBIOSは不可）
- ファームウェアでAbove 4G DecodingとResizable BAR（またはSAM）が有効
- CPUとチップセットがそれを出すこと。目安はIntel第10世代Core以降、AMD Ryzen 3000以降と、対応マザーボード
- GPU側のファームウェアと、現行のベンダードライバ

有効かどうかは、BARの大きさがVRAM全体に近いことで確認できます。256 MBのままだと、この経路は使えていません。

## macOS

実装済みです。**macOS 14以降のApple Silicon Mac**を対象にします。M1以降はいずれもUnified Memoryです。

Intel Mac、ディスクリートGPU、eGPUは対象外です。Intel世代の内蔵GPUは共有メモリを報告することがありますが、eivizが使う経路ではありません。

:::note
開発チームにApple Silicon Macの常用者がいないため、実機での性能は未検証です。開発はIntel世代のMacBookで行っています。
:::

## Linux

:::caution
GPUバックエンドもUIも未実装です。優先度はWindows/macOSより低く、本番利用は想定していません。以下は、実装時に前提にするハードウェアです。
:::

次の**両方**が必要です。

1. **Vulkan Video**（少なくともデコード）。ファイルやUVCをGPU上で解くためです。
2. **Vulkanのhost-visibleなVRAM**。CPUがデバイスローカルメモリへ直接書けること。外付けGPUでは、WindowsのReBARと同じくBARがVRAM全体に開いている状態です。

両方を同時に満たす目安は、Windowsと同じ世代です。

| GPU | 下限（これ以降） | ドライバの目安 |
| --- | --- | --- |
| NVIDIA | GeForce RTX 30 | プロプライエタリ535以降。Vulkan Videoはこれより古い世代でも出ることがありますが、ReBARと両立するのはこの世代からです |
| AMD | Radeon RX 6000 | MesaのRADV。Vulkan VideoはVCN 2（RX 5000）からありますが、host-visibleなVRAM全体はRDNA 2以降が確実です。Mesa 25以降でVCN 2/3のVideoが既定オンになります |
| Intel | Arc Aシリーズ | MesaのANV。Tiger Lake以降の内蔵GPUはVideoとhost-visibleの両方を出すことがありますが、想定は外付けのArcです |

UEFIとResizable BARの条件はWindowsと同じです。オープンソースのNVIDIA経路（NVK）のVulkan Videoは、2026年時点ではまだ狭いため前提にしません。
