# 認定と試験

## プロファイル

| ID | 内容 | 状態 |
| --- | --- | --- |
| PROFILE-1080P5994 | 1920×1080p SDR BT.709 8-bit、60000/1001、48 kHz | baseline。必須 |
| PROFILE-2160P5994 | 3840×2160p59.94 SDR 8-bit | implemented, HIL pending; not certified |
| PROFILE-HDR10 | 3840×2160p59.94 BT.2020 PQ / HLG、10-bit | implemented with explicit WGSL policy, HIL pending; not certified |
| PROFILE-INTERLACE | 1080i59.94 explicit field cadence/order | timing contract implemented, adapter HIL pending; not certified |

未認定 profile のコード経路があっても、release では capability 表示に留めます。
Extended-profile implementation does not change the required
`PROFILE-1080P5994` gates or make hardware certification claims. See
[the Phase 9 HIL matrix](hil/video-profiles.md).

## 暫定性能予算（1080p59.94）

フレーム周期は正確に `1001/60000` 秒（約 16.683 ms）。

| 項目 | P99 目標 |
| --- | --- |
| command latch | ≤ 0.25 ms |
| source select + CPU prep | ≤ 0.75 ms |
| GPU critical path | ≤ 6 ms |
| egress | ≤ 2.5 ms |
| safety margin | ≥ 2 ms |
| audio callback @ 128 samples / 48 kHz | ≤ 0.53 ms |
| lock 後 A/V | ±1 ms P99、最大 ±5 ms |

実機で未達なら黙って緩和せず ADR で予算を改訂します。

## 試験レイヤ

1. Unit / property: 有理数、reducer、DAG、audio routing、migration
2. Virtual-clock integration: TAKE、overlay、follow、queue overflow、replay
3. GPU: `Wgpu` profile は hardware adapter 必須。無ければその profile は試験対象外（CPU に置換しない）。CI は明示 `CpuReference` profile
4. HIL: DeckLink / ASIO / NDI / OMT は private runner。Stream Deck は out-of-tree プラグインが HTTP/TCP Command API を叩く試験とする
5. Soak: 24 h gate、release 72 h
6. Fault: GPU lost、disk full、NIC down、SDK 切断

Clock/Timing の共通 HIL は [hil/timing.md](hil/timing.md) に定義します。
unit test の affine/drift/jump/wrap/domain 結果だけでは、実 adapter の
monotonic correlation、genlock、A/V skew gate を合格にしません。

## CI

- PR: fmt、clippy `-D warnings`、workspace test（default features）
- Nightly: Windows / Linux / macOS
- HIL: self-hosted。SDK 再配布を CI に置かない

## トレーサビリティ

各要件 ID に test 名、HIL scenario、OS、実測、既知制限を紐付けます。Phase 完了は証跡が揃った時点です。

OMT の具体的な未実施シナリオは [hil/omt.md](hil/omt.md) を参照してください。
`OMT-HIL-01..10` が未実施のため、OMT および Phase 5 は未完了です。

GPU の未実施シナリオは [hil/gpu.md](hil/gpu.md) を参照してください。
ADR-0011 の desktop 単一Device/Queue統合は実装済みです。ただし
`GPU-HIL-01..08`（特に device recreation、admission-controlled overload、
soak）が未実施のため、Phase 2 は未完了です。R05.3 の synthetic timing test は
state machine と Program invariant の automated evidence ですが、実 GPU の
deadline/GPU timing HIL を代替しません。

`TIME-HIL-01..08` は実機未実施です。Desktop に lock、rate ppb、
offset/residual、reset/wrap、video/audio skew、A/V drift の metrics は
実装済みですが、AC-10 の合格証跡ではありません。
