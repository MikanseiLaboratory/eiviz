# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**Under active development / Unstable.**  
A multi M/E vision mixer. eiviz / 映像(eizou) + visual.

[Documentation](https://mikanseilaboratory.github.io/eiviz/en/) · [日本語](README.ja.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| Platform | Graphics | Status |
| --- | --- | --- |
| Windows x64 | Direct3D 12 | Supported |
| macOS | Metal | Tier 2 |
| Linux | Vulkan | Under development |

## Install

[Download](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [all releases](https://github.com/MikanseiLaboratory/eiviz/releases)

**Windows x64:** unzip and run `Eiviz.Host.exe`.  
**macOS Apple Silicon** (`macos-arm64`): unzip and run `./eiviz-mac`.  
**macOS Intel** (`macos-x64`): same, for x86_64 / amd64.

## What it is

Mix, switch, overlay, and send live video. Each Mixing Unit has Preview and Program, plus CUT, AUTO, and a T-bar.

The mixer is Rust + wgpu (DX12 on Windows, Metal on macOS). The Windows UI is C# WPF. The Mac host is SwiftUI with AppKit NSView present, matching the WPF layout. Session files are canonical JSON owned by the mixer core, so a file saved on one OS loads as the same session on the other.

Inputs: colour, bars, still, video file, UVC, OMT, NDI®.  
Output: OMT (GPU or CPU encode), NDI® (CPU / UYVY).  
Scenes, overlays, multiview. Audio buses: WASAPI / ASIO on Windows, Core Audio on macOS.

## Stack

| Layer | Tech |
| --- | --- |
| Mixer (core) | Rust 1.97, wgpu 30 |
| Windows host | .NET 10, C# 14, WPF, Direct3D 12 |
| macOS host | Swift 6, SwiftUI, Metal |
| Linux host | Rust, GTK 4, Vulkan — under development |

## Build

Windows, .NET 10, Rust 1.97, a DirectX 12 GPU, the [NDI SDK 6](https://ndi.video/for-developers/ndi-sdk/) (set `NDI_SDK_DIR` if it is not installed to the default path), and LLVM/Clang for bindgen.

```powershell
dotnet build eiviz.slnx -c Release
dotnet run --project host\Eiviz.Host.csproj -c Release
cargo test --manifest-path mixer\Cargo.toml
```

`dotnet build` compiles the mixer DLL first.

macOS, Rust 1.97, Swift 6, and the [NDI SDK 6](https://ndi.video/for-developers/ndi-sdk/) (set `NDI_SDK_DIR` if it is not in `/Library/NDI SDK for Apple`):

```bash
./mac/build.sh
# Intel:
# EIVIZ_MAC_ARCH=x86_64-apple-darwin ./mac/build.sh
```

## License

eiviz original source is licensed under the [PolyForm Shield License 1.0.0](LICENSE).

Internal use is allowed, including at for-profit organizations. Competing with eiviz is not: shipping a vision mixer (paid or free) that is a practical substitute for this software, or for another product Shugo Kawamura / Mikansei Laboratory provides using it. A separate license from Shugo Kawamura / Mikansei Laboratory is required for that.

Third-party crates and libraries stay under their original MIT / Apache-2.0 / Zlib terms. See [NOTICE](NOTICE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
