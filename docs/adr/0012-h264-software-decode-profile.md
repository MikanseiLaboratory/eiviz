# ADR-0012 Explicit H.264 software decode profile

- Status: Implemented; HIL/legal certification pending
- Date: 2026-08-24

## Decision

The first file-video profile is progressive, unencrypted MP4 with one `avc1`
Constrained Baseline H.264 8-bit 4:2:0 video track. Audio is explicitly
unsupported until the AAC profile is licensed and implemented.

Demux uses `shiguredo_mp4 2026.4.0`. Decode uses a separately installed Cisco OpenH264
2.6.0 binary through `openh264-sys2 0.9.8` dynamic loading. The application
does not bundle the binary, does not compile OpenH264 source, and does not
switch to another decoder when it is missing.

AVCC samples are converted by checked in-tree code with bounded allocation.
SPS/PPS are injected whenever the decoder is created or reset.
`VideoFileSource` decodes from the preceding MP4 sync sample after a seek or
loop boundary and converts OpenH264 I420 output to limited-range BT.709 RGBA
with `yuv 0.8.17`. `InputSource::Video.playback` is authoritative for
play/pause, forward seek, loop, in/out points, and positive playback speed.
The OpenH264 path is explicit runtime configuration and is never persisted.

Construction fails if the path is absent, the binary hash is not in
`openh264-sys2`'s Cisco 2.6.0 allow-list, the reported version is not 2.6.0,
or required decoder symbols are absent. No source build or decoder fallback is
enabled.

## Rejected for this profile

- `mp4io`: replaced by the user-selected Shiguredo MP4 implementation.
- high-level `openh264`: current permitted transitive versions exceed MSRV.
- compiling bundled OpenH264 source: Cisco patent coverage applies to its
  distributed binary, not arbitrary source builds.
- FFmpeg/GStreamer fallback: different license/runtime profile and would
  violate explicit backend selection.

## Limits

Main/High profile, `avc3`, encrypted/fMP4 inputs, reverse playback, AAC, and
compressed samples above the configured limit are hard errors. The current
vertical slice applies the product's fixed limited-range BT.709 profile; MP4
color metadata validation remains a certification blocker. They are not
silently accepted or decoded by another backend.

No Cisco binary or representative conformance clip is stored in this
repository, so unit tests cover missing-binary failure and playback cursor
behavior only. Hardware/integration decode evidence is not claimed.
