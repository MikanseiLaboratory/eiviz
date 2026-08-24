# ADR-0014 Explicit audio sample-rate conversion

- Status: Accepted
- Date: 2026-08-24

## Context

Live OMT, NDI, DeckLink, CPAL, and decoded-file sources can run on clocks that
do not equal the project clock. A stateless linear interpolation helper neither
handled clock drift nor preserved filter history and was not connected to the
runtime. Silently invoking it would violate the no-fallback policy.

## Decision

`Project.audio.resampling` is mandatory model state. Legacy projects deserialize
as `ExactRate`, which makes any source or output-device rate mismatch a hard
error. An operator can instead select a persisted `Asrc` profile (`Broadcast`
or `Mastering`) with explicit latency, buffer, and drift limits.

Runtime `AudioPlan` owns one stateful converter per mismatched input. The
converter is a Blackman-windowed sinc (32 or 64 taps), keeps filter history,
preserves all source channels, estimates source-clock error from capture
timestamps, and steers its ratio against a bounded queue. Discontinuities or
sample-index jumps clear history. Buffer underflow emits counted silence;
overflow drops bounded old history, marks a discontinuity, and is counted.

CPAL first requests the exact project format. Under `ExactRate`, failure remains
a hard error. Under an explicit ASRC profile, it may select the closest
same-channel device rate. Capture conversion occurs in Runtime. Playback
conversion occurs before the CPAL ring and uses ring occupancy to follow the
independent output clock. No conversion runs in a device callback.

All media sources cross the same Runtime `AudioPlan` boundary, so OMT, NDI,
DeckLink, CPAL, and file-decoder audio use the same policy. A source changing
format without a discontinuity marker is rejected.

## Consequences

Desktop shows and persists the selected policy and reports current ratio, drift,
buffer occupancy, underflow, overflow, and reset counts. Exact-rate operation
has no converter state. Unit tests certify 44.1/48 kHz duration, tone frequency,
channel preservation, discontinuity reset, bounded overflow, and exact-policy
failure. Hardware clock, callback, and hot-plug behavior remains a HIL gate.
