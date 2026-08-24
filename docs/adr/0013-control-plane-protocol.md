# ADR-0013 Authenticated control-plane protocol

- Status: Accepted
- Date: 2026-08-24

## Context

The localhost HTTP/TCP prototype accepted payload-only commands, authenticated
only HTTP mutations, had no event subscription, and exposed a no-op MIDI
function. It did not preserve the command ID, client sequence, or expected
revision supplied by a remote client.

## Decision

Control API version 1 uses `CommandEnvelope` version 1 without reconstructing
it at the server:

- HTTP provides authenticated query, command, and atomic transaction routes.
- TCP uses bounded JSON-lines `ApiRequest` frames with a token on every
  request.
- WebSocket authenticates the upgrade, then provides query, command,
  transaction, and revision-event subscription operations.
- All mutation transports feed one bounded dispatcher. WebSocket subscriber
  queues are bounded; a consumer that cannot keep up is disconnected.
- Loopback is the default. Any non-loopback HTTP, TCP, or WebSocket bind is
  rejected unless an explicit token is configured. A configured token protects
  queries and health/status as well as commands.
- MIDI is an opt-in `midi` feature using `midir`. The user selects a
  backend-stable input-port ID and supplies explicit channel-message mappings.
  The native callback only copies at most three bytes into a bounded queue; a
  worker emits versioned envelopes. Feature-disabled builds expose capability
  status but no listener function.
- Multi-command transactions clone and validate project/sequencer state before
  committing. Any failure leaves project, revision, idempotency records, and
  client sequence state unchanged.

Stream Deck interpretation remains out of tree. Such integrations are ordinary
clients of the HTTP, TCP, or WebSocket API and receive no in-tree action map or
privileged transport.

## Consequences

The direct services are plaintext. Non-loopback deployments must place them on
a trusted management network or behind a TLS-terminating authenticated proxy;
the bearer token is not a substitute for transport encryption. Optional
`midir` platform dependencies remain outside the default portable build and
CI profile.
