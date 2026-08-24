# Clock mapping and A/V timing HIL

Status: **pending**. Integer mapper unit tests pass without hardware, but they
do not establish adapter timestamp semantics, genlock, or long-run A/V sync.

## Invariants under test

- Runtime deadlines use the process monotonic clock, never UTC.
- SourceMedia, DeckLinkStream, DeckLinkGenlock, AudioSample, PTP, and Virtual
  remain distinct domains.
- Every attached source has an explicit `ScheduleTime`, `ExactCorrelation`, or
  bounded policy. A bounded live source uses explicit `Fail` or `HoldLast`
  behavior while unlocked.
- Desktop metrics expose lock state, rate in ppb, offset/residual ticks,
  observations, duplicate/bounded/reset/wrap counts, video/audio skew, and A/V
  drift.

## Required equipment

1. 1080p59.94 / 48 kHz generator with timestamp and discontinuity controls.
2. DeckLink capture/playback endpoints and reference sync generator.
3. Separate NDI and OMT reference senders.
4. Audio analyzer/device with independently adjustable sample clock.
5. Monotonic trace capture and a fault injector for restart, jump, wrap,
   jitter, and reference disconnect.

Record OS, hardware/firmware/driver/SDK versions, source timestamp mode,
configured mapper bounds/policy, commit, and raw metrics.

## Acceptance scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| TIME-HIL-01 | SourceMedia correlation | File exact correlation and NDI/OMT bounded correlation lock without treating arrival time as media PTS |
| TIME-HIL-02 | DeckLink stream/genlock | Stream mapper locks; reference connect/disconnect is reported independently and never silently free-runs under `Fail` |
| TIME-HIL-03 | Audio sample clock | Sample-to-monotonic mapper reports injected drift within measurement tolerance and ASRC remains within configured correction bound |
| TIME-HIL-04 | Drift bounds | ±50/100/500 ppm injection converges; rate ppb, residual, and lock state are recorded; out-of-bound fits increment bounded count |
| TIME-HIL-05 | Jump/reset/relock | Timestamp jump and sender seek/restart increment reset, enter Acquiring, apply selected unlocked policy, and relock without stale offset |
| TIME-HIL-06 | Counter wrap/domain safety | Real or injected counter wrap increments wrap without false reset; wrong domain/timebase is a hard diagnostic |
| TIME-HIL-07 | A/V gate | After lock, A/V sync is P99 ±1 ms and maximum ±5 ms for each adapter/profile |
| TIME-HIL-08 | 24-hour soak | No UTC step affects cadence; no unbounded mapper memory; lock/reset/drift and Program drop/repeat/xrun meet release gates |

## Automated evidence

`eiviz-time` deterministically tests exact/inverse affine mapping, bounded drift
regression, jump reset, modular wrap, domain mismatch, and cross-domain
TimingIsland composition. `eiviz-runtime` tests explicit unlocked hard failure
and missing-correlation failure. These tests are prerequisites only; all
`TIME-HIL-*` rows remain pending until raw hardware evidence is attached.
