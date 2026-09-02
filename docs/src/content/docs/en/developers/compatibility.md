---
title: "Compatibility APIs (vMix HTTP & TCP / OBS WebSocket)"
description: vMix HTTP/TCP and OBS WebSocket compatible APIs
---

eiviz exposes APIs compatible with some existing software vision mixers, so assets built for those desks can keep working.

:::caution
This feature is still in progress.
:::

## vMix API (HTTP/TCP)

### HTTP API

An HTTP API that returns vMix-compatible XML.  
Shortcut Functions are supported; the main vMix shortcuts work.

XML is flattened to the vMix shape, so Inputs and Scenes are all treated as Inputs. Scenes are mixed into the XML as a Blank input plus layers.

### TCP API

A vMix TCP API compatible endpoint.  
FUNCTION and ACTS send and receive work. eiviz event updates and hooks go out in the vMix TCP API shape.  
Aimed at embedded or low-memory hosts.

## OBS WebSocket API

An OBS Studio WebSocket API compatible endpoint.

It uses the same auth as OBS WebSocket, plus the main event data, and can control eiviz.

:::caution
These compatibility APIs do not guarantee behaviour against the original APIs. Some data is incomplete or not a full match.
:::
