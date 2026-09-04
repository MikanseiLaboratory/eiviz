---
title: "Compatibility APIs (vMix HTTP & TCP / OBS WebSocket)"
description: vMix HTTP/TCP and OBS WebSocket compatible APIs
---

eiviz exposes APIs compatible with some existing software vision mixers, so assets built for those desks can keep working.

:::caution
These compatibility APIs do not guarantee behaviour against the original APIs. Some data is incomplete or not a full match.
:::

## vMix API (HTTP)

The mixer hosts an HTTP server in-process. The default is all interfaces on port `8088`. Settings → Web API controls enable, port, and BasicAuth. Empty username and password means no auth.

| Item | Value |
| --- | --- |
| Endpoint | `GET /api` or `GET /API` |
| State | No query. Returns vMix-shaped XML as `application/xml` |
| Function | Query such as `?Function=Fade&Duration=500`. Success returns the same XML |

Scenes come first, then Inputs, as a flat Inputs list. A Scene is a Blank input plus layers. `preview` / `active` are the selected Mixing Unit’s flat numbers. Extra Mixing Units appear as `<mix>`.

Supported Functions are in the [Function Reference](/eiviz/en/developers/function-reference/). Unknown Functions return 404; bad arguments return 400. Unlike vMix, a known Function that fails is not reported as success.

HTTP access is written to `eiviz-mixer-http.log`, separate from the mixer log. Bare polling `GET /api` is not logged.

## vMix API (TCP)

Not implemented. TCP for embedded hosts is tracked in [#107](https://github.com/MikanseiLaboratory/eiviz/issues/107).

## OBS WebSocket API

Not implemented. See [#79](https://github.com/MikanseiLaboratory/eiviz/issues/79).
