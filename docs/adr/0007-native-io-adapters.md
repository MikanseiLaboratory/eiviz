# ADR-0007 Native I/O adapters

- Status: Accepted
- Date: 2026-08-23

## Context

NDI / OMT / DeckLink / ASIO は FFI、EULA、再配布制約が異なる。公開 `decklink` crate は Windows-first 製品の基盤に足りない。

## Decision

各 SDK は独立 crate + cargo feature。未リンク時は CapabilityProbe が Unavailable を返す。OS の `cfg` だけで available とせず、CPAL host 初期化と実 device enumeration が成功した場合だけ audio capability を available とする。DeckLink は公式 SDK への薄い shim を後で足せる interface を先に固定する。ASIO と WASAPI は **別の明示 backend** であり、ASIO が要求されているときに WASAPI へ暗黙切替しない。callback では lock-free bounded ring への copy のみを行い、allocation、lock、blocking を禁止する。

## Consequences

core CI は SDK なしで通る。HIL は private runner。
