# ADR-0007 Native I/O adapters

- Status: Accepted
- Date: 2026-08-23

## Context

NDI / OMT / DeckLink / ASIO は FFI、EULA、再配布制約が異なる。公開 `decklink` crate は Windows-first 製品の基盤に足りない。

## Decision

各 SDK は独立 crate + cargo feature。未リンク時は CapabilityProbe が Unavailable を返す。DeckLink は公式 SDK への薄い shim を後で足せる interface を先に固定する。ASIO はライセンス未整備なら WASAPI に落とす。callback では copy または bounded enqueue のみ。

## Consequences

core CI は SDK なしで通る。HIL は private runner。
