# ADR-0006 Encode and transport

- Status: Accepted
- Date: 2026-08-23

## Context

RTMP / SRT / MP4 が必要だが codec 特許と LGPL/GPL の組み合わせがある。FFmpeg の `--enable-nonfree` は再配布不能。

## Decision

内部契約は `EncodedAccessUnit` と `TransportSink`。既定実装はソフトウェア経路と最小 FLV/fMP4 muxer。GStreamer は optional。FFmpeg は診断用に隔離し、core に型を漏らさない。1 つの encode 結果を複数 sink へ fan-out し、sink 同士は backpressure を共有しない。

## Consequences

H.264/AAC 特許は配布前に法務確認が必要。CI は mux 構造と PTS 単調性を検証する。
