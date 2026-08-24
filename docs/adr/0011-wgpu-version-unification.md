# ADR-0011 Unify compositor and egui wgpu generation

- Status: Proposed — version change requires approval
- Date: 2026-08-24

## Context

The pinned direct compositor dependency is wgpu 24.0.5. `eframe 0.32.3`
actually resolves `egui-wgpu 0.32.3` to wgpu 25.0.2. Therefore the current
Wgpu vertical slice owns a wgpu 24 Device/Queue and eframe owns a separate
wgpu 25 Device/Queue.

The compositor is functional with explicit staging readback, but this does not
satisfy R05.1 (one Device/Queue owner per physical GPU), cannot share output
textures with egui, and adds an avoidable GPU→CPU→GPU transfer.

## Proposed decision

Unify the direct compositor dependency with eframe's wgpu generation, then
inject eframe's render-state Device/Queue into `WgpuCompositor` instead of
creating a second device. Keep the headless constructor only for output workers
and HIL tools that do not run eframe.

This requires changing the pinned direct wgpu version and adapting its API. It
must not be performed silently. Until approved and implemented:

- Wgpu composition is `Runtime wired`, not `Certified`.
- staging readback is explicit and measured as a known copy;
- no zero-copy or single-device claim is made.

## Alternatives

1. Downgrade eframe to a release using wgpu 24 — rejected unless UI dependency
   compatibility is re-evaluated.
2. Keep two wgpu generations — functional but fails R05.1 and the zero-copy
   architecture target.
3. Move compositor to a separate process — preserves version isolation but
   adds IPC copies and is not the current desktop architecture.
