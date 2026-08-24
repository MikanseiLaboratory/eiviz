# 要件定義

Issue #1 を測定可能な要件へ分解した文書です。ID は実装・試験・HIL・release evidence の結合キーです。

## 認定ベースライン

- 映像: 1920×1080p, SDR BT.709, 8-bit, `60000/1001` fps
- 音声: 48 kHz
- OS: Windows x64 必須。Linux x64 / macOS arm64 は capability profile
- 「無限」: 製品固定上限なし。実行時は admission control により拒否できる
- 「厳密 fps」: (1) 有理数 PTS に累積丸め誤差がない (2) 認定環境で deadline/drop/xrun 基準を満たす

## 機能要件

### R01 Project
- R01.1 新規作成、保存、読込ができる
- R01.2 schema version を保持し、旧版を migration できる。未知の新版は破壊せず拒否する
- R01.3 保存は temp → fsync → rename の原子的更新とする
- R01.4 欠落 asset / 欠落 device は診断として表示し、暗黙置換しない
- R01.5 通常プロジェクトは manifest + 外部参照。portable `.eiviz` は content-addressed asset を同梱する
- R01.6 OS handle、FFI pointer、GPU resource、秘密情報は永続化しない
- R01.7 ハードウェアは論理 DeviceBinding として保存し、別 PC で再割当できる

### R02 Input / Scene
- R02.1 Input はプロジェクト全体でフラット。種別は Color、Image、Video、NDI、OMT、DeckLink、AudioDevice、MixingUnitFeed
- R02.2 tag / group を付与できる
- R02.3 Scene は Input を所有せず参照する
- R02.4 SceneItem は position / size / crop / rotation / opacity / z-order / playback を持つ
- R02.5 マウス編集は domain Command のみを発行する
- R02.6 H.264 MP4 Video は明示されたCisco OpenH264 2.6.0 binaryのみでdecodeする。`InputSource::Video.playback`のplay/pause/forward seek/loop/in/out/正速度を適用し、欠落時に別decoderやfake frameへ切り替えない

### R03 Mixing
- R03.1 Mixing Unit を製品固定上限なしで追加・削除できる
- R03.2 各 Unit は Preview / Program / 任意の transition を持つ
- R03.3 TAKE は 1 つの logical frame boundary で原子的に確定する
- R03.4 Unit 出力を別 Unit の Input にできる。接続は DAG。閉路は拒否
- R03.5 Overlay は順序付き slot。実体は Scene 参照とし SceneItem 合成を再利用する
- R03.6 Unit ごとに複数 Output、複数 Multiview を 1:N で所有できる
- R03.7 Multiview タイルは Input / Preview / Program を表示できる

### R04 Timing
- R04.1 フレームレートは有理数。59.94 は常に `60000/1001`
- R04.2 `pts(n) = n * 1001 / 60000` 秒。丸めたフレーム時間の加算を禁止
- R04.3 48 kHz 境界は `floor(n * 48000 * 1001 / 60000)`
- R04.4 SourceMedia（file/NDI/OMT）、DeckLink stream/genlock、PTP、audio
  sample clock、OS monotonic は別 domain とし、整数 affine mapper と
  Timing Island で関連付ける。mapper は drift/offset bound、lock state、
  discontinuity/reset、counter wrap、診断を持つ
- R04.5 UTC は timing domain に含めず deadline、offset、drift の計算に使わない
- R04.6 source ごとに clock policy と unlocked 時の `Fail` / `HoldLast` を
  明示し、timestamp 欠落や未 lock を同一 clock と暗黙解釈しない

### R05 GPU / Media
- R05.1 物理 GPU あたり wgpu Device/Queue の単一所有者
- R05.2 live thread は検証済み immutable RenderPlan のみ読む
- R05.3 Program を最優先。Preview/Multiview の負荷制御は Project に
  `Disabled` または deadline/GPU threshold、hysteresis、cadence divisor、
  resolution tier を明示して admission する。Program dependency の Preview は
  auxiliary 扱いせず full profile で描画する。全 tier 消費後も過負荷なら
  degraded/admission diagnostic とし、Program の format/cadence/backend を
  変更しない
- R05.4 pipeline は activation 前に prewarm する
- R05.5 device lost 時は `missing_media` に従い slate / last-good / fail を cadence どおり適用し、再生成後に frame boundary で復帰する。CPU compositor へは落とさない
- R05.6 compositor backend（`CpuReference` / `Wgpu`）は Project の明示フィールド。Runtime と不一致、または選択 backend が利用不能ならエラー。暗黙切替禁止（INV-10）

### R06 Audio
- R06.1 内部は planar f32
- R06.2 Audio Bus / Matrix。gain / pan / mute / solo / delay
- R06.3 route は `Manual` または `Follow(MixingUnit)`。manual mute が最優先
- R06.4 TAKE 時の follow 切替は映像と同じ logical boundary
- R06.5 サンプルレート差は ASRC。callback 内 allocation / blocking / I/O 禁止

### R07 I/O
- R07.1 NDI: exact `grafton-ndi` 1.0.0（`ndi` feature）でdiscovery/receive/send、RGBA映像、planar f32音声、100 ns timestamp変換、bounded worker/queueを実装する。OMT: pure-Rust `openmediatransport-rs`でdiscovery/receive/send、VMX1映像、FPA1音声、timestamp、tally/metadata、reconnectを実装する。いずれも別protocolやsimulatorへ切り替えない
- R07.2 DeckLink: SDK 16 shim（feature）。Audio: CPAL。Windows は WASAPI 必須、ASIO はライセンス後
- R07.3 SDK 未導入でも core は起動し、capability 不足を表示する
- R07.4 各 source/sink は bounded queue。遅い sink が Program を止めない

### R08 Distribution
- R08.1 baseline: RTMP = H.264/AAC in FLV、SRT = H.264/AAC in MPEG-TS、録画 = fragmented MP4
- R08.2 同一 encode 結果を fan-out できる
- R08.3 disk full / 切断時も Program を停止しない。MP4 は異常終了後に回復可能とする

### R09 Command
- R09.1 UI / keyboard / HTTP / TCP / MIDI は in-tree adapter が共通 `CommandEnvelope` を発行する。Stream Deck は **out-of-tree プラグイン** が同じ HTTP/TCP Command API を呼ぶ。本リポジトリに Deck 固有プロトコルや action map を置かない
- R09.2 単一 sequencer が id、client sequence、expected revision、effective media time を検証し全順序化する
- R09.3 「順次処理」は state mutation の確定順序のみ。I/O や GPU を直列実行しない
- R09.4 再送は command id で冪等。満杯時は Busy を返し捨てない
- R09.5 HTTP/TCP は localhost 既定。remote は認証・権限・rate limit 必須
- R09.6 HTTP/TCP/WebSocket はversioned envelopeを保持し、QueryとCommandを分離する。複数Command transactionは全件成功または全件rollbackとし、WebSocket event subscriptionをbounded queueで提供する

### R10 Operations
- R10.1 structured log、metrics、30–60 秒 flight recorder、frame id 追跡
- R10.2 deadline slack、drop/repeat、A/V drift、queue high-water、xrun、GPU pass time、device loss を計測する

### R11 Portability / Security
- R11.1 Windows x64 を必須 gate
- R11.2 非対応の optional I/O / codec feature はプロセス起動を止めず capability として表示する。選択済み compositor / encode / audio backend が利用不能なときはその Project を実行せず、他 backend へ黙って乗り換えない
- R11.3 配布物は NDI / DeckLink / ASIO / codec のライセンス表示を含む

## 不変条件

- INV-01 永続 ID は Project 内で一意
- INV-02 Scene は Input を所有しない
- INV-03 Mixing graph は DAG
- INV-04 TAKE / Overlay / Audio Follow は同一 logical frame で原子的
- INV-05 live thread は immutable snapshot のみ読む
- INV-06 callback で allocation、blocking mutex、file/network I/O、同期 log、GPU wait、panic 越境を禁止
- INV-07 全 queue は bounded。Program へ backpressure を伝播させない
- INV-08 欠落資源の暗黙置換禁止。適用するのは設定済み `Slate` / `LastGood` / `Fail` のみ
- INV-09 永続データに native handle を書かない
- INV-10 compositor / codec / device backend は明示選択。利用不能時はエラーまたは capability 不足であり、他 backend へ黙って乗り換えない

## 受入条件（baseline）

- AC-01 保存→再読込で ID / 参照 / 順序 / 再生設定が一致
- AC-02 旧 schema を migration できる。未来版は拒否
- AC-03 `60000/1001` で 1001 秒あたり 60000 個のフレーム期限。累積丸め drift 0
- AC-04 Mixing 閉路は保存・実行前に拒否
- AC-05 TAKE で映像・Overlay・Audio Follow が同一境界で切り替わる
- AC-06 1 つの Output 障害が他 Output の cadence を止めない
- AC-07 複数操作元の Command に全順序が付き、replay で同じ state hash になる
- AC-08 無効参照・能力超過・欠落機器は診断付きで拒否し、プロセスは継続
- AC-09 認定負荷 24h で内部要因の Program drop/repeat と audio xrun が 0
- AC-10 lock 後 A/V sync は暫定 P99 ±1 ms、最大 ±5 ms（実測で変更する場合は ADR）
- AC-11 実I/Oは公式reference toolまたは別PCとの相互運用証跡が必要。stub、probe、同一processのsimulated loopbackだけでは合格にしない
- AC-12 mapper の deterministic drift/jump/wrap/domain test と live adapter
  ごとの monotonic correlation、lock/reset、A/V drift HIL 証跡が一致する
