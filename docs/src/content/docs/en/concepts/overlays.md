---
title: Overlays
description: DSK on a Mixing Unit’s Program
---

DSK, downstream key, vMix Overlay.  
Definitions are shared by the session. Each [Mixing Unit](/eiviz/en/concepts/mixing-unit/) keeps its own On-Air list, with no count limit. The source is a [Scene](/eiviz/en/concepts/scenes/) or an Input.  
Transition presets are session-wide too. A duration in frames uses the frame rate of the Mixing Unit that fires the transition.  
vMix `Overlay1`–`Overlay8`, and the vMix XML overlay list, follow the first eight On-Air overlays of the current Mixing Unit. Later overlays are not on that surface.

They sit on top of Program. Overlay in the main window sets position and size; the desk toggles turn them on and off.  
The Overlay window shows Program in real time, so it uses 1 slot of the video output destination window limit. Closing it frees the slot.
Transitions are Cut or Fade.
