# ADR-0008 Audio follow and matrix

- Status: Accepted
- Date: 2026-08-23

## Context

vMix の Audio Follow と Bus/Matrix を輸入する。手動 mute と follow の競合規則が無いと非決定的になる。

## Decision

各 Input の route mode は `Manual` または `Follow { unit }`。優先順位は manual mute > solo 集合 > follow/manual gain。Follow は対象 Mixing Unit の Program（transition 中は Program と Preview の mix 係数）に Input が可視なら bus へ送る。切替は映像 TAKE と同じ snapshot generation。

## Consequences

Audio 試験は impulse / gain / mute / follow を virtual clock で再生できる。
