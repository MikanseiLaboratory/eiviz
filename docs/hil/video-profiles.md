# Phase 9 extended video profile HIL

Status: **not yet executed / not hardware certified**. Deterministic unit tests,
WGSL validation, adapter capability checks, and successful builds are not
hardware evidence. The required 1080p59.94 SDR baseline remains unchanged.

## Profiles

- `PROFILE-2160P5994`: 3840×2160 progressive, `60000/1001`, BT.709 SDR, 8-bit.
- `PROFILE-HDR10-PQ`: 3840×2160 progressive, `60000/1001`, BT.2020
  non-constant-luminance, limited range, PQ, 10-bit.
- `PROFILE-HDR-HLG`: same raster/cadence/range at HLG transfer.
- `PROFILE-1080I5994`: 1920×1080 at exact `60000/1001` field boundaries with
  explicit top-field-first or bottom-field-first order.

## Required scenarios

| ID | Scenario | Required evidence |
| --- | --- | --- |
| VP-HIL-01 | 2160p59.94 SDR GPU | Adapter identity/features, uninterrupted cadence, GPU P99, VRAM high-water, and reference-chart capture |
| VP-HIL-02 | PQ 10-bit P010 | P010 code-value chart through WGSL into RGBA16F, metadata preservation, external HDR waveform/reference display |
| VP-HIL-03 | HLG 10-bit P216 | P216 4:2:2 sampling and HLG chart verified by an independent analyzer |
| VP-HIL-04 | Explicit color conversion | Exact policy rejects mismatch; selected WGSL matrix/range/transfer conversion matches reference vectors |
| VP-HIL-05 | Explicit tone map | HDR→SDR is rejected when disabled; selected peak/target policy is measured against reference output |
| VP-HIL-06 | 1080i59.94 TFF/BFF | One million exact rational field timestamps, alternating field identity, analyzer-confirmed field dominance/cadence |
| VP-HIL-07 | Adapter rejection matrix | Unsupported NDI/OMT/DeckLink/file/distribution combinations fail before start and never select another profile |
| VP-HIL-08 | Admission/VRAM limit | Dimension, texture-format, and configured VRAM limits reject activation with actionable diagnostics |
| VP-HIL-09 | Extended-profile 24 h soak | Zero internal Program drop/repeat, bounded memory, recorded GPU/deadline/A/V metrics |
| VP-HIL-10 | Baseline regression | Full existing 1080p59.94 suite remains bit/cadence compatible and meets every baseline gate |

Record GPU/card model, driver and vendor SDK versions, OS, display/analyzer,
project profile and policy, adapter capability report, commit, raw captures, and
metric logs. Do not mark a row passed from a simulator, noop adapter, unit test,
or same-process loopback.

## Current adapter boundary

- Wgpu checks raster limits and required RGBA16F render/filter capability before
  activation. P010/P216 code unpacking feeds WGSL color processing; no CPU
  compositor fallback exists.
- NDI and OMT output paths currently accept only their explicit 8-bit
  progressive SDR profiles. The fixed DeckLink SDK shim and H.264 file/
  distribution paths accept only the baseline profile. Their extended-profile
  errors are intentional capability results, not pending conversion requests.
- No `VP-HIL-*` scenario has been run on the current agent.
