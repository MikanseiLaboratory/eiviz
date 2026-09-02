---
title: 設定
description: セッションに保存される設定ウィンドウの項目
---

メインウィンドウの「設定」から開きます。値はセッションJSONに入り、WindowsとmacOSで同じファイルとして開けます。言語とウィンドウのテーマは、隣の「環境設定」です。

左側がカテゴリです。表示、パフォーマンス、出力、Multiview、音声AUX。OKでMixerへ渡し、キャンセルは捨てます。Windowsの「初期値」は、表示まわりと色、ReBAR、NDIアップロードを既定に戻します。出力とバスの一覧は触りません。

## 表示

Preview色、Program色、非アクティブ色は、ボタンやシーンタイルの縁に使います。既定は緑、赤、暗い灰です。

マスターフレームレートはミックスクロックです。既定はNTSC 59.94pで、50p、30p、24p、60pも選べます。この値はMixer生成時に渡すので、変えたあとはセッションを保存して開き直すか、アプリを再起動してください。各Mixing Unitは、ユニットのダイアログから出力フレームレートを上書きできます。

Mixing Unitの既定サイズは、これから足すユニットの解像度です。既定は1920x1080。既にあるユニットは変わりません。この項目はWindowsの表示タブにあります。

フレームバッファは、完成したフレームを何枚持つかです。1〜8、既定は3。合成が一瞬遅れても出力が止まらないための逃げで、音声も同じフレーム数だけ遅れます。空になると、クロックが追いつくまで合成を飛ばします。仕組みは[システムアーキテクチャ](/introduction/architecture/)です。

内部カラーフォーマットは、RAM上とGPUへ上げるときの画素の持ち方です。既定のUYVY 4:2:2はBGRAの半分のサイズで、実際に合成するフレームだけRGBへ変換します。BGRA 8-bit 4:4:4は変換を省きたいとき用です。変えるとファイル入力などのポンプを作り直します。この項目もWindowsの表示タブにあります。

## パフォーマンス

いま使っているグラフィックスアダプター名が出ます。Mixerが落ちていると出せません。

WindowsではResizable BARの可否と、BAR窓/VRAM、GPU upload heapの有無を出します。「ReBAR最適化を使う」は、wgpuのシステムメモリステージングではなく、D3D12のGPU upload heap（VRAM）へ書きます。内蔵GPUや、GPUがupload heapを出さない環境では選べません。ちらつきやデバイスリセットが出たらオフにしてください。ReBAR自体の前提は[eivizについて](/introduction/about/)です。

macOS（Apple Silicon）ではUnified Memory最適化です。ライブ入力を`MTLStorageModeShared`のテクスチャへ書き、そのままサンプルします。オフにすると通常のMetalアップロードに戻ります。

「NDIを取り込みスレッドでアップロード」は両OS共通で、既定はオンです。オンだと、ミキサーがサンプルする前に各フレームをGPUへ書き、ミックスクロックがmemcpyで止まりません。オフだと、レンダースレッドでCPUフレームを上げる経路に戻ります。

## 出力

OMTとNDI®はMixerから送ります。送出の中身は[NDI/OMT](/features/outputs/ndi-omt/)です。

各行は名前、輸送、Enabled、ソースです。輸送はOMTとNDI。WindowsはDeckLinkも出ますが、このビルドではSDKを繋いでいないのですぐ失敗します。ソースはInput、Scene、MU PRV、MU PGM、Multiviewから選びます。

OMTだけEncode pathを持てます。GPU encodeはフレームをGPUに載せたまま送り、CPU encodeはUYVYへパックしてリードバックします。NDIは常にCPU encodeです。新規セッションの`eiviz-pgm`はOMT、Mixing UnitのProgram、GPU encodeです。

Windowsは設定をOKしたときにMixerへ渡し、macOSは行のApplyでも渡せます。

## Multiview

モザイクの定義とウィンドウの追加です。概念は[Multiviews](/concepts/multiviews/)。`+`で足し、開く、レイアウト、削除があります。フルスクリーンはF11です。

「新規Multiviewの既定Mixing Unit」は、新しいモザイクがどのユニットのPreview/Programを見るかです。

「プロジェクト既定のプレビュー更新間隔」は、ウィンドウ側が「プロジェクトに従う」ときの間引きです。毎フレームから8フレームおきまで。既定は3フレームごとで、59.94ではおよそ20 fpsです。Program、Preview、ネットワーク出力はマスターフレームレートのままです。監視用の絵だけ落とします。

## 音声AUX

内部ミックスは48 kHzステレオです。MasterとHeadphoneは消せません。A〜HのAUXを最大8本足せます。バスの意味は[Audio Auxs](/concepts/audio-auxs/)、デバイス側は[音声、ASIOなど](/features/outputs/audio/)です。

Enabledは、ハードウェアなしでバスをミックスに残します。実機へ出すときは、WindowsがWASAPIとASIO、macOSがCore Audioです。Left/Rightをデバイスのチャンネル番号へ割り当てます。WASAPIはExclusiveを持てます。

「HeadphoneはMasterをコピー」を入れると、HeadphoneバスはMasterと同じ中身になります。キュー用に別内容を流したいときは外します。

macOSの行にはゲインとMuteがあります。Windowsのこの画面にはありません。

## 環境設定

設定とは別のダイアログです。言語（English/日本語）とテーマ（ダーク、ライト、OSの設定に従う）を持ち、`prefs.json`に保存します。セッションファイルには入りません。
