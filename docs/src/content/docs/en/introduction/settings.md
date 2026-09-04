---
title: Settings
description: Session-backed Settings dialog, item by item
---

Open it from Settings in the main window.

## Display

Adjusts what you see in the GUI.

<img src="/eiviz/images/en/introduction/settings/setting_ui.jpg" alt="Screenshot of the Settings window" style="max-width: 100%; height: auto;" />

### Colours

Preview, Program, and Inactive colours paint button and scene-tile chrome. Defaults are green, red, and grey.

### Master frame rate

The frame rate shared by the session. Default is NTSC 59.94p; 50p, 30p, 24p, and 60p are also listed.  
The mixer takes this value at create time, so save the session and reopen it, or restart the app, after a change.  
Each Mixing Unit can override output frame rate from its own dialog.

### Default Mixing Unit size

The default resolution of Mixing Units you add later. Default is 1920×1080. Existing units stay put. This control is on the Windows Display tab.

### Frame buffer

How many finished frames to keep in memory. Default is 3.  
It buffers so a late compose does not stop the output; audio is delayed by the same count.  
If processing cannot keep up within that count, frames are skipped until the clock catches up. The pipeline is in [System architecture](/eiviz/en/introduction/architecture/).

### Internal colour format

How pixels sit in RAM and on the upload path. The default, UYVY 4:2:2, is half the size of BGRA and converts to RGB only for frames that are actually composed. BGRA 8-bit 4:4:4 is for when you want to skip that convert. Changing it rebuilds file ingest pumps. Windows Display tab only.

## Performance

Settings that affect compose performance.

<img src="/eiviz/images/en/introduction/settings/performance.jpg" alt="Screenshot of the Performance settings" style="max-width: 100%; height: auto;" />

## Graphics adapter

Shows the graphics adapter in use.

### Resizable BAR / Unified Memory

On Windows you get Resizable BAR status, BAR window / VRAM, and whether GPU upload heaps exist.  
Use ReBAR optimization writes frames straight into GPU VRAM on upload. Integrated GPUs, and adapters that do not expose upload heaps, cannot turn it on. Turn it off if you see flicker or tearing. Why ReBAR is assumed is in [About eiviz](/eiviz/en/introduction/about/).

GPU upload heaps are not the same as Resizable BAR. Even on a ReBAR-capable machine — for example Windows 11 before 24H2 — GPU upload heaps may be unavailable, so this optimization cannot be used.

On Apple Silicon the same idea is Unified Memory optimization. Live inputs land in `MTLStorageModeShared` textures and are sampled there. Off falls back to the ordinary Metal upload path.

### Upload NDI on the ingest thread

On by default. On uploads each received frame to the GPU from a dedicated CPU thread. Off uses the shared render thread.

## Outputs

Where pictures leave the mixer.

<img src="/eiviz/images/en/introduction/settings/outputs.jpg" alt="Screenshot of the Outputs settings" style="max-width: 100%; height: auto;" />

Each row is a name, a transport, an On/Off switch, a video source, and audio.  
Transport is OMT or NDI. Hardware outputs such as DeckLink are still in progress.  
Source can be Input, Scene, MU PRV, MU PGM, or Multiview.  
Audio can be Master, Headphone, any Audio Aux, or None (no audio).  
When Multiview is selected as the video source, audio cannot be sent.

OMT can choose an encode path. GPU encode keeps the frame on the GPU and converts it to the VMX codec for send. If CPU encode is selected, the frame is read back as UYVY, then converted to the VMX codec and sent on a dedicated CPU send thread.  
NDI is always CPU encode.

One thread is assigned per output. Detail is in [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/).

Windows applies when you press OK. macOS can also Apply per row.

## Multiview

<img src="/eiviz/images/en/introduction/settings/multiview.jpg" alt="Screenshot of the Multiview settings" style="max-width: 100%; height: auto;" />

Adds and controls monitor mosaics. Detail is in [Multiviews](/eiviz/en/concepts/multiviews/).

### Default Mixing Unit for new Multiviews

Which Mixing Unit’s Preview/Program a newly created Multiview watches.

### Project default preview refresh interval

The project-default frame skip. Lower it on a weaker PC so monitoring does not hurt performance.  
Every frame through every 8 frames. Default is every 3, about 20 fps at 59.94.

## Audio Auxiliary

<img src="/eiviz/images/en/introduction/settings/audio-aux.jpg" alt="Screenshot of the Audio AUX settings" style="max-width: 100%; height: auto;" />

The internal mix is 48 kHz stereo. You can add up to eight Audio AUX buses, A–H.  
Detail is in [Audio Auxs](/eiviz/en/concepts/audio-auxs/) and [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/).

Enabled keeps the bus mixing internally with no output device.

Headphone copies Master makes the Headphone bus a duplicate of Master. Leave it off if you want a cue mix.

## Web API

vMix-compatible HTTP listen settings. These are stored in the session file.

- Enabled: start the HTTP server when the mixer starts. Default on
- Port: default 8088
- Username / password: BasicAuth if either is set. Both empty means no auth

If the port is already in use, eiviz starts with the HTTP server off, treats the session as HTTP-off, and shows a warning.

Endpoints and Functions are in [Compatibility APIs](/eiviz/en/developers/compatibility/) and the [Function Reference](/eiviz/en/developers/function-reference/).

## Advanced

### Video output destination window limit

Used for real-time Preview, Program, and Multiview.  
A Switcher UI shows Preview and Program, so it uses 2 slots. The main Preview/Program, each open Multiview, Scene Editor, and the Overlay window count the same way. Scene tiles and input-preview thumbs do not count.

Auto starts at 6. You can raise it in Settings, but it may become unstable.  
At the limit, a new window will not open. Close one to free a slot. The host path is in [System architecture](/eiviz/en/introduction/architecture/).

## Preferences

<img src="/eiviz/images/en/introduction/settings/preferences.jpg" alt="Screenshot of the Preferences window" style="max-width: 100%; height: auto;" />

Global eiviz settings.

### Language

English / 日本語.

### Theme

Dark, Light, or Follow OS.

### Help

Opens the official docs for the language in use.

- English: https://mikanseilaboratory.github.io/eiviz/en/
- 日本語: https://mikanseilaboratory.github.io/eiviz/ja/
