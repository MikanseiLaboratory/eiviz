---
title: About eiviz
description: Motivation and technology choices
---

## Motivation

eiviz is developed and maintained by volunteers and individuals.  
It is free to use, including for commercial purposes, under the [PolyForm Shield 1.0.0](https://github.com/MikanseiLaboratory/eiviz/blob/main/LICENSE) license. Source is on [GitHub](https://github.com/MikanseiLaboratory/eiviz).

The goal is to ship working implementations of features that existing live-production tools do not offer, and to show what a software vision mixer can do in the field. It is published at no cost so that teams can try that model without a purchase barrier.

It is not meant to replace vMix, OBS Studio, or similar products.  
Those tools have been hardened over many years. eiviz prefers landing new capabilities first, and was never expected to match them on production quality or reliability.

As of September 2026, **eiviz is not recommended for production or live shows**.  
Use it in tests, labs, and anywhere a crash is acceptable.

## Technology choices

The stack aims for performance, a native feel, and portability across operating systems.

Compositing lives in the mixer core. Each OS UI calls it through a C ABI.  
GPU paths drop through wgpu to the native API on that platform and apply extra optimization there.

| Layer | Stack |
| --- | --- |
| Mixer (core) | Rust 1.97, wgpu 30 |
| Windows UI | .NET 10, C# 14, WPF |
| Windows GPU | Direct3D 12, Resizable BAR |
| macOS UI | Swift 6, SwiftUI |
| macOS GPU | Metal |
| Linux (experimental) | Rust, GTK 4 (gtk4-rs), Vulkan |

### Mixer

The compositing engine is the mixer (core). It is Rust + wgpu 30, for real-time GPU work on every supported OS.  
It builds as a `cdylib` and is called over a C ABI.  
Session files are canonical JSON owned by the mixer, so a file saved on one OS loads as the same session on another.

### Windows: .NET 10 / C# 14 / WPF / D3D12

The Windows host is .NET 10 and C# 14, with a WPF UI. Video work runs on the GPU. The host reaches through wgpu’s abstraction to Direct3D 12 for extra optimization.

CPU-to-GPU frame upload uses Resizable BAR (ReBAR), as provided by NVIDIA and others, and writes frames straight into GPU VRAM.  
Machines without ReBAR lose a lot of that gain, so they are not a recommended baseline. Windows on ARM is not supported ([GitHub issue #80](https://github.com/MikanseiLaboratory/eiviz/issues/80)).

GPU upload heaps are not the same as Resizable BAR. Even on a ReBAR-capable machine — for example Windows 11 before 24H2 — GPU upload heaps may be unavailable, so this optimization cannot be used.

### macOS: Swift 6 / SwiftUI / Metal

The macOS host is Swift 6 / SwiftUI and calls the mixer `dylib` through the C ABI. Drawing uses Metal.  
On Apple Silicon, unified memory gives transfer characteristics close to ReBAR on Windows.

:::note
Nobody on the current team uses an Apple Silicon Mac day to day, so those performance claims are unverified. Development machines are Intel-generation MacBooks.
:::

Discrete GPUs on Mac are not planned.

### Linux (experimental): Rust / GTK 4 / Vulkan

:::caution
This path is still in development. It is a lower priority than Windows and macOS, and it is not intended for production.
:::

The Linux host is planned as Rust and GTK 4 (gtk4-rs), with Vulkan for drawing.  
The upload path is meant to use Vulkan host-visible memory, in the same spirit as ReBAR on Windows.  
There are few Linux users so far, and nobody on the team operates video on Linux as their main platform, so support ranks below the other two.

## Technology choices in similar software

A short snapshot of stacks used by other software vision mixers.

:::note
Based on public information as of September 2026.
:::

### vMix

vMix appears to use .NET Framework 4.8 and SlimDX (Direct3D 9).  
Relative to current .NET and DirectX, that is a legacy-heavy stack. It is Windows-only.

### OBS Studio

The core is C17 libobs. The UI is C++17 and Qt 6.  
The compositor uses Direct3D 11 on Windows, OpenGL 3.3+ by default on Linux and macOS, and experimental Metal 3 on Apple Silicon Macs.  
Capture and audio are abstracted per OS so the UX stays close across platforms. Because it is GPL, it also leans on FFmpeg and x264, and encoding plus media handling reuse a large existing codebase.
