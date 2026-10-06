# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**Under active development / Unstable.**  
A cross-platform vision mixer with unlimited M/E. eiviz / 映像(eizou) + visual.

[Documentation](https://mikanseilaboratory.github.io/eiviz/en/) · [日本語](README.ja.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| Platform | Graphics | Status |
| --- | --- | --- |
| Windows x64 | Direct3D 12 or Vulkan | Supported |
| macOS | Metal | Supported (untested) |
| Linux | Vulkan | Under development |

## Install

[Download](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [all releases](https://github.com/MikanseiLaboratory/eiviz/releases)

### Windows x64

Run `eiviz-*-win-x64-setup.exe`.

### macOS Apple Silicon (`macos-arm64`)

Run `eiviz-*-macos-arm64.pkg`.

### macOS Intel (`macos-x64`)

Same steps, with the `macos-x64` pkg.

### Linux

Linux currently ships as CLI (headless) only. Build from source and run `eiviz-headless`. See [Headless](https://mikanseilaboratory.github.io/eiviz/en/features/headless/). A Release build is strongly recommended for performance.

## Develop

An executable with no `eiviz-pro.required` beside it is the OSS build. It does not search for a Pro module and uses the Community limits.

Rust 1.97. The Windows host also needs .NET 10 and the NDI SDK 6 runtime DLL.

```bat
cargo test --workspace --locked
dotnet build hosts\win32\Eiviz.Host.csproj
```

`dotnet build` always builds the release `eiviz_mixer.dll` and copies it next to the host, including a Debug configuration.

```bat
hosts\win32\bin\Debug\net10.0-windows\Eiviz.Host.exe
```

The Release configuration writes `hosts\win32\bin\Release\net10.0-windows\Eiviz.Host.exe`.

Headless initializes the GPU and serves the control API:

```bat
cargo run -p eiviz-headless --features runtime --release --locked -- run
```

On macOS, from `hosts/macos`:

```bash
swift build -c release
```

Run the `eiviz-mac` product. `swift build --show-bin-path` prints its directory.

## What it is

A next-generation production graphics tool, built by video operators for video operations.

eiviz uses a modern architecture to implement features and cross-platform support that existing video operation tools and software vision mixers struggle with, and to provide high capability and extensibility. It is also designed from the awkward parts of those mixers, so it is easier to use from amateur to professional work.

eiviz is not a company product. It is developed and operated by the main maintainer with community support.

## AI in development

This tool uses AI/LLM in development.

It is not a vibe-coding application. LLMs are used as tools to assist at key points and for tasks that need them. They do not lead the development. The core maintainer is responsible for the architecture and the implementation. Only code that has passed maintainer review is used.

## License

eiviz original source is licensed under the [PolyForm Shield License 1.0.0](LICENSE).

Internal use is allowed, including at for-profit organizations. Competing with eiviz is not: shipping a vision mixer (paid or free) that is a practical substitute for this software, or for another product Shugo Kawamura / Mikansei Laboratory provides using it. A separate license from Shugo Kawamura / Mikansei Laboratory is required for that.

Third-party crates and libraries stay under their original MIT / Apache-2.0 / Zlib terms. See [NOTICE](NOTICE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
