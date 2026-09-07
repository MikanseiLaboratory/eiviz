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

When Web API is enabled, the mixer also opens a [vMix TCP API](https://www.vmix.com/help29/TCPAPI.html) text server on port `8099`. Panels such as Iryx that speak [vmix-rs](https://github.com/MikanseiLaboratory/vmix-rs) use this surface. HTTP BasicAuth does not apply to TCP. A TCP bind failure is a warning; HTTP keeps running.

UTF-8, `\r\n` terminated. Replies are `<command> <status> ...\r\n`. `XML` then writes a body of exactly that length. Clients wait for a reply before the next command. Subscribed events may arrive at any time.

| Command | Behaviour |
| --- | --- |
| `TALLY` | `TALLY OK 0121...`. One digit per flat Input. 0 off, 1 Program, 2 Preview. Program wins |
| `FUNCTION` | Same Shortcuts as HTTP, e.g. `FUNCTION Fade Duration=500`. Success is `FUNCTION OK Completed` |
| `ACTS` | `Input` / `InputPreview` / `InputMix2`… / `Overlay1`–`8`. No InputNumber means the current assignment. Unsupported activators return 0 |
| `XML` / `XMLTEXT` | Same XML as HTTP. XPath is a subset (`vmix/version`, `preview`, `active`, `vmix/inputs/input[N]/@title`, …) |
| `SUBSCRIBE` / `UNSUBSCRIBE` | Push `TALLY` and `ACTS` changes. Iryx uses this |
| `VERSION` / `QUIT` | Version string and disconnect |

Functions are listed in the [Function Reference](/eiviz/en/developers/function-reference/).

## OBS WebSocket API

Not implemented. See [#79](https://github.com/MikanseiLaboratory/eiviz/issues/79).
