# Control plane and MIDI HIL

This procedure records local protocol integration and real MIDI hardware
evidence. A simulated MIDI loopback alone does not satisfy AC-11.

## Portable local integration

1. Run `cargo test -p eiviz-control`.
2. Start Desktop with a unique token:

   ```bash
   EIVIZ_CONTROL_TOKEN="$(openssl rand -hex 32)" cargo run -p eiviz-desktop
   ```

3. Verify the Capabilities panel reports HTTP, TCP, and WebSocket ports,
   authentication required, the configured rate, and bounded command capacity.
4. With the token, query `/v1/status`, submit one envelope twice to
   `/v1/command`, and confirm the second acknowledgement has
   `duplicate=true` without increasing revision.
5. Submit a transaction whose second command is invalid. Confirm HTTP 400,
   unchanged revision, and unchanged state hash.
6. Subscribe over WebSocket, submit a valid command, and record the correlated
   response followed by a `revision` event.
7. Repeat each query/command without credentials and record HTTP 401,
   WebSocket upgrade rejection, and a TCP `unauthorized` response.
8. Exceed the configured request rate and command/event queue capacities.
   Record 429/rate errors, `busy` responses, and slow WebSocket disconnection.

Do not expose the plaintext listeners to an untrusted network. For remote HIL,
use an isolated management VLAN or a TLS reverse proxy and verify the direct
ports are firewalled.

## Physical MIDI input

Platform prerequisites:

- Windows: WinMM (provided by Windows)
- macOS: CoreMIDI (provided by macOS)
- Linux: ALSA development/runtime packages and an ALSA MIDI device

Procedure:

1. Connect a physical USB or DIN MIDI controller and record manufacturer,
   model, firmware, OS, and backend.
2. Run `cargo run -p eiviz-desktop --features midi`.
3. In **MIDI Control**, refresh ports and select the intended port by its
   displayed opaque backend ID. Confirm no port is selected automatically.
4. Configure channel and note, start the TAKE mapping, and press the physical
   control 100 times. Record received/matched/overflow/submit-error counters
   and verify exactly 100 TAKE revisions.
5. Send neighboring notes, a different channel, Note Off, Clock, Active
   Sensing, and SysEx. Confirm none invoke TAKE.
6. Disconnect the selected device. Confirm the process remains alive and does
   not attach another device. Reconnect and explicitly select/start it again.
7. Flood above operator rate while blocking control consumers. Confirm MIDI
   queue overflow is counted, memory remains bounded, and Program cadence is
   unaffected.

Archive logs, screenshots, controller configuration, state hashes, and the
exact commit SHA. Passing unit tests or a virtual port is development evidence,
not hardware certification.
