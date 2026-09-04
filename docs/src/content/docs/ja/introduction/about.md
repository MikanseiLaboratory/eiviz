---
title: eivizについて
description: 開発モチベーションと技術選定
---

## 開発モチベーション

eivizは有志コミュニティおよび個人が開発・保守するアプリケーションです。  
[PolyForm Shield 1.0.0](https://github.com/MikanseiLaboratory/eiviz/blob/main/LICENSE)のもと、営利目的を含めて無料で利用できます。ソースコードは[GitHub](https://github.com/MikanseiLaboratory/eiviz)で公開しています。

開発の目的は、既存の映像配信ツールにはない機能や拡張を動く実装として示し、ソフトウェアスイッチャーの可能性を現場に伝えることです。導入のハードルを下げるため、無償で公開しています。

vMixやOBS Studioといった既存ソフトウェアの代替を目指してはいません。  
長年磨き込まれた製品と比べ、eivizは新しい機能の投入を優先しています。品質やプロダクション向けの安定性でそれらを上回ることは、当初から想定していません。

そのため、2026年9月現在、eivizの**プロダクション環境・本番配信での利用は非推奨**です。  
試験環境や、クラッシュしても差し支えない用途でのテストを前提にしてください。

## 技術選定

モダンな技術で、性能・操作感・クロスプラットフォームの可搬性を両立することを目標にしています。

映像合成の本体はMixer（コア）に集約し、各OSのUIからC ABIで呼び出します。  
GPU経路はOSごとに、wgpuの下にあるネイティブAPIを使って最適化しています。

| 層 | 技術 |
| --- | --- |
| Mixer（コア） | Rust 1.97、wgpu 30 |
| Windows UI | .NET 10、C# 14、WPF |
| Windows GPU | Direct3D 12、Resizable BAR |
| macOS UI | Swift 6、SwiftUI |
| macOS GPU | Metal |
| Linux（実験的） | Rust、GTK 4（gtk4-rs）、Vulkan |

### Mixer

映像合成の中核をMixer（コア）と呼びます。Rust + wgpu 30で、クロスプラットフォームのGPUリアルタイム処理を行います。  
`cdylib`としてビルドし、C ABI経由で各ホストから呼び出します。  
セッションファイルはMixerが所有するJSONで、OSをまたいでも同じセッションとして開けます。

### Windows: .NET 10 / C# 14 / WPF / D3D12

Windowsホストは.NET 10とC# 14、UIはWPFです。映像処理はGPU側で行い、wgpuの抽象の下からDirect3D 12を使って最適化しています。

CPUからGPUへのフレーム転送には、NVIDIAなどが提供するResizable BAR（ReBAR）を使い、映像のアップロード時にGPUのVRAM領域へ直接書き込みます。  
ReBAR非対応環境では性能が大きく落ちるため、基本的には非推奨です。Windows on ARMには未対応です（[GitHub issue #80](https://github.com/MikanseiLaboratory/eiviz/issues/80)）。

GPU upload heapsとResizable BARは別物です。Resizable BAR対応環境でも、Windows 11 24H2以前など一部の環境ではGPU upload heapsに非対応のため、この最適化が利用できない場合があります。

### macOS: Swift 6 / SwiftUI / Metal

macOSホストはSwift 6/SwiftUIで、Mixerの`dylib`をC ABI経由で呼び出します。描画はMetalです。  
Apple SiliconのUnified Memoryと組み合わせると、WindowsのReBARに近い転送特性が得られます。

:::note
開発チームにApple Silicon Macの常用者がいないため、実機での性能は未検証です。開発はIntel世代のMacBookで行っています。
:::

MacのディスクリートGPUサポートは、現時点では予定していません。

### Linux（実験的）: Rust/GTK 4/Vulkan

:::caution
開発中の機能です。優先度はWindows/macOSより低く、本番利用は想定していません。
:::

LinuxではRustとGTK 4（gtk4-rs）、描画はVulkanを想定しています。  
Vulkanのhost-visibleメモリで、WindowsのReBARに近いアップロード経路を取る方針です。  
利用者と、Linuxを主戦場にする映像オペレーターがチーム内に少ないため、サポート優先度は他プラットフォームより低くしています。

## 類似ソフトウェアの技術選定

業界で使われているソフトウェアスイッチャーのスタックを、参考までに短く挙げます。

:::note
2026年9月時点の公開情報に基づきます。
:::

### vMix

.NET Framework 4.8とSlimDX（Direct3D 9）を使っているとみられます。  
現行の.NET/DirectX世代と比べると、レガシーな資産に寄った構成です。Windows専用です。

### OBS Studio

コアはC17のlibobs、UIはC++17とQt 6です。  
合成レンダラはWindowsがDirect3D 11、LinuxとmacOSの既定がOpenGL 3.3以上、macOS Apple SiliconではMetal 3が実験的に入ります。  
キャプチャや音声はOSごとの実装を抽象し、近い操作感を出しています。GPLのためFFmpegやx264も積極的に使い、エンコードとメディア処理は既存資産に大きく依存しています。
