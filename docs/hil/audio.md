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
Record the project `audio.resampling` value verbatim. Run rate-mismatch cases
once with `ExactRate` and once with each ASRC profile intended for shipment.

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
| AUD-HIL-11 | Set project to 48 kHz `ExactRate`, connect a 44.1 kHz-only capture device/source, and attempt start/pull | Start or first audio boundary fails visibly with both rates; no converter, alternate format, or backend starts |
| AUD-HIL-12 | Select `Asrc/Broadcast`, repeat 44.1 kHz capture into a 48 kHz project for 30 minutes with independent clocks | Output duration follows project clock; channels/polarity remain distinct; ratio and drift converge within the profile limit; buffers remain bounded |
| AUD-HIL-13 | Route a 48 kHz project to a 44.1 kHz-only CPAL output under `Asrc/Broadcast` | Device opens at 44.1 kHz, conversion occurs before the callback ring, and callback under/overflow remains zero after startup |
| AUD-HIL-14 | Inject a timestamp/sample-index discontinuity, cable clock loss, and source format restart | ASRC reset count increments, stale filter history is not replayed, and an unmarked format change is rejected |
| AUD-HIL-15 | Apply ±50, ±100, and the certified maximum ppm offset between source/device and project clocks | Reported drift has correct sign and settles near the analyzer value; ratio stays within the selected profile limit; no unbounded queue growth |
| AUD-HIL-16 | Repeat mismatch and discontinuity cases for OMT, NDI, DeckLink, CPAL, and every shipping file decoder | Every adapter follows the same persisted policy; none performs backend-local or implicit resampling |

## Evidence

Archive the project binding JSON, logs, Engine/Desktop audio diagnostics,
analyzer capture, callback-size trace, sample-index/timestamp comparison, xrun
counters, negotiated source/device rates, ASRC ratio/drift/reset counters, and
24-hour memory/queue high-water plots. Mark startup underflow separately from
steady-state underflow. Real-device, hot-plug, and soak results remain pending
until this evidence exists for each shipping profile.
