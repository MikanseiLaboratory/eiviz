---
title: System requirements and supported hardware
description: What eiviz needs to run, and which hardware is supported
---

Recommended environment as of September 2026. Why this stack exists is in [About eiviz](/eiviz/en/introduction/about/).

Frame upload assumes the CPU can write GPU memory directly, instead of a normal slow path (256 MB windows).  
That is Resizable BAR (ReBAR) on Windows, unified memory on Apple Silicon, and host-visible VRAM in Vulkan on Linux.

## Windows

The target is Windows 11 (x64). [Windows on ARM is not supported](https://github.com/MikanseiLaboratory/eiviz/issues/80).

A **discrete GPU with Resizable BAR enabled** is required. Integrated-only machines are out of scope.  
AMD calls the same feature Smart Access Memory (SAM).

GPU upload heaps are not the same as Resizable BAR. Even on a ReBAR-capable machine — for example Windows 11 before 24H2 — GPU upload heaps may be unavailable, so this optimization cannot be used.

| GPU | Recommended |
| --- | --- |
| NVIDIA | GeForce RTX 3000 series or later |
| AMD | Radeon RX 6000 series or later |
| Intel | Arc A-series (Alchemist) or later |

Any GPU with Direct3D 12 support can run.

## macOS

The target is **macOS 14 or later on Apple Silicon**.

Intel Macs and external GPUs are out of scope.

:::note
Nobody on the current team uses an Apple Silicon Mac day to day, so those performance claims are unverified. Development machines are Intel-generation MacBooks.
:::

## Linux

:::caution
This platform is not implemented yet. The notes below may change without notice and do not describe a shipping build.
:::

Both of the following are required:

1. **Vulkan Video** — a path that ties video files and USB Video Capture to the GPU.
2. **Vulkan host-visible VRAM** — the CPU must be able to write VRAM directly through Vulkan. Same idea as ReBAR on Windows.

The generation that satisfies both at once matches Windows:

| GPU | Floor | Driver guidance |
| --- | --- | --- |
| NVIDIA | GeForce RTX 3000 series or later | v535 or later |
| AMD | Radeon RX 6000 series or later | Mesa RADV or later |
| Intel | Arc A-series or later | Mesa ANV or later |
