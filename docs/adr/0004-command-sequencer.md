# ADR-0004 Command sequencer

- Status: Accepted
- Date: 2026-08-23

## Context

Issue は TCP/HTTP/MIDI/Stream Deck/keyboard/UI を Command Queue で順次処理すると書く。Stream Deck の action 解釈は **プラグイン側** の責務であり、本リポジトリは Command API だけを提供する。I/O 完了待ちまで直列化するとフレームが止まる。

## Decision

順次化対象は検証済み状態遷移だけとする。各 envelope は `command_id`、`client_seq`、`expected_revision`、`effective_time` を持つ。適用は frame/audio buffer 境界の atomic snapshot swap。再送は id で冪等。queue 満杯は Busy。

## Consequences

外部 API は Query と Command を分ける。リアルタイムスレッドは sequencer を直接呼ばない。Stream Deck 用の別プロトコルや action map は追加しない。
