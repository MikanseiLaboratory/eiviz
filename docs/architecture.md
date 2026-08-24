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
| eiviz-control | HTTP/TCP Command API。Stream Deck 固有ロジックは置かない | Project 直接更新 |

## 時刻

内部時刻は `ticks + Rational timebase` です。59.94 の第 n フレームは `n * 1001 / 60000` 秒です。OS deadline は毎回フレーム番号から計算し、丸めた周期を加算しません。

## コマンド

すべての状態変更は `CommandEnvelope` です。sequencer だけが `Project` を mutate し、`revision` を進め、frame boundary で immutable snapshot を差し替えます。TAKE と record start/stop は coalesce しません。

## メディア

source は `MediaSource`、sink は `MediaSink` です。queue は bounded、policy は用途別です。

Native source adapter は専用受信 thread で vendor/network API を呼び、
所有権のある `VideoFrame` / `AudioBuffer` へコピーしてから bounded slot
へ公開します。Runtime tick は登録済み `MediaSource` から非blocking pull
し、`InputId` で Project の論理 Input と結びます。adapter 未登録や受信前
は Project の missing-media policy を適用し、simulator を暗黙使用しません。

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

- Program: drop しない。間に合わなければ last-good を cadence どおり繰り返す
- Preview/Multiview: latest-wins
- Recorder/Network: 独立。満杯時は自分だけ Degraded

## GPU

RenderPlan は due output から逆算した demand-driven DAG です。submit は GPU executor の単一箇所に限ります。ゼロコピーは `GpuInterop` に隔離し、既定経路は staging copy です。`Project.compositor` が `Wgpu` のとき CPU 合成へ落とさない。`CpuReference` は CI / 参照用の明示 profile です。

## 失敗ドメイン

source、Program、各 Output、audio、control、recording は独立した状態機械です。一つの sink の失敗で engine 全体を止めません。
