# Control API v1

All state changes carry a complete `CommandEnvelope`; servers never replace a
client's ID, sequence, expected revision, or effective time. API and envelope
versions are currently `1`.

## HTTP

Authenticated query routes:

- `GET /v1/status`
- `GET /v1/project`
- `GET /v1/metrics`
- `GET /v1/events`

Submit one envelope:

```http
POST /v1/command
Authorization: Bearer TOKEN
Content-Type: application/json

{"version":1,"id":"COMMAND_UUID","client":"CLIENT_UUID","client_seq":1,
 "expected_revision":12,"effective_time":null,"coalesce_key":null,
 "payload":{"SetName":{"name":"Studio A"}}}
```

Submit an atomic batch:

```json
{
  "version": 1,
  "envelopes": [
    {
      "version": 1,
      "id": "COMMAND_UUID",
      "client": "CLIENT_UUID",
      "client_seq": 2,
      "expected_revision": 13,
      "effective_time": null,
      "coalesce_key": null,
      "payload": {"SetName": {"name": "Studio B"}}
    }
  ]
}
```

POST that object to `/v1/transaction`. An invalid command rolls back the whole
batch, including revision, client-sequence, and idempotency records. Replaying a
committed command ID returns `duplicate=true` and its original revision.

## TCP JSON-lines

Send one `ApiRequest` per newline. The maximum frame is 1 MiB. A configured
token is included on every request:

```json
{"version":1,"request_id":"q-1","token":"TOKEN","type":"query","query":"status"}
{"version":1,"request_id":"c-1","token":"TOKEN","type":"command","envelope":{"version":1,"id":"COMMAND_UUID","client":"CLIENT_UUID","client_seq":1,"expected_revision":null,"effective_time":null,"coalesce_key":null,"payload":"Noop"}}
```

Responses are newline-delimited and contain the request ID, current revision,
state hash, and exactly one of `result` or `error`.

## WebSocket

Connect to the configured WebSocket port. Authenticate the upgrade using
`Authorization: Bearer TOKEN`. Browser clients that cannot set that header may
offer `eiviz.bearer.TOKEN` in `Sec-WebSocket-Protocol`; the server selects it.

After upgrade, send the same `ApiRequest` objects as TCP without the `token`.
WebSocket additionally accepts:

```json
{"version":1,"request_id":"s-1","type":"subscribe"}
```

Successful subscription emits:

```json
{"version":1,"event":"revision","revision":14,"state_hash":"...","command_ids":["..."]}
```

Event queues are bounded. A subscriber that fills its queue is disconnected
and must query current state before resubscribing; events are not silently
dropped.

## Errors and security

Revision/client-sequence conflicts use HTTP 409. Queue saturation/unavailability
uses HTTP 503 with error code `busy`/`unavailable`; admission failures use 422.
Rate excess uses HTTP 429 or a transport error response.

The services do not provide TLS. Non-loopback binds require a token and should
still be isolated on a management network or placed behind a TLS reverse proxy.
Do not put tokens in project files or URL query strings.
