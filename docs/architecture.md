# アーキテクチャ

## 目的

eiviz はプロダクション用途のクロスプラットフォーム映像スイッチャーです。制御面とリアルタイム面を分離し、外部 SDK を adapter に閉じ込めます。

## 論理構造

```
Controls ──► CommandSequencer ──► Versioned Project
                                      │
                                      ▼
                              RenderPlan / AudioPlan
                                      │
Sources ──► ClockMapper ──► TimingIsland ──► Runtime
                                      │
                          ┌───────────┴───────────┐
                          ▼                       ▼
                     GPU Executor            Audio Engine
                          │                       │
                          └───────────┬───────────┘
                                      ▼
                         Independent bounded fanout
                                      │
                    Preview / NDI / OMT / DeckLink / RTMP / SRT / MP4
```

各 `Output` は `OutputVideoSource::Program` または
`OutputVideoSource::Multiview(id)` を永続化します。Engine は同じ source selector
を direct `MediaSink` と encode session key の両方へ適用するため、NDI/OMT/
DeckLink/RTMP/SRT/MP4 は OutputKind ごとに独立して Program または Multiview
全体を受け取ります。baseline 要件は Multiview 全体の出力であり、tile 単体 crop
は要求していません。

## クレート境界

| crate | 責任 | 禁則 |
| --- | --- | --- |
| eiviz-time | 有理数時刻、clock domain | I/O、GPU |
| eiviz-core | ドメインモデルと不変条件 | wgpu、SDK、GUI |
| eiviz-command | envelope、reducer、冪等、revision | リアルタイム I/O |
| eiviz-project | schema、migration、ZIP | 再生 |
| eiviz-media | frame/audio/packet 契約 | 具体 SDK 型 |
| eiviz-runtime | グラフ検証、スケジューラ、registry | GUI |
| eiviz-gpu | 合成（明示 CpuReference / Wgpu。暗黙切替なし） | NDI/DeckLink 型 |
| eiviz-engine | 所有権の結線 | 個別 SDK 実装の詳細 |
| eiviz-io-* / codec-* | adapter | domain 型の再定義 |
| eiviz-control | HTTP/TCP/WebSocket Query・Command API、optional MIDI。Stream Deck 固有ロジックは置かない | Project 直接更新、default buildでnative MIDI依存 |

## 時刻

内部時刻は `ticks + Rational timebase` です。59.94 の第 n フレームは `n * 1001 / 60000` 秒です。OS deadline は毎回フレーム番号から計算し、丸めた周期を加算しません。

`ClockMapper` は SourceMedia（file / NDI / OMT）、DeckLinkStream、
AudioSample、PTP を process-wide `Monotonic` reference へ整数 affine
mapping します。offset/drift は有限 window の bounded regression/filter
で更新し、counter wrap は設定された modulus で unwrap します。逆行、
明示 discontinuity、閾値を超える jump は mapper を `Acquiring` へ reset
します。`TimingIsland` は reference を介して domain 間を写像し、DeckLink
genlock のような外部 lock signal も aggregate lock state に含めます。

source 登録時の clock policy は必須です。生成/still は明示
`ScheduleTime`、file は `ExactCorrelation`、live adapter は
`Bounded { unlocked: Fail | HoldLast }` を選びます。Desktop の live
adapter は `Fail` を選択し、lock 前または timestamp correlation 欠落時に
同一 clock と仮定しません。adapter は source timestamp と capture 時の
monotonic correlation を frame/audio contract へ格納します。Runtime は
logical PTS を exact mapper で monotonic deadline に変換し、video/audio
skew と A/V drift を source ごとに記録します。UTC は clock domain ではなく、
deadline や mapper へ入力できません。

## コマンド

すべての状態変更は versioned `CommandEnvelope` です。sequencer は command ID、client sequence、expected accepted revision を受付時に検証し、`effective_time`（未指定または過去なら次の logical boundary）と受付順で bounded pending queue を全順序化します。受付は即時に `revision`（accepted revision）を返しますが、active `Project` と `state_hash` は変わりません。`Engine::tick` が due batch を取り出し、検証・compile済みの `Project` / `RenderPlanSnapshot` / `AudioPlan` を Runtime 実行前に一括 latch して `applied_revision` を進めます。

同じ effective time は受付順です。異なる effective time を一つの transaction に混在させず、transaction は一つの boundary で全件または0件を反映します。新規commandを時刻順へ挿入した結果、既にpendingの後続状態がinvalidになる場合も新規command全体を拒否し、revision・idempotency・client sequence・queueをrollbackします。TAKE と record start/stop は coalesce しません。Runtime は immutable snapshotだけを読み、transitionの残りframeはRuntime内部状態で進めてProjectへ書き戻しません。Audio FollowはTAKE後のProgramを映像と同じsnapshot generationから読むため、同一boundaryで切り替わります。

`state_hash` はactive Projectだけのhashです。pendingを含む候補は `staged_state_hash` として別に診断します。pending depth/capacity、batchごとのeffective time/command ID/accepted revision、accepted/applied revision、保持中idempotency record数をstatus/metricsで公開します。

Control API v1はHTTP query/command/transaction、TCP JSON-lines、
WebSocket query/command/transaction/event subscriptionを提供します。全mutationは
bounded dispatcherを通り、遅いWebSocket subscriberはbounded queue満杯時に切断
します。loopback以外のbindには明示tokenが必須で、token設定時はhealth/queryを
含む全requestを認証します。MIDIは`midi` featureの`midir`実backendだけで、
backend stable port IDとmappingを明示選択します。feature無効時にlistener stubは
存在しません。Stream Deck固有actionはout-of-tree clientの責務です。

## メディア

source は `MediaSource`、sink は `MediaSink` です。queue は bounded、policy は用途別です。

Native source adapter は専用受信 thread で vendor/network API を呼び、
所有権のある `VideoFrame` / `AudioBuffer` へコピーしてから bounded slot
へ公開します。Runtime tick は登録済み `MediaSource` から非blocking pull
し、`InputId` で Project の論理 Input と結びます。adapter 未登録や受信前
は Project の missing-media policy を適用し、simulator を暗黙使用しません。

NDI は `ndi` feature でのみ `grafton-ndi` 1.0.0 とローカルの NDI 6
SDK/runtime を使います。映像・音声 capture は別 worker、送信は Output ごとの
worker で行い、全 queue は bounded です。受信した SDK 所有bufferは
RGBA/planar f32 の eiviz 所有bufferへコピーし、NDI の100 ns timestampを
`MediaTime`/sample indexへ整数変換します。feature未選択時にfake capability、
OMT、simulator、slateをNDI adapterとして生成する経路はありません。

file-video は `eiviz-io-file::VideoFileSource` が `shiguredo_mp4` の
Annex-B sample indexを再生し、`eiviz-codec-software` が明示されたCisco
OpenH264 2.6.0 binaryだけをdynamic loadします。seek/loop時は直前sync
sampleからdecoderをresetしてSPS/PPSを再投入します。binary pathはruntime
設定でありProjectへ保存しません。欠落・hash/version不一致時はsource構築
エラーとし、別decoder、source build、fake frameへ切り替えません。

Output adapter は `OutputId` で Engine の sink registry へ登録します。
各 tick は `Output.owner` の Mixing Unit Program だけを該当sinkへ送り、
`enabled=false` と削除済みOutputは送信しません。全sinkへprimary unitを
一律送信する経路は使用しません。

- Program: load shedding の対象にしない。Project の full format/cadence で毎 boundary 描画する
- Preview/Multiview: latest-wins。上書き drop と queue high-water を計測する
- Recorder/Network: 独立。満杯時は自分だけ Degraded

## GPU

RenderPlan は due output から逆算した demand-driven DAG です。submit は GPU executor の単一箇所に限ります。ゼロコピーは `GpuInterop` に隔離し、既定経路は staging copy です。`Project.compositor` が `Wgpu` のとき CPU 合成へ落とさない。`CpuReference` は CI / 参照用の明示 profile です。

`Project.auxiliary_load_shedding` は既定 `Disabled` です。有効時は前 frame の
deadline slack と GPU frame time を immutable `RuntimeSnapshot` の threshold
へ入力し、連続 frame 数の hysteresis で ordered quality tier を一段ずつ
escalate/recover します。tier は Preview/Multiview ごとの cadence divisor と
resolution divisor だけを変更します。Program から参照される Preview mixfeed は
依存 closure を取って critical path に残すため、auxiliary policy が Program の
pixel、format、PTS、cadence を変える経路はありません。最後の tier でも過負荷が
継続すれば state は `Exhausted` となり、Program を続行したまま明示的な
degraded/admission diagnostic を出します。

`RuntimeSnapshot` は active pointer を交換する前に、全 Program/Preview/
Multiview quality tier の pipeline と source/output/readback resource requirement
を Wgpu compositor へ渡します。resource pool は purpose・working format・width・
height を key にし、resident byte/resource 数を hard limit します。idle resource
は返却 sequence、次に key の順で決定論的に eviction します。render/readback は
prewarm 済み lease の取得だけを許し、未準備 key は frame 中に生成せず hard error
にします。command encoder/command buffer は submission 用の一時 command object
であり、resident texture/buffer/bind-group/pipeline allocation には数えません。

GUI では eframe が Device/Queue owner です。device-lost callback を compositor
diagnostic に latch し、Runtime と Engine は `Degraded` へ遷移して Wgpu frame を
停止します。新しい GUI-owned compositor は Engine に queue され、次の media
boundary で active snapshot を再 prewarm できた場合だけ swap します。
`eframe 0.32` は実行中の `RenderState` 再生成 API を公開しないため、Desktop は
明示 `restart-required` と終了操作を表示します。いずれの経路も
`Project.compositor` を `CpuReference` へ変更しません。

## 失敗ドメイン

source、Program、各 Output、audio、control、recording は独立した状態機械です。一つの sink の失敗で engine 全体を止めません。Wgpu device loss は Program compositor 自体の failure domain なので Engine 全体の GPU media boundary を明示 Degraded にしますが、control/persistence は利用可能なままにし、backend を暗黙変更しません。
