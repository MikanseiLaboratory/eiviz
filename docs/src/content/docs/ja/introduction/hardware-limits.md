---
title: ハードウェア固有の制約
description: GPUエンコーダー、デコード資源、メモリ帯域の同時利用に関する制約
---

eivizはGPUのエンコード・デコード資源を直接使うため、同時に使える本数や帯域は**ソフトウェアの設定ではなくハードウェアとドライバーが決めます**。ここではOS・GPUごとに分かっている制約と、eiviz上での現れ方をまとめます。

:::note
同時セッション数や帯域の上限を**問い合わせる公開APIは、VideoToolbox・NVENC・Media Foundationのいずれにもありません**。取得できるのは「このビルドが開始できるエンコーダー種別」（`mixer_capabilities`の`stream_encoder_mask`）と、Media Foundationのデバイス一覧だけです。上限に達したときは開始時にドライバーが拒否し、エラーとしてログと配信ストリップに出ます。
:::

## macOS: VideoToolbox

- **同時セッション数の上限は公開されていません。** `kVTCompressionPropertyKey_RecommendedParallelizationLimit` は「1本のエンコードを並列分割するときの推奨数」であり、同時セッション数ではありません（[Apple Developer](https://developer.apple.com/documentation/videotoolbox/kvtcompressionpropertykey_recommendedparallelizationlimit)）。
- **デコードとエンコードが同じ資源を共有します。** eivizは映像ファイルとカメラ入力をAVFoundation（内部でVideoToolbox）でデコードし、配信はVideoToolboxでエンコードします。同時に動かすとエンコーダー側のコールバックが返らず、`VideoToolbox is not responding; dropping frames for …` が続いて**0 fpsのまま配信が始まらない**ことがあります。
- 回避策は次のいずれかです。
  - 配信のエンコード設定を**OpenH264**にする（CPUエンコード。動画・カメラ入力を止めずに配信できる）。OpenH264は初回利用時に取得されます。
  - 問題になっている入力（動画ファイルやカメラ）を停止・削除してから配信を開始する。
  - 入力の**デコード**をソフトウェアにする（Video/UVC入力の「デコード」設定。カメラ内部の変換はAVFoundationが行うため、完全にCPUへは切り替わりません）。
- ハードウェアエンコーダーが無い・応答しない場合は、モジュールが理由を付けて拒否します。別のエンコーダーへ黙って切り替わることはありません。

## Windows: NVENC / Media Foundation

- **NVENC（GeForce）** は世代により同時エンコード本数が制限されます（公式にはGeForceで3本とされていた時期があります）。上限はAPIで取得できず、セッション作成時にドライバーが拒否します（[NVENC](https://en.wikipedia.org/wiki/NVENC)）。Quadro/RTX Aシリーズやデータセンター向けはこの制限が緩い場合があります。
- **Media Foundation** はOS側のエンコーダーで、GPUベンダーのハードウェアMFTかOSのソフトウェアMFTが選ばれます。利用できるデバイスは `mixer_stream_encoder_devices` で列挙でき、設定の「エンコーダーデバイス」に出ます。同時セッション数の上限は非公開で、開始時に拒否されます。
- **Vulkan Video**（Linux実験的、Windowsでは入力デコードに使用）も、対応コーデックとセッション数はドライバー依存です。

## メモリ帯域とPCIe

- **Windows（ディスクリートGPU）**: 入力フレームはPCIe経由でGPUへ上がります。4K60のUYVYは約1.0 GB/s、BGRAは約2.0 GB/sです。複数の4K入力を同時に受けるとPCIe帯域とVRAM転送がボトルネックになります。CPUからVRAMへ直接書ける**Resizable BAR**が有効だとこの転送が軽くなります（[システム要件](./requirements/)）。
- **macOS（Apple Silicon）**: Unified MemoryのためCPU/GPU間のコピーはありませんが、GPUのメモリ帯域は映像合成・デコード・エンコードで共有されます。4K入力を多数同時に扱うとフレーム落ちの原因になります。
- **Linux（実験的）**: Vulkan host-visible VRAMが必要です（[システム要件](./requirements/)）。

## DeckLink

DeckLinkの入出力数はハードウェアのポート数ではなく**プランの上限**で決まります（[プラン](./plans/)）。上限を超える追加は `ERR_NOT_SUPPORTED_PLAN` になります。

## ログでの見え方

| ログ | 意味 |
| --- | --- |
| `VideoToolbox is not responding; dropping frames for …` | VTエンコーダーが期限内に応答していない。他の入力がVTを掴んでいる可能性 |
| `stream encoder id=…: …` | エンコーダー開始の拒否理由（解像度・fps・プラン・エンコーダー種別） |
| `stream destination id=…: …` | 宛先追加の拒否理由（URL・シークレット・エンコーダー未起動） |
| `omt_connect id=…: Connection refused` | OMT入力の送信側が居ない。再試行は1分に1回だけ記録 |
