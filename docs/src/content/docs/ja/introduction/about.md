---
title: eivizについて
description: 開発モチベーションと技術選定
---

# eivizについて

## 開発モチベーション

eiviz は有志コミュニティおよび個人が開発・保守するアプリケーションです。  
[PolyForm Shield 1.0.0](https://github.com/MikanseiLaboratory/eiviz/blob/main/LICENSE) のもと、営利目的を含めて無料で利用できます。ソースコードは [GitHub](https://github.com/MikanseiLaboratory/eiviz) で公開しています。

開発の目的は、既存の映像配信ツールにはない機能や拡張を動く実装として示し、ソフトウェアスイッチャーの可能性を現場に伝えることです。導入のハードルを下げるため、無償で公開しています。

vMix や OBS Studio といった既存ソフトウェアの代替を目指してはいません。長年磨き込まれた製品と比べ、eiviz は新しい機能の投入を優先しています。品質やプロダクション向けの安定性でそれらを上回ることは、当初から想定していません。

そのため、2026年9月現在、eiviz の **プロダクション環境・本番配信での利用は非推奨** です。試験環境や、クラッシュしても差し支えない用途でのテストを前提にしてください。

## 技術選定

モダンな技術で、性能・操作感・クロスプラットフォームの可搬性を両立することを目標にしています。

映像合成の本体は Mixer（コア）に集約し、各 OS の UI から C ABI で呼び出します。GPU 経路は OS ごとに、wgpu の下にあるネイティブ API を使って最適化しています。

| 層 | 技術 |
| --- | --- |
| Mixer（コア） | Rust 1.97、wgpu 30 |
| Windows UI | .NET 10、C# 14、WPF |
| Windows GPU | Direct3D 12、Resizable BAR |
| macOS UI | Swift 6、SwiftUI |
| macOS GPU | Metal |
| Linux（実験的） | Rust、GTK 4（gtk4-rs）、Vulkan |

### Mixer

映像合成の中核を Mixer（コア）と呼びます。Rust + wgpu 30 で、クロスプラットフォームの GPU リアルタイム処理を行います。`cdylib` としてビルドし、C ABI 経由で各ホストから FFI 呼び出しできます。セッションファイルは Mixer が所有する JSON で、OS をまたいでも同じセッションとして開けます。

### Windows: .NET 10 / C# 14 / WPF / D3D12

Windows ホストは .NET 10 と C# 14、UI は WPF です。映像処理は GPU 側で行い、wgpu の抽象の下から Direct3D 12 の HAL を露出して最適化しています。

CPU から GPU へのフレーム転送には、NVIDIA などが提供する Resizable BAR（ReBAR）を使い、大容量データの VRAM 転送を短縮します。ReBAR 非対応環境では性能が大きく落ちるため、基本的には非推奨です。Windows on ARM には未対応です（[GitHub issue #80](https://github.com/MikanseiLaboratory/eiviz/issues/80)）。

### macOS: Swift 6 / SwiftUI / Metal

macOS ホストは Swift 6 / SwiftUI で、Mixer の `dylib` を C ABI 経由で呼び出します。描画は Metal です。Apple Silicon の Unified Memory と組み合わせると、Windows の ReBAR に近い転送特性が得られます。

:::note
開発チームに Apple Silicon Mac の常用者がいないため、実機での性能は未検証です。開発は Intel 世代の MacBook で行っています。
:::

Mac のディスクリート GPU サポートは、現時点では予定していません。

### Linux（実験的）: Rust / GTK 4 / Vulkan

:::caution
開発中の機能です。優先度は Windows / macOS より低く、本番利用は想定していません。
:::

Linux では Rust と GTK 4（gtk4-rs）、描画は Vulkan を想定しています。Vulkan の host-visible メモリで、Windows の ReBAR に近いアップロード経路を取る方針です。利用者と、Linux を主戦場にする映像オペレーターがチーム内に少ないため、サポート優先度は他プラットフォームより低くしています。

## 類似ソフトウェアの技術選定

業界で使われているソフトウェアスイッチャーのスタックを、参考までに短く挙げます。

:::note
2026年9月時点の公開情報に基づきます。
:::

### vMix

.NET Framework 4.8 と SlimDX（Direct3D 9）を使っているとみられます。現行の .NET / DirectX 世代と比べると、レガシーな資産に寄った構成です。Windows 専用です。

### OBS Studio

コアは C17 の libobs、UI は C++17 と Qt 6 です。合成レンダラは Windows が Direct3D 11、Linux と macOS の既定が OpenGL 3.3 以上、macOS Apple Silicon では Metal 3 が実験的に入ります。キャプチャや音声は OS ごとの実装を抽象し、近い操作感を出しています。GPL のため FFmpeg や x264 も積極的に使い、エンコードとメディア処理は既存資産に大きく依存しています。
