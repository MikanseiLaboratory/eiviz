---
title: NDI / OMT
description: Sending Program over NDI and OMT
---

The exit that puts a chosen source on the network. How to add a row and pick a source is [Outputs](/eiviz/en/concepts/outputs/) and [Settings](/eiviz/en/introduction/settings/) → Outputs.

## Transport and encode

Transport is OMT or NDI.

OMT can choose an encode path. GPU encode keeps the frame on the GPU and converts it to VMX. CPU encode packs UYVY, reads back, and compresses to VMX on the send thread. PCM is posted right after video on that thread and is not held for the next picture.  
NDI is always a CPU path. UYVY is handed to the SDK without waiting for encode to finish on the send thread. The sender is clocked from video; PCM goes out on a side thread.

Each output has its own send thread. One output’s encode wait does not stall another output’s accept or audio.

## Audio

The row’s audio is Master or None. Default is Master.  
An output whose source is a Multiview is silent. Mosaic encode and PCM do not share a path.

Device mix is [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/). Network PCM is taken from that internal mix.
