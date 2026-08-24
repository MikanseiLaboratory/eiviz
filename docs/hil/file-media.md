# H.264/AAC file-media HIL

Default CI uses deterministic MP4 demux/timeline fixtures and never downloads
codec binaries. Real decode is opt-in and requires all three explicit paths:

```bash
EIVIZ_FILE_HIL_OPENH264=/absolute/path/to/cisco/libopenh264-2.6.0.so \
EIVIZ_FILE_HIL_FDK_AAC=/absolute/path/to/reviewed/libfdk-aac.so \
EIVIZ_FILE_HIL_MP4=/absolute/path/to/representative-avc-aac.mp4 \
  cargo test -p eiviz-io-file decodes_real_h264_aac_mp4_with_explicit_binaries -- --ignored
```

The OpenH264 binary must pass the `openh264-sys2` Cisco 2.6.0 SHA-256 allow-list
and runtime version checks. The FDK binary is not supplied or approved by this
repository: record its exact hash, provenance, upstream license, and the
deployment's patent/legal review before running or distributing it.

Use an unencrypted regular MP4 containing one `avc1` Constrained Baseline track
and one mono/stereo AAC-LC `mp4a` track. Include clips with and without an
audio edit-list priming offset, 44.1 and 48 kHz source rates, seeks, and loop
boundaries. For mismatched rates, select the project ASRC policy explicitly;
`ExactRate` must fail admission. Verify decoded content externally and measure
A/V skew across start, seek, and at least 100 loop boundaries.

Passing this smoke test proves only that the supplied binaries decode the
supplied clip. It is not codec conformance, patent clearance, cross-platform
coverage, or long-duration sync certification.
