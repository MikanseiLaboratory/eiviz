---
title: Audio, ASIO, and related
description: Audio output and Audio Inputs
---

The internal mix is 48 kHz stereo. Hardware assignment lives under Audio Auxiliary in [Settings](/eiviz/en/introduction/settings/).

## Audio Input

Microphone and output-device loopback are regular Inputs with kind `Audio`. There is no separate Audio Input collection. Audio Inputs stay on their bus mask and do not participate in Scene Audio Follow. They are hidden from Scene layers and other video pickers.

Windows microphone, output-device loopback, and WASAPI playback use cpal's WASAPI host in shared mode. Application Audio uses rsac process loopback (f32 / 48 kHz / stereo, no PCM autoconvert). An empty device id follows the current default endpoint. Output is WASAPI shared or ASIO. Application Audio lists only processes that currently have a visible window. Process capture does not fall back to endpoint loopback. ASIO input shares the driver instance with output buses. L and R each pick any input channel. macOS Core Audio playback and microphone / loopback capture use cpal. Core Audio process loopback is not implemented and fails explicitly.

## OMT send

OMT Program audio leaves through `AudioIngress`, not the video encode worker. A slow VMX encode must not stall the 10 ms PCM cadence.
