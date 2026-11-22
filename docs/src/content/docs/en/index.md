---
title: eiviz
description: A cross-platform vision mixer with unlimited M/E. Official documentation.
---

**eiviz** (ˈeɪvɪz) is a software vision mixer maintained by [Mikansei Laboratory](https://mikanseilaboratory.github.io/) and Shugo Kawamura / [FlowingSPDG](https://github.com/FlowingSPDG).  
Windows and macOS are supported. Linux is experimental.

This site covers how to use eiviz, why it exists, and how it is meant to be run in the field.

## Introduction

- [About eiviz](/en/introduction/about/) — motivation and technology choices
- [Mikansei Laboratory](/en/introduction/mikansei-laboratory/) — the community behind eiviz
- [System requirements](/en/introduction/requirements/) — supported environments and hardware
- [Hardware limits](/en/introduction/hardware-limits/) — encoder concurrency, decode resources, bandwidth
- [Plans](/en/introduction/plans/) — limits for Community, Professional, and Enterprise
- [Settings](/en/introduction/settings/) — items in the Settings window
- [System architecture](/en/introduction/architecture/) — mixer and UI hosts

## Concepts

- [Inputs](/en/concepts/inputs/) — video sources, and how Input, Scene, Mixing Unit, and Output connect
- [Scenes](/en/concepts/scenes/) — a composite stacked from Inputs
- [Mixing Unit](/en/concepts/mixing-unit/) — the unit that switches Preview and Program
- [MU Bus](/en/concepts/audio-auxs/) — one audio bus per Mixing Unit, plus Headphone
- [Outputs](/en/concepts/outputs/) — sending the chosen source
- [Multiviews](/en/concepts/multiviews/) — a monitor mosaic
- [Overlays](/en/concepts/overlays/) — DSK on a Mixing Unit’s Program

## Features

- [UVC Capture](/en/features/inputs/uvc/) — video from UVC devices
- [NDI / OMT Capture](/en/features/inputs/ndi-omt/) — video from NDI and OMT
- [Media](/en/features/inputs/media/) — video files and stills
- [Colour](/en/features/inputs/colour/) — colour inputs
- [Compositing](/en/features/compositing/) — Scene compositing and layers
- [NDI / OMT](/en/features/outputs/ndi-omt/) — output to NDI and OMT
- [Decklink](/en/features/outputs/decklink/) — output to DeckLink
- [Audio, ASIO, and related](/en/features/outputs/audio/) — audio output and ASIO
- [Vision Mixing](/en/features/vision-mixing/) — multi M/E switching with Mixing Units
- [Remote connection](/en/features/remote/) — operate another eiviz from the GUI
- [Headless](/en/features/headless/) — run the mixer without a GUI, then operate it from eivizctl or Remote

## Developers

- [Compatibility APIs (vMix HTTP & TCP / OBS WebSocket)](/en/developers/compatibility/) — vMix HTTP/TCP and OBS WebSocket compatible APIs
- [Native API](/en/developers/api/) — eiviz-native APIs
- [Function Reference](/en/developers/function-reference/) — eiviz function reference
