# ADR-0001 Workspace and layering

- Status: Accepted
- Date: 2026-08-23

## Context

Issue #1 は GUI、リアルタイム合成、多数の vendor SDK、永続化を同時に要求する。単一 crate に置くと SDK ライセンスと wgpu 版更新が全体を破壊する。

## Decision

Cargo workspace を `core/time/command/project/media/runtime/gpu/engine` と `io-*` / `codec-*` / `control` / `desktop` に分割する。依存は adapter → engine → core の一方向とする。

## Consequences

機能追加は adapter または command の拡張が基本になる。crate 数は増えるが、SDK 欠如環境でも core test が回る。
