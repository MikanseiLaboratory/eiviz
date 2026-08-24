# Distribution HIL

Default CI proves queue isolation, reconnect state transitions, RTMP framing
against an in-process protocol server, SRT delivery against a local SRT
listener, MPEG-TS PIDs, and recoverable fMP4 box boundaries. It does **not**
certify a production H.264/AAC encoder or real-server interoperability.

## Dependency and license decision (Rust 1.97)

| Candidate | License / native surface | Rust 1.97 result | Decision |
| --- | --- | --- | --- |
| `rml_rtmp` 0.8.0 | MIT, pure Rust; upstream does not state an MSRV | Builds on 1.97 | In-tree RTMP session/framing |
| `srt-tokio` 0.4.4 | Apache-2.0, pure safe Rust; upstream does not state an MSRV | Builds on 1.97 | In-tree SRT caller; interop HIL required |
| GStreamer Rust 0.25.3 | bindings MIT/Apache-2.0, GStreamer LGPL-2.1+, plugin licenses vary; system packages required; MSRV 1.92 | Compatible, but not portable default CI | Not selected until encoder/plugin/SBOM policy is approved; any adapter must be feature-gated |
| `fdk-aac` 0.8.0 | Rust wrapper MIT; bundled FDK AAC has a bespoke license and no patent grant | Toolchain-compatible candidate | Not selected; requires a separately reviewed distribution profile |
| Cisco OpenH264 2.6.0 binary | Cisco binary license/patent terms | Decoder is dynamically loaded today | Encoder is not integrated; in-tree I_PCM remains test-only |

The Project profile names an encoder and transport. Missing capabilities are a
hard error. PCM is never substituted for AAC and I_PCM is never presented as a
production encoder.

## Real RTMP

1. Install a current MediaMTX or nginx-rtmp release on a separate machine.
2. Provision an H.264 Annex-B encoder and raw AAC-LC encoder adapter that has
   passed legal review; register their exact names in the distribution build.
3. Configure `rtmp://HOST:1935/APP/KEY`, 1080p59.94 BT.709, 48 kHz stereo,
   8 Mbit/s AVC, 192 kbit/s AAC, and a 120-frame GOP.
4. Confirm publish with the server log and probe the received stream with an
   independent player/analyzer. Record SPS/PPS, AAC AudioSpecificConfig,
   monotonic DTS/PTS, keyframe interval, and A/V offset.
5. Terminate the TCP session for 10 seconds. Confirm bounded local drops,
   exponential reconnect, AVC/AAC sequence headers after reconnect, and no
   media before the next IDR.

## Real SRT and loss

1. Run Haivision `srt-live-transmit` or a current MediaMTX SRT listener on a
   separate machine.
2. Configure caller mode with an explicit latency and stream ID. Verify the
   receiver sees PAT PID 0, PMT PID `0x1000`, AVC PID `0x0100`, and AAC PID
   `0x0101`.
3. On Linux, place `tc netem` between hosts at 1%, 3%, then 5% random loss,
   plus 50 ms jitter and a 10-second outage. Remove netem after each case.
4. Record SRT retransmit/loss statistics, queue high-water, dropped access
   units, reconnect count, recovery IDR time, continuity-counter errors, and
   independent decoder errors.

## Recording recovery

1. Record to a local filesystem and confirm two tracks (`avc1`, `mp4a`) plus
   repeated `moof`/`mdat` pairs with an independent MP4 analyzer.
2. Kill the process during an `mdat`. Restart with tail recovery enabled.
3. Confirm only the incomplete top-level box is truncated, append resumes at
   an IDR, all earlier fragments decode, and the recovered artifact retains
   AAC sync.
4. Repeat with a full disk and a removable-volume disconnect. Program cadence
   must continue and only the recording sink may fail.

## 24-hour gate

Run RTMP, SRT, and fMP4 from the same encoded fanout for 24 hours. Preserve:

- server and receiver logs;
- per-sink queue depth/high-water/drop/reconnect counters;
- encoder frame/keyframe counts and hashes proving one shared encode result;
- P50/P95/P99 A/V offset and reconnect-to-IDR time;
- Program deadline/drop/repeat and audio-xrun counters;
- final fMP4 recovery/analyzer report.

Acceptance requires zero Program stalls caused by a distribution sink, bounded
memory, no undecodable post-reconnect interval before IDR, and independent
real-tool playback. A local mock pass alone is not certification.
