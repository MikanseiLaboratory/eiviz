# ADR-0003 Rational time and clocks

- Status: Accepted
- Date: 2026-08-23

## Context

59.94 fps を `f64` で持つと長時間ドリフトする。NDI の合成 timecode も 59.94 で誤差が報告されている。複数デバイスは共通 genlock を持つとは限らない。

## Decision

- 時刻は整数 ticks + 既約有理数 timebase。
- 59.94 は `60000/1001` のみ。
- PTS はフレーム番号から都度計算する。
- clock domain（monotonic / device / audio / media / UTC）を混在させない。
- genlock のない Timing Island 間の同時 TAKE は「各 island の次 boundary で原子的」と定義し、実測 skew を返す。

## Consequences

drop-frame timecode は表示ラベルであり実行クロックではない。試験は virtual clock で 1001 秒 = 60000 フレームを検証する。
