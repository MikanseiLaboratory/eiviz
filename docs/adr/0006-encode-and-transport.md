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
最初の software encode profile は、明示 path の Cisco OpenH264 2.6.0 binary
（`openh264-sys2` allow-list SHA-256 + runtime version 検証）と、明示 path の
license-reviewed FDK AAC binary（AAC-LC/raw transport C ABI）を dynamic load
する。どちらも同梱/source build せず、欠落時は hard error とする。in-tree
I_PCM は `cfg(test)` のみで、PCM/test bytes/別 codec への fallback はしない。
FDK upstream license は patent grant を含まないため、binary provenance、
license、patent、配布形態を distribution ごとに別途法務確認する。

## Consequences

H.264/AAC 特許は配布前に法務確認が必要。CI は共有 fan-out、queue/reconnect、
RTMP/SRT local protocol peer、mux 構造、fMP4 tail recovery を検証する。
実 server、loss、24h は `docs/hil/distribution.md` の証跡が必要である。
