# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**Under active development / Unstable.**  
A cross-platform vision mixer with unlimited M/E. eiviz / 映像(eizou) + visual.

[Documentation](https://mikanseilaboratory.github.io/eiviz/en/) · [日本語](README.ja.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| Platform | Graphics | Status |
| --- | --- | --- |
| Windows x64 | Direct3D 12 | Supported |
| macOS | Metal | Supported (untested) |
| Linux | Vulkan | Under development |

## Install

[Download](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [all releases](https://github.com/MikanseiLaboratory/eiviz/releases)

### Windows x64

Run `eiviz-*-win-x64-setup.exe`.  
Or unzip the zip and run `Eiviz.Host.exe`.

### macOS Apple Silicon (`macos-arm64`)

Run `eiviz-*-macos-arm64.pkg` to install into `/Applications`.

Or unzip the zip, then:

```bash
cd eiviz-*-macos-arm64
xattr -cr .
open eiviz-mac.app
```

The `.app` is required for NDI® discovery and send. If macOS blocks it: System Settings → Privacy & Security → Open Anyway.

### macOS Intel (`macos-x64`)

Same steps, with the `macos-x64` pkg or zip.

## What it is

A next-generation production graphics tool, built by video operators for video operations.

A modern architecture lets eiviz ship features — and real cross-platform support — that existing video-ops tools and software vision mixers have struggled to implement, while staying capable and extensible. It is also designed around what those mixers get wrong: easier to work with, and usable from amateur productions through professional shows.

eiviz is not a company product. It is developed and operated by the main maintainer, with support from the community.

## License

eiviz original source is licensed under the [PolyForm Shield License 1.0.0](LICENSE).

Internal use is allowed, including at for-profit organizations. Competing with eiviz is not: shipping a vision mixer (paid or free) that is a practical substitute for this software, or for another product Shugo Kawamura / Mikansei Laboratory provides using it. A separate license from Shugo Kawamura / Mikansei Laboratory is required for that.

Third-party crates and libraries stay under their original MIT / Apache-2.0 / Zlib terms. See [NOTICE](NOTICE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
