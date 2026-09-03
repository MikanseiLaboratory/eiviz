---
title: Mixing Unit
description: The unit that switches Scenes on Preview and Program
---

The M/E. Same job as a vMix Mix Input, a TriCaster or ATEM M/E, or a Kairos scene.  
This is the core switching desk. Preview holds the next shot; Program holds the picture in use.

No per-session cap. Add as many as the machine will take.  
Each unit has its own resolution and output frame rate.

[Overlays](/eiviz/en/concepts/overlays/) sit on that unit’s Program. One unit’s Program can feed another unit or an Output.

The switching buses are Preview and Program.  
A Switcher UI shows Preview and Program, so it uses 2 slots of the video output destination window limit. Closing it frees those slots.

Add Input can wrap this unit’s Preview or Program as a Mix Input. That Input reads the existing FrameDelay ring (1–8 frames) and does not add a swapchain. Put it on another Mixing Unit for a nested M/E. Wiring it back onto the same unit is refused. Mix audio is a delayed copy of one selected Audio Bus, or silence if None.
