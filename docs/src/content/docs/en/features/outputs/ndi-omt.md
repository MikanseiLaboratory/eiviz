---
title: NDI / OMT
description: Sending Program over NDI and OMT
---

The exit that puts a chosen source on the network. How to add a row and pick a source is [Outputs](/eiviz/en/concepts/outputs/) and [Settings](/eiviz/en/introduction/settings/) → Outputs.

## Transport and encode

Transport is OMT or NDI.

OMT can choose an encode path. GPU encode keeps the frame on the GPU and converts it to VMX. If CPU encode is selected, the frame is read back as UYVY, then converted to the VMX codec and sent on a dedicated CPU send thread.  
NDI is always a CPU path.

One thread is assigned per output.

Each OMT output can skip VMX encode when no receiver is subscribed. The default is on. Turn off “Skip encode when there are no OMT receivers” in Settings → Outputs to keep encoding. NDI ignores this option.

## Audio

Audio can be Master, Headphone, any Audio Aux, or None (no audio).  
When Multiview is selected as the video source, audio cannot be sent.

Device mix is [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/). Network PCM is taken from that internal mix.
