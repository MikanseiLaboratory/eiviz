---
title: NDI / OMT
description: Sending Program over NDI and OMT
---

The exit that puts a chosen source on the network. How to add a row and pick a source is [Outputs](/eiviz/en/concepts/outputs/) and [Settings](/eiviz/en/introduction/settings/) → Outputs.

## Transport and encode

Transport is OMT or NDI.

OMT can choose an encode path. GPU encode keeps the frame on the GPU and converts it to VMX. If CPU encode is selected, the frame is read back as UYVY, then converted to the VMX codec and sent on a dedicated CPU send thread.  
NDI is always a CPU path.

One thread is assigned per output. Video frames are sent at that output’s frame rate, not as soon as compose finishes. Resolution and frame rate are per output in Settings → Outputs. eiviz paces NDI submits on the shared media clock. The NDI SDK still requires one clock; audio clocking blocked send_audio, so video stays the required SDK clock. OMT encodes a VMX bitstream once per Output and fans that bitstream out to receivers. The pinned `openmediatransport-rs` revision already shares one encoded `Arc` and writes peers from a bounded background queue (depth 4, drop when full, 40 ms video write timeout). Ten receivers do not mean ten encodes.

Each OMT output can skip VMX encode when no receiver is subscribed. The default is on. Turn off “Skip encode when there are no OMT receivers” in Settings → Outputs to keep encoding. NDI ignores this option.

## Audio

Audio can be Master, Headphone, any Audio Aux, or None (no audio).  
When Multiview is selected as the video source, audio cannot be sent.

Device mix is [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/). Network PCM is taken from that internal mix.
