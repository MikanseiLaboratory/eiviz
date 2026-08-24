# ADR-0015 Remove the incompatible gpu-video capability stub

- Status: Accepted
- Date: 2026-08-24

## Context

eiviz uses one exact wgpu 25.0.2 Device/Queue shared by eframe and the
compositor (ADR-0011). The published `gpu-video` 0.4.0 API is built on wgpu 29
and Vulkan Video; its upstream development branch has moved beyond that
generation. wgpu handle types from different major generations are distinct
Rust types and cannot share eiviz's device, textures, or synchronization.

The repository contained an empty feature and a probe that always returned
unavailable. That was not an adapter and falsely suggested a compilable
capability path.

## Decision

Remove `eiviz-codec-gpu-video`, its feature, Desktop probe, workspace entry,
and notices. Do not maintain a capability stub. The explicit production path
remains the software/external `ProgramEncoderFactory` contract.

A future hardware-video proposal must first provide a wgpu-25-compatible API
that accepts the existing shared Device/Queue and texture ownership contract,
or update eframe, compositor, and the codec adapter to one wgpu generation in
one reviewed change. It must include Vulkan Video platform coverage and HIL;
it may not reintroduce an always-unavailable probe.

## Consequences

gpu-video is not an eiviz capability and is not listed in SBOM/notices.
Projects cannot select it, so there is no implicit fallback. Hardware encoding
remains future work rather than a fake feature.
