# eiviz
[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

Work in progress. Multi M/E vision mixer for Windows.  
eiviz / 映像(eizou) + visual

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

[Download](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [all releases](https://github.com/MikanseiLaboratory/eiviz/releases)

Windows x64 zip. Unzip and run `Eiviz.Host.exe`.

## What it is

Mix, switch, overlay, and send live video. Each Mixing Unit has Preview and Program, plus CUT, AUTO, and a T-bar.

The mixer is Rust + wgpu (DX12). The UI is C# WPF. Proof of concept.

Inputs: colour, bars, still, video file, UVC, OMT, NDI®.  
Output: OMT (GPU or CPU encode), NDI® (CPU / UYVY).  
Scenes, overlays, multiview. Audio buses over WASAPI / ASIO.

## Build

Windows, .NET 10, Rust 1.97, a DirectX 12 GPU, the [NDI SDK 6](https://ndi.video/for-developers/ndi-sdk/) (set `NDI_SDK_DIR` if it is not installed to the default path), and LLVM/Clang for bindgen.

```powershell
dotnet build eiviz.slnx -c Release
dotnet run --project host\Eiviz.Host.csproj -c Release
cargo test --manifest-path mixer\Cargo.toml
```

`dotnet build` compiles the mixer DLL first.

## License

eiviz original source is licensed under the [PolyForm Shield License 1.0.0](LICENSE).

Internal use is allowed, including at for-profit organizations. Competing with eiviz is not: shipping a vision mixer (paid or free) that is a practical substitute for this software, or for another product Shugo Kawamura / Mikansei Laboratory provides using it. A separate license from Shugo Kawamura / Mikansei Laboratory is required for that.

Third-party crates and libraries stay under their original MIT / Apache-2.0 / Zlib terms. See [NOTICE](NOTICE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
