---
title: System requirements and supported hardware
description: What eiviz needs to run, and which hardware is supported
---

This is the intended environment as of September 2026. It is a generation floor for the upload path, not a SKU catalog. Why that path exists is in [System architecture](/eiviz/en/introduction/architecture/). Stack choices are in [About eiviz](/eiviz/en/introduction/about/).

eiviz is not recommended for production.

Frame upload into compose textures assumes the CPU can write GPU memory directly, instead of a normal staging copy. That is Resizable BAR (ReBAR) on Windows, unified memory on Apple Silicon, and host-visible VRAM in Vulkan on Linux. The app may still launch without those, but that is not the performance we design for.

## Windows

This host ships. The target is 64-bit Windows 11 (x64). [Windows on ARM is not supported](https://github.com/MikanseiLaboratory/eiviz/issues/80).

A **discrete GPU with Resizable BAR enabled** is required. Integrated-only machines are out of scope. AMD calls the same feature Smart Access Memory (SAM).

| GPU | Floor (and newer) |
| --- | --- |
| NVIDIA | GeForce RTX 30 (Ampere), including 40 and 50 |
| AMD | Radeon RX 6000 (RDNA 2), including 7000 and 9000 |
| Intel | Arc A-series (Alchemist), including B-series |

Officially unsupported examples: GeForce GTX 10, GTX 16, and RTX 20. Laptops count only when the vendor exposes ReBAR.

The platform has to match:

- UEFI boot (not CSM / legacy BIOS)
- Above 4G Decoding and Resizable BAR (or SAM) enabled in firmware
- A CPU and chipset that expose it. Rule of thumb: Intel Core 10th gen or later, AMD Ryzen 3000 or later, on a board that actually offers the option
- Current GPU firmware and vendor driver

Confirm that the BAR size is close to total VRAM. If it is still 256 MB, this path is not active.

## macOS

This host ships. The target is **macOS 14 or later on Apple Silicon**. Every M1 and later Mac has unified memory.

Intel Macs, discrete GPUs, and eGPUs are out of scope. Intel-era iGPUs may report shared memory; that is not the path eiviz uses.

:::note
Nobody on the current team uses an Apple Silicon Mac day to day, so those performance claims are unverified. Development machines are Intel-generation MacBooks.
:::

## Linux

:::caution
There is no GPU backend or UI yet. This ranks below Windows and macOS, and it is not intended for production. The hardware below is what an implementation would assume.
:::

Both of the following are required:

1. **Vulkan Video** (decode at least), so files and UVC can be decoded on the GPU.
2. **Vulkan host-visible VRAM**. The CPU must be able to write device-local memory directly. On a discrete GPU that means the BAR covers all of VRAM, same as ReBAR on Windows.

The generation that satisfies both at once matches Windows:

| GPU | Floor (and newer) | Driver guidance |
| --- | --- | --- |
| NVIDIA | GeForce RTX 30 | Proprietary driver 535 or later. Vulkan Video can appear on older generations; ReBAR and Video together start here |
| AMD | Radeon RX 6000 | Mesa RADV. Vulkan Video exists from VCN 2 (RX 5000); full host-visible VRAM is reliable from RDNA 2. Mesa 25 turns Video on by default for VCN 2/3 |
| Intel | Arc A-series | Mesa ANV. Tiger Lake and later iGPUs can expose both Video and host-visible memory; the intended target is discrete Arc |

UEFI and Resizable BAR requirements are the same as Windows. The open-source NVIDIA stack (NVK) is not a baseline: its Vulkan Video coverage is still narrow as of 2026.
