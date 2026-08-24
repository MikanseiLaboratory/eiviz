# OMT receive HIL

Status: **not yet executed**. Passing unit tests or loading `libomt` is not an
interop pass.

## Supported native library

- Official Open Media Transport `libomt` C ABI
- Verified header/API target: `v1.0.0.16`
- Official release asset SHA-256:
  `c70e67f7e2a7ed5b4c389d99af62796a8c9c7be23c8debfae3fd8020c1dc66b9`
- License: MIT

Official binary releases currently contain Windows x64/arm64 and macOS
libraries. Linux must build `libomt` from source; it is not silently replaced
with a simulator.

Set `EIVIZ_OMT_LIBRARY` to the exact **absolute** `libomt.dll`,
`libomt.dylib`, or `libomt.so` path, or package libomt and libvmx beside the
eiviz executable. Relative/system DLL lookup is intentionally rejected.
`EIVIZ_OMT_SOURCE` may contain an OMT discovery name or
`omt://hostname:port` to connect at desktop startup.

## Required equipment

1. Two separate machines on the same LAN.
2. Official OMT signal generator/reference sender on machine A.
3. eiviz plus official `libomt`/`libvmx` on machine B.
4. Packet loss/jitter injection between them for the fault cases.

## Acceptance scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| OMT-HIL-01 | Discovery | Sender name appears and selected address connects |
| OMT-HIL-02 | BGRA 1080p59.94 | Program pixels/color bars match reference; cadence measured |
| OMT-HIL-03 | UYVY BT.709 | Color conversion matches reference chart |
| OMT-HIL-04 | FPA1 48 kHz stereo | Channel order, level and timestamp verified |
| OMT-HIL-05 | TAKE/audio follow | OMT video and audio switch on the same logical boundary |
| OMT-HIL-06 | Tally/metadata | Preview/program tally and metadata round trip |
| OMT-HIL-07 | Sender restart | Explicit degraded state, reconnect, discontinuity marker |
| OMT-HIL-08 | Loss/jitter/reorder | Bounded queues; Program policy honored; no deadlock |
| OMT-HIL-09 | 24 h receive | Frame/audio counters and A/V skew within baseline gates |
| OMT-HIL-10 | Program send | Reference receiver gets BGRA video and FPA1 audio with monotonic timestamp |

## Current automated evidence

- ABI size/offset test for `OMTMediaFrame`
- BGRA/BGRX → RGBA conversion
- Premultiplied BGRA → straight-alpha RGBA conversion
- UYVY BT.709 → RGBA conversion
- FPA1 planar audio conversion
- Bounded 8-packet audio FIFO with explicit overflow diagnostic
- Truncated frame rejection
- Registered live `MediaSource` reaches Program and Audio Matrix
- RGBA/FPA1 Program output conversion and bounded sender queue

These tests do not satisfy any `OMT-HIL-*` scenario.
