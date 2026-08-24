# eiviz

Cross-platform GPU vision mixer (映像 + viz). Rust workspace implementing
Issue #1: mixing units, scenes, command sequencer, rational 59.94 timing,
and feature-gated I/O adapters.

## Status

Phase 0–1 of the architecture plan are in-tree:

- Deterministic core (`eiviz-time`, `eiviz-core`, `eiviz-command`, `eiviz-project`)
- Explicit CpuReference compositor + virtual-clock runtime (`eiviz-gpu`, `eiviz-runtime`, `eiviz-engine`). `Wgpu` is a separate project backend, never a silent fallback.
- Native GUI (`apps/eiviz-desktop`) talking **only** through `CommandEnvelope`
- Adapter stubs for NDI / OMT / DeckLink / ASIO / gpu-video (capability probes)
- Portable `.eiviz` packages and crash-safe JSON save

Hardware HIL (DeckLink genlock, NDI round-trip, Vulkan Video) is **not** claimed.

## Build

```bash
rustup toolchain install 1.85
cargo test --workspace
cargo run -p eiviz-desktop
```

Windows x64 is the first-class target. Linux/macOS compile the same core.
MSRV is 1.85; the repo `rust-toolchain.toml` pins the CI/dev toolchain.

## Layout

See [directorystructure.md](directorystructure.md) and [technologystack.md](technologystack.md).
Requirements live in [docs/requirements.md](docs/requirements.md).

## License

MIT for this repository. Third-party SDKs (NDI, DeckLink, ASIO, codecs) keep
their own terms and are never compiled in by default.
