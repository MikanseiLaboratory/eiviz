# eiviz

Cross-platform GPU vision mixer (映像 + viz). Rust workspace implementing
Issue #1: mixing units, scenes, command sequencer, rational 59.94 timing,
and feature-gated I/O adapters.

## Status

Parts of the architecture plan are in-tree. No phase is considered complete
until its implementation, runtime wiring, interop, and required HIL evidence
all pass:

- Deterministic core (`eiviz-time`, `eiviz-core`, `eiviz-command`, `eiviz-project`)
- Explicit CpuReference compositor + virtual-clock runtime (`eiviz-gpu`, `eiviz-runtime`, `eiviz-engine`). `Wgpu` is a separate project backend, never a silent fallback.
- The optional Wgpu backend now executes RGBA layer transform/crop/rotation,
  alpha composition, transition mixing, and staging readback on a hardware GPU.
  GPU HIL and device-loss recovery remain pending.
- Native GUI (`apps/eiviz-desktop`) talking **only** through `CommandEnvelope`
- Desktop starts the localhost HTTP Command API on `127.0.0.1:8090` and TCP
  JSON-lines API on `127.0.0.1:8091`. Set `EIVIZ_CONTROL=off` to disable.
- Real OMT receive/output adapter through the official `libomt` C ABI
  (runtime-loaded); interop HIL is still pending
- Capability stubs only for NDI / DeckLink / ASIO / gpu-video
- Portable `.eiviz` packages and crash-safe JSON save

Hardware/interoperability HIL (OMT, DeckLink genlock, NDI round-trip, Vulkan
Video) is **not** claimed. See the current truth table in the implementation
plan and [OMT HIL procedure](docs/hil/omt.md).

## Build

```bash
rustup toolchain install 1.85
cargo test --workspace
cargo run -p eiviz-desktop

# Explicit hardware-GPU profile. Fails if no hardware adapter is available.
EIVIZ_COMPOSITOR=wgpu cargo run -p eiviz-desktop --features wgpu-backend
```

Control binds can be changed with `EIVIZ_HTTP_BIND` / `EIVIZ_TCP_BIND`, but
only loopback addresses are accepted until remote authorization is implemented.
`EIVIZ_CONTROL_TOKEN` protects HTTP commands; TCP remains localhost-only.

Windows x64 is the first-class target. Linux/macOS compile the same core.
MSRV is 1.85; the repo `rust-toolchain.toml` pins the CI/dev toolchain.

## Layout

See [directorystructure.md](directorystructure.md) and [technologystack.md](technologystack.md).
Requirements live in [docs/requirements.md](docs/requirements.md).

## License

MIT for this repository. Third-party SDKs (NDI, DeckLink, ASIO, codecs) keep
their own terms and are never compiled in by default.
