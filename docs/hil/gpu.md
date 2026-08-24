# GPU compositor HIL

Status: **not yet executed**. WGSL parser validation and feature tests are not
hardware certification.

## Automated evidence

- WGSL parses and validates with naga 25.
- Desktop dependency-tree gate resolves exactly one `wgpu v25.0.2`.
- A noop-device test constructs all compositor resources through the injected
  device path without requesting another adapter/device.
- Wgpu backend rejects fallback/CPU adapters.
- Wgpu project cannot run on a CpuReference Runtime.
- Runtime Wgpu transitions call the GPU compositor, not CPU `mix_frames`.
- Pixel-space transform and crop uniform generation is tested.
- On a machine where a hardware adapter is visible to the test process, the
  feature test renders a red RGBA layer and checks readback pixels.
- Desktop Program/Preview/Multiview use egui-wgpu native texture registration
  on eframe's Device/Queue. `gpu_readbacks` counts explicit staging copies still
  required by `VideoFrame` consumers.

## Required scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| GPU-HIL-01 | NVIDIA 1080p59.94 | golden layers/crop/rotation/opacity/transition; P99 pass time |
| GPU-HIL-02 | AMD 1080p59.94 | same evidence |
| GPU-HIL-03 | Intel 1080p59.94 | same evidence |
| GPU-HIL-04 | Device loss | callback report and explicit failure are implemented; prove owner-driven render-state/device recreation and frame-boundary recovery before passing |
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

For non-GUI HIL only, `WgpuCompositor::new_headless_hardware` explicitly creates
a separate hardware device. Desktop must use `from_shared_device` with eframe's
render state.

## Remaining certification gap

ADR-0011's version/device unification and native texture path are implemented.
The media graph still materializes `VideoFrame` for CPU-frame sinks and
mixfeed/multiview construction; every such GPU staging copy is counted.
Device loss is reported and GPU operations stop, but automatic recreation is
not implemented because eframe owns the injected device. GPU-HIL-01..07 remain
unexecuted, so this path is not Certified.
