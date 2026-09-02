---
title: Settings
description: Session-backed Settings dialog, item by item
---

Open it from Settings in the main chrome. Values live in the session JSON, so the same file opens on Windows and macOS. Language and window theme are the neighbouring Preferences dialog.

The left list is the category: Display, Performance, Outputs, Multiview, Audio Auxiliary. OK pushes into the mixer. Cancel discards. On Windows, Default restores display numbers, bus colours, ReBAR, and NDI upload. It does not touch the output or bus lists.

## Display

Preview, Program, and Inactive colours paint button and scene-tile chrome. Defaults are green, red, and dark grey.

Master Frame Rate clocks the mix. Default is NTSC 59.94p; 50p, 30p, 24p, and 60p are also listed. The mixer takes this value at create time, so save the session and reopen it, or restart the app, after a change. Each Mixing Unit can override output frame rate from its own dialog.

Default Mixing Unit Size is the resolution of units you add later. Default is 1920×1080. Existing units stay put. This control is on the Windows Display tab.

Frame buffer is how many finished frames to keep (1–8, default 3). It covers a compose spike so the output does not hitch; audio is delayed by the same count. If the buffer runs dry, compose is skipped until the clock catches up. The pipeline is in [System architecture](/introduction/architecture/).

Internal color format is how pixels sit in RAM and on the upload path. UYVY 4:2:2 is the default: half the size of BGRA, converted to RGB only for frames that are actually composed. BGRA 8-bit 4:4:4 skips that convert. Changing it rebuilds file ingest pumps. Windows Display tab only.

## Performance

The current graphics adapter name is shown. Nothing here if the mixer is down.

On Windows you also get Resizable BAR status, BAR window / VRAM, and whether GPU upload heaps exist. Use ReBAR optimization writes into a D3D12 GPU upload heap (VRAM) instead of wgpu’s system-memory staging. Integrated GPUs, and adapters that do not expose upload heaps, cannot turn it on. Turn it off if you see flicker or a device reset. Why ReBAR is assumed is in [About eiviz](/introduction/about/).

On Apple Silicon the same checkbox is Unified Memory optimization. Live CPU inputs land in `MTLStorageModeShared` textures and are sampled there. Off falls back to the ordinary Metal upload path.

Upload NDI on the ingest thread is on both OSes and on by default. Each frame hits the GPU before the mixer samples it, so the mix clock is not stuck in memcpy. Off restores CPU frames uploaded on the render thread.

## Outputs

OMT and NDI® leave from the mixer. Transport detail is [NDI / OMT](/features/outputs/ndi-omt/).

Each row is a name, a transport, Enabled, and a source. Transport is OMT or NDI. Windows also lists DeckLink; it fails immediately in this build because the SDK is not linked. Source is Input, Scene, MU PRV, MU PGM, or Multiview.

OMT has an encode path. GPU encode keeps the frame on the GPU. CPU encode packs UYVY and reads back. NDI is always CPU encode. A new session’s `eiviz-pgm` is OMT, Mixing Unit Program, GPU encode.

Windows applies on OK. macOS can also Apply per row.

## Multiview

Defines mosaics and opens their windows. The concept is [Multiviews](/concepts/multiviews/). `+` adds one; Open, Layout, and Delete sit under the list. Fullscreen is F11.

Default Mixing Unit for new Multiview windows is which unit’s Preview/Program a new mosaic watches.

Project default preview refresh interval is the skip used when a window is set to Follow project. Every frame through every 8 frames; default is every 3, about 20 fps at 59.94. Program, Preview, and network outputs stay on the master frame rate. Only the monitor picture is thinned.

## Audio Auxiliary

The internal mix is 48 kHz stereo. Master and Headphone cannot be removed. You can add up to eight AUX buses, named A–H. What a bus is: [Audio Auxs](/concepts/audio-auxs/). Devices: [Audio, ASIO, and related](/features/outputs/audio/).

Enabled keeps the bus in the mix with no hardware device. To a device, Windows uses WASAPI or ASIO; macOS uses Core Audio. Map Left/Right onto device channel numbers. WASAPI can be Exclusive.

Headphone copies Master makes the Headphone bus a duplicate of Master. Leave it off if you want a cue mix.

macOS rows have gain and Mute. The Windows dialog does not.

## Preferences

A different dialog. Language (English / 日本語) and theme (Dark, Light, Follow OS). Stored in `prefs.json`, not in the session file.
