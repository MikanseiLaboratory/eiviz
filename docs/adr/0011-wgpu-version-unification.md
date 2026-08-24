# ADR-0011 Unify compositor and egui wgpu generation

- Status: Accepted
- Date: 2026-08-24

## Context

The pinned direct compositor dependency is wgpu 24.0.5. `eframe 0.32.3`
actually resolves `egui-wgpu 0.32.3` to wgpu 25.0.2. Therefore the current
Wgpu vertical slice owns a wgpu 24 Device/Queue and eframe owns a separate
wgpu 25 Device/Queue.

The compositor is functional with explicit staging readback, but this does not
satisfy R05.1 (one Device/Queue owner per physical GPU), cannot share output
textures with egui, and adds an avoidable GPU→CPU→GPU transfer.

## Decision

Pin the direct compositor dependency to exact wgpu 25.0.2, the generation
resolved by eframe/egui-wgpu 0.32.3. Desktop constructs `WgpuCompositor` with
the adapter, cloned Device handle, and cloned Queue handle from
`CreationContext::wgpu_render_state`; these clones refer to the same logical
device and queue. Desktop never invokes the headless device constructor.

`WgpuCompositor::new_headless_hardware` remains available only for processes
without eframe and for HIL. It requests a high-performance adapter, rejects
CPU-type adapters, and has no CPU fallback.

Compositor output is exposed as `WgpuTextureFrame`. Runtime keeps stream-specific
Program, Preview, and Multiview texture slots, and desktop registers those
texture views with egui-wgpu's native texture registry. This removes the
GPU→CPU→GUI GPU preview transfer. The existing media graph still requires
`VideoFrame` for software/network/hardware sinks and mixfeed construction, so
those explicit staging readbacks remain and increment `gpu_readbacks`.

wgpu 25 device-loss callbacks populate diagnostics and subsequent compositor
operations fail explicitly. Automatic device recreation is **not implemented**:
eframe owns an injected desktop device and must recreate its render state before
a new compositor can be injected. No recovery or certification claim is made.

## Verification

- `cargo tree -p eiviz-desktop --features wgpu-backend -i wgpu` must resolve one
  node only: `wgpu v25.0.2`, shared by eframe, egui-wgpu, and eiviz-gpu.
- CI executes that command as the version-singleton gate.
- naga 25 validates WGSL without hardware.
- a wgpu noop device exercises injected-device pipeline construction without
  requesting an adapter or second device.
- hardware feature tests remain conditional and explicitly skip when no
  hardware adapter is available.

This implements dependency and desktop device unification. It does not certify
the HIL scenarios in `docs/hil/gpu.md`.

## Alternatives

1. Downgrade eframe to a release using wgpu 24 — rejected unless UI dependency
   compatibility is re-evaluated.
2. Keep two wgpu generations — functional but fails R05.1 and the zero-copy
   architecture target.
3. Move compositor to a separate process — preserves version isolation but
   adds IPC copies and is not the current desktop architecture.
