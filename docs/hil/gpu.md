# GPU compositor HIL

Status: **not yet executed**. WGSL parser validation and feature tests are not
hardware certification.

## Automated evidence

- WGSL parses and validates with naga 25.
- Desktop dependency-tree gate resolves exactly one `wgpu v25.0.2`.
- A noop-device test constructs all compositor resources through the injected
  device path without requesting another adapter/device.
- Noop injected-device tests prewarm source/output/readback pools, render and
  read back multiple steady-state frames, and prove the allocation counter does
  not change. Separate tests prove hard pool-limit rejection, exact resident
  accounting, deterministic oldest-idle eviction, and unprepared-key failure.
- An injected loss test latches a device-loss report and proves subsequent GPU
  operations hard-fail without entering CpuReference. Engine lifecycle tests
  cover Active → Degraded → RestartRequired and show a CpuReference Engine is
  unaffected by GPU recovery requests.
- Wgpu backend rejects fallback/CPU adapters.
- Wgpu project cannot run on a CpuReference Runtime.
- Runtime Wgpu transitions call the GPU compositor, not CPU `mix_frames`.
- Pixel-space transform and crop uniform generation is tested.
- On a machine where a hardware adapter is visible to the test process, the
  feature test renders a red RGBA layer and checks readback pixels.
- Desktop Program/Preview/Multiview use egui-wgpu native texture registration
  on eframe's Device/Queue. `gpu_readbacks` counts explicit staging copies still
  required by `VideoFrame` consumers.
- Synthetic deadline/GPU samples deterministically prove auxiliary tier
  escalation, `Exhausted`, hysteretic recovery, and unchanged Program
  frame ID/PTS/dimensions/pixels. These tests do not replace hardware timing
  evidence.

## Required scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| GPU-HIL-01 | NVIDIA 1080p59.94 | golden layers/crop/rotation/opacity/transition; P99 pass time |
| GPU-HIL-02 | AMD 1080p59.94 | same evidence |
| GPU-HIL-03 | Intel 1080p59.94 | same evidence |
| GPU-HIL-04 | Device loss | callback report, Engine Degraded, Desktop restart-required, and frame-boundary same-backend reinjection are automated; pass still requires physical loss plus owner-created replacement Device/Queue |
| GPU-HIL-05 | GUI stress | Program cadence unaffected by editor/preview load |
| GPU-HIL-06 | Maximum admitted graph | VRAM/copy count/queue measurements |
| GPU-HIL-07 | 24 h compositor soak | zero internal Program drop/repeat |
| GPU-HIL-08 | Admission-controlled overload | inject GPU/GUI pressure across every configured tier; capture state transitions, Preview/Multiview decimation/drop/high-water counters, explicit `Exhausted` diagnostic, and bit-identical full-profile Program capture versus the no-pressure reference |

## Explicit profile

Run:

```bash
EIVIZ_COMPOSITOR=wgpu cargo run -p eiviz-desktop --features wgpu-backend
```

The process must fail if no hardware adapter exists. It must not switch to
CpuReference.

For non-GUI HIL only, `WgpuCompositor::new_headless_hardware` explicitly creates
a separate hardware device. Desktop must use `from_shared_device` with eframe's
render state.

## Remaining certification gap

ADR-0011's version/device unification, native texture path, bounded reusable
resource pools, activation prewarm, and explicit device-loss lifecycle are
implemented. The media graph still materializes `VideoFrame` for CPU-frame
sinks and mixfeed/multiview construction; every such GPU staging copy is
counted. Source adapters must deliver the snapshot-negotiated dimensions:
an unannounced dimension/working-format key is rejected instead of allocating
inside a frame.

eframe 0.32 has no public in-place `RenderState` recreation API. Desktop
therefore marks `restart-required` and offers a clean close. Embedders that can
create a replacement shared compositor may queue it; Engine installs it only
at a frame boundary after re-prewarming the active snapshot. The automated
noop/mock results are not physical device-loss evidence. GPU-HIL-01..08 remain
unexecuted, so this path is not Certified.
