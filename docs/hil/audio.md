# Audio device HIL

This procedure certifies the production CPAL path. Unit tests cover binding and
buffer conversion without hardware; xrun, clock, hot-plug, and real driver
behavior require a private hardware runner.

## Profiles and prerequisites

Use exactly one explicitly selected host in the desktop UI. A failed host or
persisted device binding is an error; do not select another host or the system
default as a fallback.

- Windows WASAPI: `cargo run -p eiviz-desktop --features audio-cpal`
- macOS CoreAudio: `cargo run -p eiviz-desktop --features audio-cpal`
- Linux ALSA (including a configured PipeWire ALSA compatibility device):
  `cargo run -p eiviz-desktop --features audio-cpal`
- Native Linux PipeWire: install the matching PipeWire development packages,
  then use `--features audio-pipewire`.
- Windows ASIO: obtain and accept the current Steinberg ASIO SDK license,
  install LLVM/Clang and the vendor driver, set `CPAL_ASIO_DIR` to that
  operator-installed SDK, then use `--features audio-asio`. Do not build this
  profile without `CPAL_ASIO_DIR`; upstream `asio-sys` can otherwise download
  an SDK during its opt-in build. ASIO artifacts are a separate distribution
  profile and are not covered by the repository's MIT license.

Record the OS/build, CPAL version, host, persistent device IDs, driver and
firmware versions, sample format, negotiated callback size, and cable topology.

## Equipment

- Certified two-input/two-output (or larger) interface and vendor driver.
- A second clocked analyzer/interface or hardware loopback.
- 48 kHz stereo reference PCM with per-channel identity and absolute sample
  markers.
- For drift testing, an independent monotonic/PTP reference where available.

## Cases

| ID | Procedure | Pass condition |
| --- | --- | --- |
| AUD-HIL-01 | Enumerate each compiled host twice and restart eiviz | Persistent IDs remain stable; input/output/default flags match the OS |
| AUD-HIL-02 | Save a binding, rename the device, restart, and reconnect | Persistent ID resolves the same device; no default-device substitution |
| AUD-HIL-03 | Remove the bound device and start capture/output | Start fails visibly; no synthetic device, tone, or alternate backend starts |
| AUD-HIL-04 | Capture known interleaved PCM at 48 kHz | Planar channels, polarity, level, sample order, and sample indices match |
| AUD-HIL-05 | Route Program master to output and loop back | Output channel order and levels match; callback playback timestamps are monotonic |
| AUD-HIL-06 | Run 64/128/256-frame callback requests supported by the device | No callback allocation/blocking evidence; queue bounds remain fixed |
| AUD-HIL-07 | Unplug/replug and restart the driver while active | Health/error/xrun counters change; no backend or device fallback occurs |
| AUD-HIL-08 | Saturate producer and consumer independently | Overflow/underflow is bounded and counted; Program/video cadence continues |
| AUD-HIL-09 | Run the certified workload for 24 hours | Internal xrun count is zero and sample-clock drift meets the certification gate |
| AUD-HIL-10 | Repeat AUD-HIL-01 through 09 for WASAPI/CoreAudio/ALSA/PipeWire/ASIO profiles in scope | Evidence is stored separately per host/profile |

## Evidence

Archive the project binding JSON, logs, Engine/Desktop audio diagnostics,
analyzer capture, callback-size trace, sample-index/timestamp comparison, xrun
counters, and 24-hour memory/queue high-water plots. Real-device, hot-plug, and
soak results remain pending until this evidence exists for each shipping
profile.
