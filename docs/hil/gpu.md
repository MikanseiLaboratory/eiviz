# GPU compositor HIL

Status: **not yet executed**. WGSL parser validation and feature tests are not
hardware certification.

## Automated evidence

- WGSL parses and validates with naga 24.
- Wgpu backend rejects fallback/CPU adapters.
- Wgpu project cannot run on a CpuReference Runtime.
- Runtime Wgpu transitions call the GPU compositor, not CPU `mix_frames`.
- Pixel-space transform and crop uniform generation is tested.
- On a machine where a hardware adapter is visible to the test process, the
  feature test renders a red RGBA layer and checks readback pixels.

## Required scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| GPU-HIL-01 | NVIDIA 1080p59.94 | golden layers/crop/rotation/opacity/transition; P99 pass time |
| GPU-HIL-02 | AMD 1080p59.94 | same evidence |
| GPU-HIL-03 | Intel 1080p59.94 | same evidence |
| GPU-HIL-04 | Device loss | explicit degraded policy, resource rebuild, frame-boundary recovery |
| GPU-HIL-05 | GUI stress | Program cadence unaffected by editor/preview load |
| GPU-HIL-06 | Maximum admitted graph | VRAM/copy count/queue measurements |
| GPU-HIL-07 | 24 h compositor soak | zero internal Program drop/repeat |

## Explicit profile

Run:

```bash
EIVIZ_COMPOSITOR=wgpu cargo run -p eiviz-desktop --features wgpu-backend
```

The process must fail if no hardware adapter exists. It must not switch to
CpuReference.

## Known architecture gap

See ADR-0011. eframe currently uses wgpu 25 while the pinned compositor uses
wgpu 24, so they own separate devices and the desktop path uses explicit
staging readback. Single-device texture sharing is not yet implemented.
