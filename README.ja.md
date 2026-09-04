# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**活発に開発中/不安定。**  
マルチM/Eの映像スイッチャーです。eivizは映像(eizou)+visualです。

[ドキュメント](https://mikanseilaboratory.github.io/eiviz/ja/) · [English](README.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| プラットフォーム | グラフィックス | 状態 |
| --- | --- | --- |
| Windows x64 | Direct3D 12 | 対応 |
| macOS | Metal | Tier 2 |
| Linux | Vulkan | 開発中 |

## インストール

[Download](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [すべてのリリース](https://github.com/MikanseiLaboratory/eiviz/releases)

**Windows x64:** zipを展開して`Eiviz.Host.exe`を実行します。  
**macOS Apple Silicon**（`macos-arm64`）: zipを展開して`./eiviz-mac`を実行します。  
**macOS Intel**（`macos-x64`）: 同じ手順です。x86_64/amd64向けです。

## できること

ライブ映像のミックス、スイッチ、オーバーレイ、送出です。各Mixing UnitにPreviewとProgramがあり、CUT、AUTO、Tバーで切り替えます。

ミキサーはRust + wgpu（WindowsはDX12、macOSはMetal）です。WindowsのUIはC# WPFです。MacホストはSwiftUIで、AppKitのNSViewに提示し、WPFと同じレイアウトです。セッションファイルはMixerコアが所有する正規のJSONなので、一方のOSで保存したファイルを他方でも同じセッションとして開けます。

入力: カラー、バー、静止画、動画ファイル、UVC、OMT、NDI®。  
出力: OMT（GPUまたはCPUエンコード）、NDI®（CPU/UYVY）。  
シーン、オーバーレイ、マルチビュー。音声バス: WindowsはWASAPI/ASIO、macOSはCore Audio。

## 技術スタック

| 層 | 技術 |
| --- | --- |
| Mixer（コア） | Rust 1.97、wgpu 30 |
| Windowsホスト | .NET 10、C# 14、WPF、Direct3D 12 |
| macOSホスト | Swift 6、SwiftUI、Metal |
| Linuxホスト | Rust、GTK 4、Vulkan — 開発中 |

## ビルド

Windowsでは.NET 10、Rust 1.97、DirectX 12対応GPU、[NDI SDK 6](https://ndi.video/for-developers/ndi-sdk/)（既定パス以外に入れた場合は`NDI_SDK_DIR`を設定）、bindgen用のLLVM/Clangが必要です。

```powershell
dotnet build eiviz.slnx -c Release
dotnet run --project host\Eiviz.Host.csproj -c Release
cargo test --manifest-path mixer\Cargo.toml
```

`dotnet build`が先にミキサーDLLをコンパイルします。

macOSではRust 1.97、Swift 6、[NDI SDK 6](https://ndi.video/for-developers/ndi-sdk/)（`/Library/NDI SDK for Apple`以外に入れた場合は`NDI_SDK_DIR`を設定）が必要です。

```bash
./mac/build.sh
# Intel:
# EIVIZ_MAC_ARCH=x86_64-apple-darwin ./mac/build.sh
```

## ライセンス

eiviz本体のソースは[PolyForm Shield License 1.0.0](LICENSE)です。

営利組織を含む内部利用は許可されています。eivizとの競合は許可されていません。このソフトウェアの実用的な代替となる映像スイッチャー（有償・無償を問わない）や、河村柊吾/未完成成果物研究所がこれを使って提供する他製品の代替を出荷することです。それには河村柊吾/未完成成果物研究所からの別ライセンスが必要です。

第三者のクレートとライブラリは、元のMIT/Apache-2.0/Zlibのままです。[NOTICE](NOTICE)と[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)をご参照ください。
