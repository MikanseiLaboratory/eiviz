---
title: NDI / OMT Capture
description: Capturing video from NDI and OMT
---

An Input that receives a network NDI or OMT source. How to add one is [Inputs](/eiviz/en/concepts/inputs/) and [Settings](/eiviz/en/introduction/settings/). Remote receive is covered in [Remote](/eiviz/en/features/remote/).

## Transport and decode

NDI is received on the CPU and uploaded for compose.

OMT can choose a decode path. The default is CPU (Recommended).

CPU (Recommended) is the default OMT decoder. Use it for camera inputs and other mission-critical video. CPU decode copies frames into system memory.

GPU is an auxiliary OMT decoder. It can offload work from the CPU, but it loses more than CPU, so use it for Multiview and other auxiliary video. GPU decode keeps VMX frames on the GPU.

Saved sessions that already set GPU stay on GPU. Only newly created Inputs default to CPU.

## Quality and buffer

Frame buffer (1–8) absorbs jitter on this source. Quality is requested from the sender. Unused sources stay connected and also request Preview (1/8 decode). Bandwidth save and keeping full quality on Multiview are available in the Windows Input dialog.
