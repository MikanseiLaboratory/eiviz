# ADR-0012 Explicit H.264 software decode profile

- Status: Implemented; HIL/legal certification pending
- Date: 2026-08-24

## Decision

The first file-media profile is progressive, unencrypted MP4 with one `avc1`
Constrained Baseline H.264 8-bit 4:2:0 video track and zero or one `mp4a`
AAC-LC mono/stereo track.

Demux uses `shiguredo_mp4 2026.4.0`. Decode uses a separately installed Cisco OpenH264
2.6.0 binary through `openh264-sys2 0.9.8` dynamic loading. The application
does not bundle the binary, does not compile OpenH264 source, and does not
switch to another decoder when it is missing.

AVCC samples are converted by checked in-tree code with bounded allocation.
SPS/PPS are injected whenever the decoder is created or reset.
`FileMediaSource` decodes video from the preceding MP4 sync sample after a seek
or loop boundary and converts OpenH264 I420 output to limited-range BT.709 RGBA
with `yuv 0.8.17`. It parses `mp4a`/`esds` AudioSpecificConfig through
Shiguredo's typed boxes and accepts AAC-LC object type 2, 1024-sample frames,
and channel configurations 1 or 2. Exposed single-media edit lists map encoder
priming and optional leading empty edits onto a shared presentation timeline.
Both dynamic decoders reset together at seek/loop boundaries.
`InputSource::Video.playback` is authoritative for
play/pause, forward seek, loop, in/out points, and positive playback speed.
The A/V profile currently requires speed 1.0 because ASRC is sample-rate
conversion, not time stretching. Codec paths are explicit runtime
configuration and are never persisted.

Construction fails if the path is absent, the binary hash is not in
`openh264-sys2`'s Cisco 2.6.0 allow-list, the reported version is not 2.6.0,
or required decoder symbols are absent. No source build or decoder fallback is
enabled.

When an AAC track is present, construction additionally requires an explicit
operator-selected FDK AAC shared library. It loads only the raw decoder ABI;
there is no discovery, bundled binary, alternate decoder, audio-track drop, or
PCM substitution. Audio remains at its MP4 rate. Project `ExactRate` rejects a
mismatch, while a persisted `Asrc` policy sends it through Runtime's common
stateful converter.

## Rejected for this profile

- `mp4io`: replaced by the user-selected Shiguredo MP4 implementation.
- high-level `openh264`: current permitted transitive versions exceed MSRV.
- compiling bundled OpenH264 source: Cisco patent coverage applies to its
  distributed binary, not arbitrary source builds.
- FFmpeg/GStreamer fallback: different license/runtime profile and would
  violate explicit backend selection.

## Limits

Main/High profile, `avc3`, encrypted/fMP4 inputs, reverse/time-stretched A/V
playback, HE-AAC, more than stereo, complex edit lists, and compressed samples
above configured limits are hard errors. The decoder API requires the explicitly
selected `Bt709Sdr` Project profile and exposes the conversion as
`decode_bt709_limited`; other Project color profiles are rejected. Demux
validates Shiguredo's retained `colr`/`nclx` sample-entry box or H.264 SPS VUI
colour description before constructing the decoder. At least one source must
explicitly signal primaries/transfer/matrix `1/1/1` and limited range. Missing,
malformed, full-range, or mismatched metadata is a hard error; BT.709 is never
assumed. Inputs are not silently accepted or decoded by another backend.

No Cisco/FDK binary or representative conformance clip is stored in this
repository. Default tests cover deterministic `mp4a`/`esds` demux, edit-list
timeline mapping, synchronized cursor generations, AudioSpecificConfig and
MP4/H.264 color-signaling validation, and missing-binary failures. Ignored opt-in video and A/V tests
accept explicit binary and MP4 paths. On
2026-08-24 that test decoded an ffmpeg-generated 16x16 Constrained Baseline MP4
with Cisco's Linux x64 2.6.0 `.8.so` whose SHA-256 matched the
`openh264-sys2` allow-list. This is a local smoke test, not representative
conformance, cross-platform evidence, or certification HIL.
