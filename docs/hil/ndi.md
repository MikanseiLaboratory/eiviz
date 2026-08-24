# NDI receive/send HIL

Status: **not yet executed**. This host has no NDI 6 SDK/runtime installation
and no second NDI endpoint. Unit tests and successful default-feature builds
do not establish interoperability.

## Implementation and prerequisites

- `grafton-ndi` 1.0.0, Apache-2.0
- NDI 6 SDK headers at build time (`NDI_SDK_DIR` for a nonstandard location)
- Matching NDI runtime discoverable by the platform dynamic loader
- Desktop built with the explicit `ndi` feature
- No OMT, simulator, generated slate, or other adapter fallback

The NDI SDK/runtime is separately licensed. Before distribution, review the
agreement in the installed SDK and current guidance at
https://docs.ndi.video/all/developing-with-ndi/sdk/licensing. The product must
link to https://ndi.video/ near NDI controls. If permitted runtime files are
bundled, keep them application-local and include
`Processing.NDI.Lib.Licenses.txt`.

## Required equipment

1. Two separate machines on the same LAN with synchronized clocks.
2. Current NDI reference sender/receiver or NDI Tools on the other machine.
3. NDI 6 SDK/runtime installed on the eiviz build and test host.
4. Packet loss, jitter, bandwidth limiting, and NIC disconnect injection.

## Acceptance scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| NDI-HIL-01 | Discovery | Reference source name appears and the selected source alone is connected |
| NDI-HIL-02 | 1080p59.94 RGBA/RGBX receive | Reference color bars, alpha policy, dimensions, cadence, and 100 ns timestamp conversion match |
| NDI-HIL-03 | 48 kHz planar audio receive | Channel order, level, sample index, and A/V relationship match |
| NDI-HIL-04 | TAKE/audio follow | NDI video and routed audio switch at the same logical boundary |
| NDI-HIL-05 | Program send | Reference receiver gets 1080p59.94 RGBA video and 48 kHz audio with monotonic timecode |
| NDI-HIL-06 | Queue pressure | Slow output degrades only that Output; Program and other Outputs continue |
| NDI-HIL-07 | Sender restart/NIC loss | Explicit degraded state, configured missing-media policy, and recovery are observed |
| NDI-HIL-08 | Loss/jitter/bandwidth | Bounded memory, no deadlock, and measured drop/recovery counters |
| NDI-HIL-09 | 24 h bidirectional run | No unbounded growth; cadence, xrun, drop, and A/V skew meet baseline gates |

## Current automated evidence

- Exact integer conversion between `MediaTime` and NDI 100 ns ticks
- Absolute audio sample-index/timestamp conversion
- Bounded latest-wins capture queue behavior in the native-feature test module
- Engine routes each `OutputId` from its owning Mixing Unit to its registered
  nonblocking `MediaSink`
- Runtime applies only the configured missing-media policy when a source has no
  frame

These checks do not satisfy any `NDI-HIL-*` scenario. The native-feature tests
also remain unexecuted until an NDI SDK/runtime host is available. On
2026-08-24, `cargo check -p eiviz-desktop --features ndi` on the current Linux
agent stopped in `grafton-ndi`'s build script because
`/usr/share/NDI SDK for Linux/include/Processing.NDI.Lib.h` was not installed;
no native adapter code or interoperability scenario was claimed as executed.
