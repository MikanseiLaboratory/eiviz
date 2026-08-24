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
  Activated snapshots prewarm bounded reusable source/output/readback pools;
  steady-state rendering cannot create an unprepared resident GPU resource.
  Device loss explicitly degrades Engine, never selects CpuReference, and only
  accepts a re-prewarmed replacement compositor at a frame boundary. Desktop
  requires restart because eframe 0.32 cannot recreate RenderState in place.
  Physical GPU/device-loss HIL remains pending.
- Native GUI (`apps/eiviz-desktop`) talking **only** through `CommandEnvelope`
- Desktop starts authenticated-capable HTTP (`127.0.0.1:8090`), TCP JSON-lines
  (`:8091`), and WebSocket (`:8092`) Query/Command APIs. Versioned envelopes,
  expected revision, idempotent command IDs, atomic transactions, rate limits,
  bounded command/event queues, and WebSocket revision events are preserved.
  Set `EIVIZ_CONTROL=off` to disable.
- Optional real MIDI input uses `midir` and requires explicit backend-stable
  device selection plus channel-message mapping to versioned envelopes. The
  default build has no MIDI listener or no-op substitute.
- Mixing Units expose rendered multi-tile Multiview frames for Input, Preview,
  and Program sources. Every NDI/OMT/DeckLink/distribution Output independently
  selects the owning Unit's Program or a whole Multiview in Desktop.
- Desktop can ingest PNG/JPEG files into the content-addressed asset store,
  create a fullscreen Preview scene, and export/import portable `.eiviz`
  packages without replacing the running control-server Engine instance.
- Desktop can ingest constrained H.264 video-only or H.264/AAC-LC MP4 through
  explicit end-user-installed Cisco OpenH264 2.6.0 and, when AAC is present,
  license-reviewed FDK AAC binary paths. Shiguredo MP4 supplies `avc1`,
  `mp4a`/`esds` AudioSpecificConfig, sample timing, and exposed edit-list
  priming. A coordinated source resets both decoders on seek/loop and publishes
  video and planar audio on one presentation timeline. ExactRate rejects AAC
  rate mismatch; only the persisted ASRC policy permits conversion to project
  rate. AAC is never dropped or replaced when FDK is absent. Desktop reports
  video-only versus A/V admission. A Linux x64 OpenH264 video smoke test passed;
  representative A/V, conformance, and cross-platform HIL remain pending.
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
- Production-shaped distribution after the encoder boundary: one immutable
  H.264/AAC access-unit allocation fans out to independent bounded fMP4, RTMP
  FLV, and SRT MPEG-TS workers with reconnect-at-IDR and fMP4 tail recovery.
  `rml_rtmp` and `srt-tokio` local protocol peers are covered in default CI.
  Product activation dynamically loads an explicitly selected Cisco OpenH264
  2.6.0 binary (allow-listed SHA-256 plus runtime version check) and an
  explicitly selected, license-reviewed FDK AAC binary for raw AAC-LC. Neither
  binary is bundled or built from source. Missing/rejected binaries hard-fail;
  I_PCM, PCM, test bytes, and alternate codecs are never substituted.
  Real-server/loss/24-hour HIL remains pending.
- Persisted, truth-preserving capability reports and CI-generated SPDX 2.3 /
  CycloneDX 1.5 SBOM artifacts
- Unsigned MSIX (Windows x64), pkg (macOS arm64), and deb (Ubuntu 24.04 x64)
  packaging smoke profiles. Release signing/notarization is a separate
  secret-gated workflow; no signed release is claimed without those keys.
- Structured operational spans/metrics, a bounded 45-second redacted flight
  recorder, and crash reports containing recent diagnostics plus project hash
- Portable `.eiviz` packages, crash-safe JSON save, and explicit Desktop
  autosave recover/discard startup UX. Deterministic corruption, truncation,
  path-traversal/hash-spoof, and injected disk-write failure tests run in CI.
- Audio callback regions are statically guarded against allocation, blocking,
  I/O, synchronous logging, and panic primitives. CI also runs portable Loom,
  Miri, and AddressSanitizer subsets. Certification resident memory uses Linux,
  Windows, and macOS platform APIs.

Hardware/interoperability HIL (OMT, DeckLink genlock, NDI round-trip, audio
devices, RTMP, and SRT) is **not** claimed. gpu-video is not a capability
because its wgpu generation cannot share the unified device (ADR-0015). See the current truth table in
the implementation plan, the [DeckLink HIL procedure](docs/hil/decklink.md),
the [audio HIL procedure](docs/hil/audio.md), and the
[file-media HIL procedure](docs/hil/file-media.md), and the
[distribution HIL procedure](docs/hil/distribution.md). Operational export,
recovery semantics, and SBOM generation are documented in
[operations](docs/operations.md). Native package profiles, upgrade rollback,
SDK exclusions, and signing verification are documented in
[release packaging](docs/packaging.md).

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

# H.264/AAC file ingest and distribution require an operator-provided FDK AAC
# shared library whose license/patent profile has been reviewed for deployment.
EIVIZ_OPENH264_PATH=/absolute/path/to/cisco/libopenh264-2.6.0.so \
EIVIZ_FDK_AAC_PATH=/absolute/path/to/libfdk-aac.so \
  cargo run -p eiviz-desktop

# Opt-in real-binary A/V file HIL (ignored by default).
EIVIZ_FILE_HIL_OPENH264=/absolute/path/to/cisco/libopenh264-2.6.0.so \
EIVIZ_FILE_HIL_FDK_AAC=/absolute/path/to/libfdk-aac.so \
EIVIZ_FILE_HIL_MP4=/absolute/path/to/representative-avc-aac.mp4 \
  cargo test -p eiviz-io-file decodes_real_h264_aac_mp4_with_explicit_binaries -- --ignored

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

# Real platform MIDI input (WinMM/CoreMIDI/ALSA). Linux requires ALSA
# development packages. Select the port and TAKE mapping in Desktop.
cargo run -p eiviz-desktop --features midi
```

Control binds are `EIVIZ_HTTP_BIND`, `EIVIZ_TCP_BIND`, and `EIVIZ_WS_BIND`.
Loopback is the default. Any non-loopback bind is rejected unless a non-empty
`EIVIZ_CONTROL_TOKEN` is configured; when configured, it protects health,
queries, events, and commands on every transport. HTTP and WebSocket use
`Authorization: Bearer`; TCP includes `token` in each versioned request.
`EIVIZ_CONTROL_RATE`, `EIVIZ_CONTROL_QUEUE`, `EIVIZ_CONTROL_EVENT_QUEUE`, and
`EIVIZ_CONTROL_MAX_CONNECTIONS` configure bounded admission. These listeners
are plaintext: use a trusted management network or TLS reverse proxy remotely.
See [control/MIDI HIL](docs/hil/control-midi.md) for protocol examples and
physical-device verification, and the [Control API reference](docs/control-api.md)
for wire formats.

Windows x64 is the first-class target. Linux/macOS compile the same core.
MSRV is 1.97; the repo `rust-toolchain.toml` pins the CI/dev toolchain.
Release packages use explicit `wgpu-backend,audio-cpal,midi` profiles and never
bundle NDI, DeckLink, ASIO, OpenH264, or FDK AAC implicitly.

## Layout

See [directorystructure.md](directorystructure.md) and [technologystack.md](technologystack.md).
Requirements live in [docs/requirements.md](docs/requirements.md).

## License

MIT for this repository. Third-party SDKs (NDI, DeckLink, ASIO, codecs) keep
their own terms and are never compiled in by default. DeckLink builds compile
the repository's MIT C ABI shim against a separately installed official SDK;
the SDK headers, dispatch/interface source, driver, and tools remain under
Blackmagic Design's terms. See [NOTICE](NOTICE).
