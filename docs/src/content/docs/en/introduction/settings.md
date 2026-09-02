---
title: Settings
description: Session-backed Settings dialog, item by item
---

Open it from Settings in the main window.

## Display

Adjusts what you see in the GUI.

<img src="/eiviz/introduction/settings/setting_ui.jpg" alt="Screenshot of the Settings window" style="max-width: 100%; height: auto;" />

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

<img src="/eiviz/introduction/settings/performance.jpg" alt="Screenshot of the Performance settings" style="max-width: 100%; height: auto;" />

## Graphics adapter

Shows the graphics adapter in use.

### Resizable BAR / Unified Memory

On Windows you get Resizable BAR status, BAR window / VRAM, and whether GPU upload heaps exist.  
Use ReBAR optimization writes frames straight into GPU VRAM on upload. Integrated GPUs, and adapters that do not expose upload heaps, cannot turn it on. Turn it off if you see flicker or a device reset. Why ReBAR is assumed is in [About eiviz](/eiviz/en/introduction/about/).

On Apple Silicon the same idea is Unified Memory optimization. Live inputs land in `MTLStorageModeShared` textures and are sampled there. Off falls back to the ordinary Metal upload path.

### Upload NDI on the ingest thread

On by default. On writes each NDI frame to the GPU so compose and send stay faster. Off uploads CPU frames on the render thread.

## Outputs

Where pictures leave the mixer.

<img src="/eiviz/introduction/settings/outputs.jpg" alt="Screenshot of the Outputs settings" style="max-width: 100%; height: auto;" />

Each row is a name, a transport, an On/Off switch, and a video source.  
Transport is OMT or NDI. Hardware outputs such as DeckLink are still in progress.  
Source can be Input, Scene, MU PRV, MU PGM, or Multiview.

OMT can choose an encode path. GPU encode keeps the frame on the GPU and converts it to the VMX codec for send. CPU encode converts to UYVY and reads back to the CPU.  
NDI is always CPU encode.

Windows applies when you press OK. macOS can also Apply per row.

## Multiview

<img src="/eiviz/introduction/settings/multiview.jpg" alt="Screenshot of the Multiview settings" style="max-width: 100%; height: auto;" />

Adds and controls monitor mosaics. Detail is in [Multiviews](/eiviz/en/concepts/multiviews/).

### Default Mixing Unit for new Multiviews

Which Mixing Unit’s Preview/Program a newly created Multiview watches.

### Project default preview refresh interval

The project-default frame skip. Lower it on a weaker PC so monitoring does not hurt performance.  
Every frame through every 8 frames. Default is every 3, about 20 fps at 59.94.

## Audio Auxiliary

<img src="/eiviz/introduction/settings/audio-aux.jpg" alt="Screenshot of the Audio AUX settings" style="max-width: 100%; height: auto;" />

The internal mix is 48 kHz stereo. You can add up to eight Audio AUX buses, A–H.  
Detail is in [Audio Auxs](/eiviz/en/concepts/audio-auxs/) and [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/).

Enabled keeps the bus mixing internally with no output device.

Headphone copies Master makes the Headphone bus a duplicate of Master. Leave it off if you want a cue mix.

## Preferences

<img src="/eiviz/introduction/settings/preferences.jpg" alt="Screenshot of the Preferences window" style="max-width: 100%; height: auto;" />

Global eiviz settings.

### Language

English / 日本語.

### Theme

Dark, Light, or Follow OS.
