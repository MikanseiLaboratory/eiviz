# ADR-0006 Encode and transport

- Status: Accepted
- Date: 2026-08-23

## Context

RTMP / SRT / MP4 が必要だが codec 特許と LGPL/GPL の組み合わせがある。FFmpeg の `--enable-nonfree` は再配布不能。

## Decision

内部契約は共有 immutable `EncodedAccessUnit` と `EncodedSink`。同じ `Arc`
を各 sink の独立 bounded queue へ fan-out し、queue full / disconnect 後は
その sink だけ次の IDR まで捨てて再接続する。RTMP は `rml_rtmp` 0.8.0
（MIT）、SRT は `srt-tokio` 0.4.4（Apache-2.0）の pure-Rust caller、
container は in-tree FLV / MPEG-TS / fragmented MP4 とする。

Project は H.264 encoder、AAC encoder、transport、queue、reconnect を明示する。
in-tree I_PCM は test-only で、AAC encoder は未実装であるため、現在の製品
profile activation は hard error になる。PCM や別 codec への fallback はしない。
GStreamer / FDK AAC は Rust 1.97 では build 可能な候補だが、system/plugin
license、特許、配布形態の審査前には選択しない。

## Consequences

H.264/AAC 特許は配布前に法務確認が必要。CI は共有 fan-out、queue/reconnect、
RTMP/SRT local protocol peer、mux 構造、fMP4 tail recovery を検証する。
実 server、loss、24h は `docs/hil/distribution.md` の証跡が必要である。
