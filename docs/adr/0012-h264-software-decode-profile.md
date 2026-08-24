# ADR-0012 Explicit H.264 software decode profile

- Status: Accepted for implementation; HIL/legal certification pending
- Date: 2026-08-24

## Decision

The first file-video profile is progressive, unencrypted MP4 with one `avc1`
Constrained Baseline H.264 8-bit 4:2:0 video track. Audio is explicitly
unsupported until the AAC profile is licensed and implemented.

Demux uses `mp4io 0.1.2`. Decode uses a separately installed Cisco OpenH264
2.6.0 binary through `openh264-sys2 0.9.8` dynamic loading. The application
does not bundle the binary, does not compile OpenH264 source, and does not
switch to another decoder when it is missing.

AVCC samples are converted by checked in-tree code with bounded allocation.
SPS/PPS are injected whenever the decoder is created or reset.

## Rejected for this profile

- `mp4box 0.13`: MSRV 1.88, above the pinned 1.85.
- high-level `openh264`: current permitted transitive versions exceed MSRV.
- compiling bundled OpenH264 source: Cisco patent coverage applies to its
  distributed binary, not arbitrary source builds.
- FFmpeg/GStreamer fallback: different license/runtime profile and would
  violate explicit backend selection.

## Limits

Main/High profile, `avc3`, encrypted/fMP4 inputs, reverse playback, AAC,
missing color metadata, and compressed samples above the configured limit are
hard errors. They are not silently accepted or decoded by another backend.
