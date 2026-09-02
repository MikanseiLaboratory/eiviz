---
title: Vision Mixing
description: Multi M/E switching with Mixing Units
---

eiviz calls the video compose unit a **Mixing Unit**. Other switchers call the same idea Mix Effect or M/E. It maps to vMix Mix Input, Panasonic Kairos scenes, and the M/E on Viz/NewTek TriCaster and Blackmagic Design.

There is no per-session cap on Mixing Units. Add as many as the machine will take.

## Buses

Each Mixing Unit has **one Preview** bus, **one Program** bus, and **16 Multiview** buses — 18 in total.

Preview is for checking the next shot. Program is the picture that leaves the unit.

A Multiview bus assigns Inputs, Scenes, and Preview/Program into one mosaic. Up to 16 Multiview buses, each with its own tile layout and assignments.

## Input/Output re-route

Preview and Program can take a Scene or an Input. An Output’s source is Input, Scene, MU Preview, MU Program, or Multiview. Signal flow is [Inputs](/eiviz/en/concepts/inputs/).
