---
title: Vision Mixing
description: Multi M/E switching with Mixing Units
---

eiviz calls the video compose unit a **Mixing Unit**. Other switchers call the same idea Mix Effect or M/E.  
It maps to vMix Mix Input, Panasonic Kairos scenes, and the M/E on Viz/NewTek TriCaster and Blackmagic Design.

There is no per-session cap on Mixing Units. Add as many as the machine will take.  
Each unit has its own resolution and output frame rate. The session clock is Master Frame Rate in [Settings](/eiviz/en/introduction/settings/).

## Buses

Each Mixing Unit has two switching buses: **Preview** and **Program**.

Preview is for checking the next shot. Program is the picture that leaves the unit.  
The switcher UI assigns a [Scene](/eiviz/en/concepts/scenes/) to Preview. CUT, AUTO, or the T-bar swaps Preview onto Program.

### Overlay

[Overlays](/eiviz/en/concepts/overlays/) are up to eight per Mixing Unit. The source is a Scene or an Input.  
They sit on Program after CUT or the T-bar has mixed. They do not sit on Preview.

### Multiview

Monitor mosaics are not Mixing Unit buses. They are session-level [Multiviews](/eiviz/en/concepts/multiviews/).  
There is no cap on layouts in the session. Open monitor windows count toward the video output destination window limit in [Settings](/eiviz/en/introduction/settings/). A tile can be an Input, a Scene, MU Preview, or MU Program.

## Transition

A Transition runs when the picture changes.  
Several kinds are available; compose uses WGSL.  
Pick the animation and frame interpolation, then run it to change Program.

## Outputs

An Output’s source can be an Input, a Scene, Mixing Unit Preview or Program, or a Multiview. The default is Mixing Unit Program. When Multiview is selected as the video source, audio cannot be sent. Signal flow is in [Inputs](/eiviz/en/concepts/inputs/) and [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/).
