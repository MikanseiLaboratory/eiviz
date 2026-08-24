# ADR-0008 Audio follow and matrix

- Status: Accepted
- Date: 2026-08-23

## Context

vMix の Audio Follow と Bus/Matrix を輸入する。手動 mute と follow の競合規則が無いと非決定的になる。

## Decision

各 Input の route mode は `Manual` または `Follow { unit }`。優先順位は manual mute > solo 集合 > follow/manual gain。Follow はlatch済みimmutable AudioPlanと対象 Mixing Unit のProgramにInputが可視ならbusへ送る。TAKEは目標ProgramをProject snapshotへ原子的に確定し、映像transitionの進捗だけをRuntime内部で保持するため、Follow切替は映像TAKEと同じsnapshot generation / logical boundaryで確定する。

## Consequences

Audio 試験は impulse / gain / mute / follow を virtual clock で再生できる。
