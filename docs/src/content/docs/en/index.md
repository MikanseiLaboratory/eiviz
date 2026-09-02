---
title: eiviz
description: Official eiviz documentation
---

**eiviz** (ˈeɪvɪz) is a software vision mixer maintained by [Mikansei Laboratory](https://mikanseilaboratory.github.io/) and Shugo Kawamura / [FlowingSPDG](https://github.com/FlowingSPDG).  
Windows and macOS are supported. Linux is experimental.

This site covers how to use eiviz, why it exists, and how it is meant to be run in the field.

## Introduction

- [About eiviz](/eiviz/en/introduction/about/) — motivation and technology choices
- [Mikansei Laboratory](/eiviz/en/introduction/mikansei-laboratory/) — the community behind eiviz
- [System requirements](/eiviz/en/introduction/requirements/) — supported environments and hardware
- [Settings](/eiviz/en/introduction/settings/) — items in the Settings window
- [System architecture](/eiviz/en/introduction/architecture/) — mixer and UI hosts

## Concepts

- [Inputs](/eiviz/en/concepts/inputs/) — video sources, and how Input, Scene, Mixing Unit, and Output connect
- [Scenes](/eiviz/en/concepts/scenes/) — a composite stacked from Inputs
- [Mixing Unit](/eiviz/en/concepts/mixing-unit/) — the unit that switches Preview and Program
- [Audio Auxs](/eiviz/en/concepts/audio-auxs/) — Master, Headphone, and AUX buses
- [Outputs](/eiviz/en/concepts/outputs/) — sending the chosen source
- [Multiviews](/eiviz/en/concepts/multiviews/) — a monitor mosaic
- [Overlays](/eiviz/en/concepts/overlays/) — DSK on a Mixing Unit’s Program

## Features

- [UVC Capture](/eiviz/en/features/inputs/uvc/) — video from UVC devices
- [NDI / OMT Capture](/eiviz/en/features/inputs/ndi-omt/) — video from NDI and OMT
- [Media](/eiviz/en/features/inputs/media/) — video files and stills
- [Colour](/eiviz/en/features/inputs/colour/) — colour inputs
- [Compositing](/eiviz/en/features/compositing/) — Scene compositing and layers
- [NDI / OMT](/eiviz/en/features/outputs/ndi-omt/) — output to NDI and OMT
- [Decklink](/eiviz/en/features/outputs/decklink/) — output to DeckLink
- [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/) — audio output and ASIO
- [Vision Mixing](/eiviz/en/features/vision-mixing/) — multi M/E switching with Mixing Units

## Developers

- [Compatibility APIs (vMix HTTP & TCP / OBS WebSocket)](/eiviz/en/developers/compatibility/) — vMix HTTP/TCP and OBS WebSocket compatible APIs
- [Native API](/eiviz/en/developers/api/) — eiviz-native APIs
- [Function Reference](/eiviz/en/developers/function-reference/) — eiviz function reference
