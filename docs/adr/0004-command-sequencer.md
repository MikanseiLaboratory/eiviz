# ADR-0004 Command sequencer

- Status: Accepted
- Date: 2026-08-23

## Context

Issue は TCP/HTTP/MIDI/Stream Deck/keyboard/UI を Command Queue で順次処理すると書く。Stream Deck の action 解釈は **プラグイン側** の責務であり、本リポジトリは Command API だけを提供する。I/O 完了待ちまで直列化するとフレームが止まる。

## Decision

順次化対象は検証済み状態遷移だけとする。各 envelope は `command_id`、`client_seq`、`expected_revision`、`effective_time` を持つ。受付時にaccepted revisionを割り当てるがactive stateは変更せず、effective media timeと受付順でbounded queueへstageする。同一transactionのeffective timeは一つに限定する。

`Engine::tick` はboundary時刻以下のbatchを全て取り出し、検証・compile済みのimmutable `Project` / `RenderPlanSnapshot` / `AudioPlan` をRuntimeより先にatomic latchしてapplied revisionを進める。active `state_hash` はpendingを含めず、候補hashは別診断とする。再送は保持中idempotency IDで冪等、外部clientの保持期間外replayはclient sequenceでstale拒否する。pending queue満杯は Busy。

## Consequences

外部 API は accepted と applied を区別し、Query と Command を分ける。即時successは受付完了でありmedia適用完了ではない。リアルタイムスレッドは sequencer を直接呼ばない。transition progressはRuntime内部状態でありProjectをmutateしない。Stream Deck 用の別プロトコルや action map は追加しない。
