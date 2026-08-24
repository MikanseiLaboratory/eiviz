# DeckLink capture/playback and genlock HIL

Status: **pending**. The SDK and hardware are absent from the current agent.
Default-feature unit tests do not establish Desktop Video interoperability,
signal integrity, or genlock behavior.

## Build and licensing prerequisites

1. Install Blackmagic Desktop Video and the separately downloaded Desktop
   Video SDK 16 from Blackmagic Design.
2. Review and accept the license shipped with that SDK. Do not copy SDK
   headers, samples, tools, or binaries into this repository.
3. Set `DECKLINK_SDK_DIR` to the extracted SDK root. The build locates
   `DeckLinkAPI.h` and the official platform dispatch/interface source below
   that root.
4. Build the explicit profile:

   ```bash
   DECKLINK_SDK_DIR=/absolute/path/to/Blackmagic_DeckLink_SDK_16 \
     cargo run -p eiviz-desktop --features decklink
   ```

The feature has no simulator, generated test source, OMT/NDI substitution, or
other backend fallback. A missing header/dispatch source is a build error; a
missing Desktop Video driver or card is an explicit runtime diagnostic.

## Required equipment

- Two direction-capable DeckLink endpoints, or one full-duplex card configured
  by Desktop Video Setup plus an independent SDI analyzer/generator.
- 3G-SDI cabling and a reference black-burst/tri-level-sync generator.
- Independent waveform/audio/cadence analyzer.
- 1080p59.94 generator with RP 188 timecode, known color bars, and 48 kHz PCM.
- A second capture/monitor path for scheduled output.

Record card model, persistent hardware ID, Desktop Video version, SDK version,
firmware, connector/profile configuration, reference format, OS, and commit.

## Acceptance scenarios

| ID | Scenario | Pass evidence |
|---|---|---|
| DL-HIL-01 | Enumeration/binding | Display name and persistent ID are stable across restart; removing a remembered card fails instead of selecting a same-named card |
| DL-HIL-02 | 1080p59.94 capture | Exact dimensions/cadence, BGRA-to-RGBA channel order, no-input indication, and monotonic SDK stream times |
| DL-HIL-03 | 48 kHz capture audio | Channel order, levels, absolute sample index, and A/V phase match the generator |
| DL-HIL-04 | Program routing | Captured input reaches Preview/Program; TAKE and follow-audio switch at the expected frame/sample boundary |
| DL-HIL-05 | Scheduled playback | Analyzer receives continuous 1080p59.94 BGRA video and 48 kHz PCM with exact 1001/60000 scheduling |
| DL-HIL-06 | Genlock | With reference connected, diagnostics report locked and analyzer shows stable phase; disconnect reports unlocked without blocking Engine |
| DL-HIL-07 | Queue pressure | Capture remains latest-wins, output queue remains bounded, counters increase, and unrelated Outputs/Program continue |
| DL-HIL-08 | Signal/driver loss | Explicit degraded/failed diagnostics, no fallback frames from this adapter, no deadlock, and controlled recovery after reconnect |
| DL-HIL-09 | 24-hour duplex soak | No unbounded memory growth; no timestamp drift; late/drop/flushed and audio discontinuity counts meet the recorded gate |

## Automated evidence and limits

Default SDK-free tests cover:

- C ABI structure sizes/alignment and shim ABI version contract;
- exact million-frame 59.94 video timestamp conversion;
- absolute 48 kHz audio sample/timestamp conversion;
- bounded latest-wins capture queue behavior;
- persistent-ID binding safety and ambiguous logical-name rejection;
- Engine routing of each Output from its owning Mixing Unit.

The native feature must additionally be compiled on each shipping platform
against SDK 16. Interop, SDI electrical behavior, completion timing, reference
lock, and soak scenarios remain pending until every `DL-HIL-*` row has attached
hardware evidence.
