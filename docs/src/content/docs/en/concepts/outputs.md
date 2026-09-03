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

One thread is assigned per output.  
Hardware outputs such as DeckLink are still in progress.  
Send detail is in [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/).
