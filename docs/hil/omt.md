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
| OMT-HIL-09 | 24 h receive | SourceMedia→Monotonic remains locked; frame/audio counters, reset/drift, and A/V skew are within baseline gates |
| OMT-HIL-10 | Program send | Reference receiver gets BGRA video and FPA1 audio with monotonic timestamp |

## Current automated evidence

- Adapter capability and test-only loopback tests
- Generic registered `MediaSource` → Program/Audio Matrix integration test
- Upstream `openmediatransport-rs`/`vmx-rs` protocol and codec test suites via pinned revisions
- OMT video/audio receive stamps adapter capture with process monotonic and
  Runtime exposes bounded mapper lock and A/V drift metrics
- Bounded metadata receive queues classify OMT control/application payloads and
  surface them through Engine/Desktop diagnostics
- Engine derives receiver preview/program tally from visible SceneItems on the
  latched Preview and Program buses, including TAKE boundaries
- Deterministic BGRA channel-layout and explicit BT.601/BT.709 limited-range
  UYVY conversion tests
- Adapter reconnect counters and first-recovered-frame discontinuity accounting

Reference-tool color, tally/metadata wire interoperability, real sender restart,
and long-run behavior remain HIL gaps. The deterministic tests do not establish
that an external OMT endpoint accepts either packed output path.

These tests do not satisfy any `OMT-HIL-*` scenario.

The common `TIME-HIL-01`, `04..08` scenarios in [timing.md](timing.md) are
required in addition to `OMT-HIL-*`.
