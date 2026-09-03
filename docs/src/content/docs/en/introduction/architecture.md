---
title: System architecture
description: Mixer versus host, the compose pipeline, and how GPU and audio move
---

eiviz system architecture. The shape is largely the same across platforms.

## Shape

eiviz runs as one process.  
The video and audio state machine is the mixer (Rust + wgpu). Each OS host talks to it over a C ABI. Windows is WPF; macOS is SwiftUI. There is no Linux host yet.

The host owns windows, interaction, and preview surfaces.  
Compose, audio, I/O, and session data live in the mixer. Keeping that work off the UI is how the stack stays fast and portable.

```mermaid
flowchart TB
  subgraph proc["One process"]
    host["Host UI"]
    abi["C ABI"]
    subgraph mixer["Mixer"]
      ctrl["Control and session"]
      clock["Mix clock"]
      ingest["Ingest"]
      send["Network send"]
      audio["Audio graph"]
    end
  end
  host --> abi --> ctrl
  ctrl --> clock
  ingest --> clock
  clock --> send
  clock --> audio
  clock --> host
```

## Who owns what

| | Mixer | Host |
| --- | --- | --- |
| Compose and transitions | Yes | Sends the gesture |
| GPU and audio devices | Yes | Passes settings |
| Live preview | Draws into a native surface | Supplies the surface (HWND / NSView) |
| Scene tiles and similar | Reads back from the GPU | Shows the thumbnail |

There is one mixer per process. The host never receives GPU pointers except live preview surfaces. Inputs, scenes, and Mixing Units are integer ids.

## Concurrency

The UI thread drives interaction and picture present.  
File, UVC, NDI, and OMT ingest fill buffers on other paths. The mixer takes frames on a fixed interval.

If processing falls behind, compose is skipped and audio still advances so the clock can catch up.  
Video keeps a few frames of buffer so it lines up with audio.

```mermaid
flowchart LR
  ui["UI"] --> mixer["Mixer control"]
  cap["Ingest"] --> buf["Frame buffer"]
  mixer --> clock["Mix clock"]
  buf --> clock
  clock --> pvw["Preview"]
  clock --> net["OMT / NDI"]
  clock --> spk["Audio out"]
```

## How sources are named

Generators such as solid colour and colour bars, session inputs, composited scenes, and a Mixing Unit’s Preview / Program / Multiview all live in one **source-id space**.  
That is why one Mixing Unit’s Program can feed another Mixing Unit.

```mermaid
flowchart LR
  gen["Generators"] --> id["Source id"]
  inp["Inputs"] --> id
  scene["Scenes"] --> id
  mu["Mixing Unit PVW/PGM/MV"] --> id
  id --> compose["Compose"]
  compose --> mu
```

## Textures

Inputs, scenes, a Mixing Unit’s Preview/Program, and Multiview all live in one source-id space as **GPU textures**. Compose and the GUI sample those `TextureView`s.

| Kind | What it is |
| --- | --- |
| Input | Ingest result. GPU ingest shares the handle; CPU ingest overwrites one texture |
| Scene | Composited layers |
| Multiview | The same scene object, plus labels and tallies |
| Preview | The Mixing Unit `preview` bus |
| Program | `mixed` after mix and overlays. GUI and send point here |

Program keeps a pre-mix `program` target and the on-air `mixed` target. Compose draws live scenes. Session scene textures stay allocated.

### Path to the GUI

The host supplies the window surface; the mixer draws into it. There are two paths.

```mermaid
flowchart TB
  inp["Input"]
  sc["Scene / MV"]
  prv["MU preview"]
  pgm["MU mixed"]
  delay["Frame Delay"]
  inp --> sc
  inp --> prv
  sc --> prv
  prv --> pgm
  pgm --> delay
  prv --> delay
  delay --> swap["swapchain blit"]
  pgm --> swap
  sc --> swap
  inp --> swap
  sc --> thumb["downscale blit + readback"]
  inp --> thumb
  swap --> live["Live surfaces"]
  thumb --> tiles["List thumbnails"]
```

Live Preview/Program, an open Multiview, Scene Editor, and the Overlay window blit an existing view into an HWND / NSView swapchain. Several live surfaces of the same source each blit that view.

Input lists, scene lists, and switcher source buttons read back a downscale (up to 960×540).

### GPU copies

These copies run in the frame:

- Frame Delay. Copies `mixed` and `preview` into a ring so picture lines up with audio. GUI Preview/Program sample that delayed surface
- Transition history. Copies `mixed` into `prev`
- Send. If CPU encode is selected, the frame is read back as UYVY, then converted to the VMX codec and sent on a dedicated CPU send thread. GPU OMT copies into a per-output send slot

Intermediate buffers such as sort / flow / bloom stay in VRAM. Their compute runs during those transitions.

## One frame

A frame has three lanes:

1. On-air ingest — every master frame
2. On-air compose (Preview/Program/outputs) — every master frame
3. Monitor compose (scene tiles, input preview) and ingest for those sources — present interval

On-air sources upload every master frame. Monitor and thumbnail Inputs upload on their present interval. OMT receive quality reads every attached monitor and thumb each tick.

Then the picture moves like this:

1. An ingest thread deposits the latest frame
2. Each Mixing Unit draws Preview and Program, mixes with the T-bar or AUTO, then overlays and multiview
3. Outputs that send pack UYVY or copy into a GPU slot
4. One thread is assigned per output to compress and transmit
5. Audio buses mix on the same master tick and ride those outputs. When Multiview is selected as the video source, audio cannot be sent

```mermaid
sequenceDiagram
  participant Cap as Ingest
  participant Buf as Buffer
  participant Clock as Mix clock
  participant GPU as Compose
  participant Out as Preview and send
  Cap->>Buf: Frame
  Clock->>Buf: Take
  Clock->>GPU: PVW / PGM / mix / overlay
  GPU->>Out: Texture
  Clock->>Out: Audio
```

## GPU

Compose calls the GPU through a wgpu abstraction. Windows is Direct3D 12; macOS is Metal.  
CPU pictures usually upload through system memory.

On Windows with Resizable BAR on a discrete GPU, the host reaches through wgpu to the DX12 low-level API and writes straight into VRAM.  
Apple Silicon uses unified memory for a similar path.

File and UVC decode on the GPU when they can, then convert into the compose format.  
GPU work is preferred so the CPU can spend time on NDI and other CPU-heavy paths.  
That behaviour can be changed in [Settings](/eiviz/en/introduction/settings/).

```mermaid
flowchart TB
  cpu["CPU pixels"]
  staging["Ordinary staging"]
  fast["ReBAR / Unified Memory"]
  gpu["Compose texture"]
  cpu --> staging --> gpu
  cpu --> fast --> gpu
```

## Audio

The internal mix is a 48 kHz graph. Master and Headphone are fixed; AUX buses can be added.  
Inputs have a bus mask and gain. A Mixing Unit can send Program-follow audio (Audio Follow) onto a bus. Overlays can do the same.

Detail is in [Audio Auxs](/eiviz/en/concepts/audio-auxs/).

## Outputs

| | Video | Status |
| --- | --- | --- |
| OMT | Stay on the GPU, or read back as UYVY and convert to VMX on a dedicated send thread | Shipped |
| NDI | CPU path | Shipped |
| DeckLink | — | In progress |

One thread is assigned per output.

OMT receive stays full quality on Preview/Program and drops bandwidth otherwise.  
Full quality holds a short time after leaving those buses so TAKE and the T-bar do not rebuild the receiver.

Detail is in [Settings](/eiviz/en/introduction/settings/) → Outputs and [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/).

## Hosts

Live Preview/Program, an open Multiview, Scene Editor, the Overlay window, and a switcher’s Preview/Program are drawn by the mixer into a native surface. Windows uses a child HWND; macOS uses an NSView with a Metal layer from wgpu.

Scene tiles, switcher scene thumbs, and input previews are GPU readback thumbnails. Adding scenes or Mix Inputs does not add swapchains. A Mix Input is a delayed alias of a Mixing Unit bus or a session Multiview; it reads the FrameDelay ring and uses the same thumbnail path.

Windows cannot keep many DXGI flip swapchains at once. [Settings](/eiviz/en/introduction/settings/) → Advanced, Video output destination window limit, caps how many may be open. They are used for real-time Preview, Program, and Multiview. A Switcher UI shows Preview and Program, so it uses 2 slots. You can raise the limit, but it may become unstable. Closing a window detaches its swapchain and frees a slot. Closing the main window closes the extra windows and exits the process.

Reloading a session rebuilds the main window so preview surfaces attach on first layout. HWNDs are not reused across mixer lifetimes.
