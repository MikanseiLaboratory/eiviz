---
title: Audio, ASIO, and related
description: Audio output and Audio Inputs
---

The internal mix is 48 kHz stereo. Hardware assignment lives under Audio Auxiliary in [Settings](/eiviz/en/introduction/settings/).

## Audio Input

Microphone and output-device loopback are regular Inputs with kind `Audio`. There is no separate Audio Input collection. Audio Inputs stay on their bus mask and do not participate in Scene Audio Follow. They are hidden from Scene layers and other video pickers.

Windows capture uses WASAPI shared mode for microphone, output-device loopback, and Application Audio. An empty device id follows the current default endpoint. Exclusive output does not fall back to shared. ASIO input shares the driver instance with output buses and is added as a stereo pair (1+2, 3+4). macOS Core Audio input is not implemented and fails explicitly.

## OMT send

OMT Program audio leaves through `AudioIngress`, not the video encode worker. A slow VMX encode must not stall the 10 ms PCM cadence.
