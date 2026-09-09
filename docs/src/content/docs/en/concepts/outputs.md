---
title: Outputs
description: Sending the chosen source
---

The exit that sends a chosen picture. Mixing Unit Program is the internal program bus; an Output is how that (or another source) leaves over the network.  
Same job as ATEM output routing or vMix NDI/SRT.

## Add and source

Add one under [Settings](/eiviz/en/introduction/settings/) → Outputs.  
Transport is OMT or NDI. Source can be Input, Scene, MU Preview, MU Program, or Multiview.  
Audio can be Master, Headphone, any Audio Aux, or None (no audio).  
When Multiview is selected as the video source, audio cannot be sent.  
A new session defaults to Mixing Unit Program.

Each output has its own resolution and frame rate under Settings → Outputs. Follow session settings uses the Mixing Unit (or the session master frame rate and default size). Video is sent at that output rate, not as soon as a compose finishes.

Master compose, Mixing Unit compose, and each Output share one rational media clock from a common epoch. Deadlines and PTS are `index × interval`, not a running wall-clock sum. A late Output skips the missed slot and does not burst catch-up frames. If the Output rate is higher than the source, the latest finished frame is repeated. If it is lower, older finished frames are dropped so the newest one is sent. Audio uses the same content timeline and the same frame-buffer delay.

One thread is assigned per output. Encode and socket wait on one Output do not stall compose, the audio scheduler, or other Outputs.  
OMT and NDI encode once per configured Output; extra receivers on that Output are transport fan-out, not another encode. Ten Outputs that point at the same picture are ten independent encodes.  
Hardware outputs such as DeckLink are still in progress.  
Send detail is in [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/).

## Clock soak

Automated tests cover 23.976–120 fps slot counts, 59.94→50 / 59.94→25 drop cadence, 29.97→59.94 repeat, and a shared A/V epoch. On hardware, run at least 30 minutes with a flash+tone pattern on GPU OMT, CPU OMT, and NDI. A-V offset should stay within one audio packet or one output frame. Add a delayed or stopped Output and confirm the others keep cadence.
