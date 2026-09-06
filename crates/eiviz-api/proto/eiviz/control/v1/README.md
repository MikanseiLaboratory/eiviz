# eiviz.control.v1

`package eiviz.control.v1` is the published native control contract.

## Compatibility

- Field numbers that have shipped must never change meaning or be reused.
- When a field is removed, add both the number and the name to `reserved`.
- Clients and servers are not updated in lockstep. Unknown protobuf fields must be ignored.
- Wire transports share the same Envelope meaning. WebSocket and TCP must not diverge Command semantics.

## Transports

- WebSocket: one binary frame is one Envelope. Subprotocol `eiviz.protobuf.v1`. Non-binary frames are rejected.
- TCP (optional, default off): `EIVZ` + protocol version byte + 4-byte big-endian length + Envelope. Challenge/response auth is required; tokens are not sent in the clear.
