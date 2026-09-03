---
title: Outputs
description: Sending the chosen source
---

The exit that sends a chosen picture. Mixing Unit Program is the internal program bus; an Output is how that (or another source) leaves over the network.  
Same job as ATEM output routing or vMix NDI/SRT.

## Add and source

Add one under [Settings](/eiviz/en/introduction/settings/) → Outputs.  
Transport is OMT or NDI. Source can be Input, Scene, MU Preview, MU Program, or Multiview.  
Audio is Master or None. Default is Master. An output whose source is a Multiview is silent.  
A new session defaults to Mixing Unit Program.

Each output has its own send thread. One output’s encode wait does not stall another output’s accept or audio.  
Hardware outputs such as DeckLink are still in progress.  
Send detail is in [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/).
