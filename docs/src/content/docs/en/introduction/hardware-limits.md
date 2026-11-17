---
title: Hardware limits
description: Constraints on GPU encoders, decode resources, and memory bandwidth
---

eiviz drives the GPU's encode and decode engines directly, so how many sessions run at once and how much bandwidth is available is decided by the hardware and driver, not by a setting. This page lists what is known per OS and GPU, and how the limit shows up in eiviz.

:::note
**No public API reports the concurrency limit** for VideoToolbox, NVENC, or Media Foundation. What the mixer can report is which encoder kinds this build can start (`stream_encoder_mask` in `mixer_capabilities`) and the list of Media Foundation devices. When the limit is reached, the driver refuses the session at start and the reason appears in the log and on the stream strip.
:::

## macOS: VideoToolbox

- **The number of concurrent sessions is not documented.** `kVTCompressionPropertyKey_RecommendedParallelizationLimit` is the recommended number of sessions when splitting *one* encode across several sessions, not a limit on simultaneous streams ([Apple Developer](https://developer.apple.com/documentation/videotoolbox/kvtcompressionpropertykey_recommendedparallelizationlimit)).
- **Decode and encode share the same hardware.** eiviz decodes video files and camera input through AVFoundation (VideoToolbox internally) and encodes the stream with VideoToolbox. Running both can leave the encoder callback waiting, so the stream logs `VideoToolbox is not responding; dropping frames for …` and stays at **0 fps**.
- Workarounds:
  - Set the stream encode profile to **OpenH264** (CPU encoding), which keeps working while the video and camera inputs stay open. OpenH264 is fetched on first use.
  - Stop or remove the input that holds VideoToolbox, then start the stream.
  - Set the input **decode** to software (the Video/UVC "Decode" setting). The camera's own conversion still runs inside AVFoundation, so this is not a complete CPU switch.
- When the hardware encoder is missing or does not answer, the module refuses with a reason. It never switches to another encoder on its own.

## Windows: NVENC / Media Foundation

- **NVENC on GeForce** is limited in how many simultaneous encodes it allows; the number depends on the generation (three concurrent streams was the official GeForce figure for a period). The limit is not queryable and the driver refuses the extra session ([NVENC](https://en.wikipedia.org/wiki/NVENC)). Quadro/RTX A and datacenter parts are less restricted.
- **Media Foundation** uses the OS encoder, either a GPU-vendor hardware MFT or the OS software MFT. The available devices are listed by `mixer_stream_encoder_devices` and appear as **Encoder device** in the settings. The concurrency limit is not public; an over-limit start is refused.
- **Vulkan Video** (experimental on Linux, used for input decode on Windows) also depends on the driver for codecs and session count.

## Memory bandwidth and PCIe

- **Windows (discrete GPU)**: input frames cross PCIe into the GPU. 4K60 UYVY is about 1.0 GB/s and BGRA about 2.0 GB/s, so several 4K inputs at once make PCIe and VRAM transfer the bottleneck. Resizable BAR, when enabled, lets the CPU write straight into VRAM and eases this transfer ([System requirements](./requirements/)).
- **macOS (Apple Silicon)**: Unified Memory removes the CPU/GPU copy, but the GPU's memory bandwidth is shared by composition, decode, and encode. Many 4K inputs at once cause dropped frames.
- **Linux (experimental)**: requires Vulkan host-visible VRAM ([System requirements](./requirements/)).

## DeckLink

DeckLink input and output counts are set by the **plan**, not only by the card's ports ([Plans](./plans/)). Adding beyond the limit returns `ERR_NOT_SUPPORTED_PLAN`.

## How it shows in the log

| Line | Meaning |
| --- | --- |
| `VideoToolbox is not responding; dropping frames for …` | The VT encoder did not answer in time; another input may be holding VideoToolbox |
| `stream encoder id=…: …` | Why the encoder was refused (size, frame rate, plan, encoder kind) |
| `stream destination id=…: …` | Why the destination was refused (URL, secret, encoder not running) |
| `omt_connect id=…: Connection refused` | The OMT sender is absent; retries are logged once a minute |
