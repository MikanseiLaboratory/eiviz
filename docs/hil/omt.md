# OMT receive HIL

Status: **not yet executed**. Passing unit tests or compiling the pure-Rust stack is not an
interop pass.

## Implementation

- Pure Rust `MikanseiLaboratory/openmediatransport-rs`
- Pure Rust `vmx-rs` VMX1 codec
- License: MIT
- No libomt/native-library path and no runtime fallback

`EIVIZ_OMT_SOURCE` may contain an OMT discovery name or
`omt://hostname:port` to connect at desktop startup.

## Required equipment

1. Two separate machines on the same LAN.
2. Official OMT signal generator/reference sender on machine A.
3. eiviz with `openmediatransport-rs`/`vmx-rs` on machine B.
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

- Adapter capability and test-only loopback tests
- Generic registered `MediaSource` → Program/Audio Matrix integration test
- Upstream `openmediatransport-rs`/`vmx-rs` protocol and codec test suites via pinned revisions

UYVY, tally/metadata surfacing, reconnect behavior, and reference-tool
interoperability remain HIL gaps rather than automated evidence.

These tests do not satisfy any `OMT-HIL-*` scenario.
