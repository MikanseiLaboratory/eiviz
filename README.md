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
- Mixing Units expose rendered multi-tile Multiview frames for Input, Preview,
  and Program sources; the desktop bootstraps a PRV/PGM two-up view.
- Desktop can ingest PNG/JPEG files into the content-addressed asset store,
  create a fullscreen Preview scene, and export/import portable `.eiviz`
  packages without replacing the running control-server Engine instance.
- Desktop can ingest the constrained H.264 MP4 profile through an explicit
  end-user-installed Cisco OpenH264 2.6.0 binary path, decode I420 to BT.709
  limited-range RGBA, and play/pause/seek/loop. There is no decoder fallback.
  A Linux x64 smoke test passed with Cisco's hash-verified binary and a
  generated Constrained Baseline clip; representative conformance and
  certification HIL remain pending.
- Real OMT receive/output through pure-Rust `openmediatransport-rs` / `vmx-rs`;
  interop HIL is still pending
- Optional real NDI discovery/receive/output through `grafton-ndi` 1.0.0 and
  an installed NDI 6 SDK/runtime. Video and planar audio use bounded adapter
  workers and NDI's 100 ns timestamps. NDI interop HIL is still pending.
- Optional real Blackmagic DeckLink capture and scheduled playback through an
  official SDK 16 C++ shim. The fixed vertical slice is 1080p59.94 BGRA with
  48 kHz PCM, persistent device binding, bounded queues, and reference-lock /
  completion diagnostics. DeckLink interop and genlock HIL are still pending.
- Optional real CPAL input/output for explicit WASAPI, CoreAudio, ALSA, and
  native PipeWire hosts, with persistent device IDs, bounded lock-free callback
  queues, planar f32 conversion, device sample-clock timestamps, and xrun/queue
  diagnostics. ASIO is a separate opt-in licensed profile. Audio device HIL is
  still pending; unavailable selected devices/hosts never fall back.
- Capability stub only for gpu-video
- Portable `.eiviz` packages and crash-safe JSON save

Hardware/interoperability HIL (OMT, DeckLink genlock, NDI round-trip, Vulkan
Video, and audio devices) is **not** claimed. See the current truth table in
the implementation plan, the [DeckLink HIL procedure](docs/hil/decklink.md),
and the [audio HIL procedure](docs/hil/audio.md).

## Build

```bash
rustup toolchain install 1.97
cargo test --workspace
cargo run -p eiviz-desktop

# Explicit hardware-GPU profile. Fails if no hardware adapter is available.
EIVIZ_COMPOSITOR=wgpu cargo run -p eiviz-desktop --features wgpu-backend

# Optional initial value for the desktop's explicit binary-path field.
EIVIZ_OPENH264_PATH=/absolute/path/to/libopenh264-2.6.0-linux64.8.so \
  cargo run -p eiviz-desktop

# Explicit NDI profile. NDI 6 SDK headers and runtime must already be installed.
# Set NDI_SDK_DIR for a nonstandard SDK location and make the runtime library
# discoverable using the platform's documented loader mechanism.
cargo run -p eiviz-desktop --features ndi

# Explicit DeckLink profile. Desktop Video and SDK 16 must be installed.
# The SDK remains external and is never downloaded or vendored.
DECKLINK_SDK_DIR=/absolute/path/to/Blackmagic_DeckLink_SDK_16 \
  cargo run -p eiviz-desktop --features decklink

# Production OS-default CPAL host: WASAPI, CoreAudio, or ALSA.
cargo run -p eiviz-desktop --features audio-cpal

# Explicit native PipeWire host; requires PipeWire development packages.
cargo run -p eiviz-desktop --features audio-pipewire

# Separately licensed Windows ASIO profile. CPAL_ASIO_DIR is mandatory for
# eiviz builds so the SDK is operator-installed and license-reviewed.
CPAL_ASIO_DIR=C:\path\to\asiosdk \
  cargo run -p eiviz-desktop --features audio-asio
```

Control binds can be changed with `EIVIZ_HTTP_BIND` / `EIVIZ_TCP_BIND`, but
only loopback addresses are accepted until remote authorization is implemented.
`EIVIZ_CONTROL_TOKEN` protects HTTP commands; TCP remains localhost-only.

Windows x64 is the first-class target. Linux/macOS compile the same core.
MSRV is 1.97; the repo `rust-toolchain.toml` pins the CI/dev toolchain.

## Layout

See [directorystructure.md](directorystructure.md) and [technologystack.md](technologystack.md).
Requirements live in [docs/requirements.md](docs/requirements.md).

## License

MIT for this repository. Third-party SDKs (NDI, DeckLink, ASIO, codecs) keep
their own terms and are never compiled in by default. DeckLink builds compile
the repository's MIT C ABI shim against a separately installed official SDK;
the SDK headers, dispatch/interface source, driver, and tools remain under
Blackmagic Design's terms. See [NOTICE](NOTICE).
