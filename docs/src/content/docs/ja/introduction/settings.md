---
title: 設定
description: セッションに保存される設定ウィンドウの項目
---

メインウィンドウの「設定」から開きます。

## 表示

GUI上の表示を調整します。

<img src="/eiviz/images/ja/introduction/settings/setting_ui.jpg" alt="設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

### 色設定

Preview色、Program色、非アクティブ色は、ボタンやシーンタイルの縁に使います。既定は緑、赤、灰色です。

### マスターフレームレート

セッションで共通で使われるフレームレートです。既定はNTSC 59.94pで、50p、30p、24p、60pも選べます。  
この値はMixer生成時に渡すので、変えたあとはセッションを保存して開き直すか、アプリを再起動してください。  
各Mixing Unitは、ユニットのダイアログから出力フレームレートを上書きできます。

### Mixing Unitの規定サイズ

これから追加で作成するMixing Unitの規定解像度です。既定は1920x1080。既にあるユニットは変わりません。この項目はWindowsの表示タブにあります。

### フレームバッファ

完成したフレームをメモリ上に保持する枚数です。既定は3フレーム保持します。  
映像処理が遅れても出力が止まらないためのバッファリングとして機能し、音声も同じフレーム数だけ遅れます。  
このフレーム数中に処理が追い付かなかった場合、リアルタイムに追いつくまでフレームをスキップします。仕組みは[システムアーキテクチャ](/eiviz/ja/introduction/architecture/)です。


### 内部カラーフォーマット

RAM上とGPUへ上げるときの画素の持ち方です。既定のUYVY 4:2:2はBGRAの半分のサイズで、実際に合成するフレームだけRGBへ変換します。BGRA 8-bit 4:4:4は変換を省きたいとき用です。変えるとファイル入力などのポンプを作り直します。この項目もWindowsの表示タブにあります。

## パフォーマンス

映像合成処理に関わるパフォーマンス設定です。

<img src="/eiviz/images/ja/introduction/settings/performance.jpg" alt="パフォーマンス設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

## グラフィックスアダプター

いま使っているグラフィックスアダプター名を表示します。

### Resizable Bar / Unified Memory

WindowsではResizable BARの可否と、BAR窓/VRAM、GPU upload heapの有無を出します。  
「ReBAR最適化を使う」は、映像のアップロード時にGPUのVRAM領域へ直接書き込みます。内蔵GPUや、GPUがupload heapを出さない環境では選べません。ちらつきやデバイスリセットが出たらオフにしてください。ReBAR自体の前提は[eivizについて](/eiviz/ja/introduction/about/)をご参照ください。

macOS（Apple Silicon）ではUnified Memory向けの最適化を導入しています。ライブ入力を`MTLStorageModeShared`の共有テクスチャ領域へ書き、そのままサンプルします。オフにすると通常のMetalアップロードに戻ります。

### NDIを取り込みスレッドでアップロード

既定はオンです。オンだと、NDIから受信した各フレームをGPUへ書き、映像合成・転送処理を高速化します。オフだと、レンダースレッドでCPUフレームをアップロードします。

## 出力

映像の出力先設定です。

<img src="/eiviz/images/ja/introduction/settings/outputs.jpg" alt="出力設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

各行は名前、転送方式、On/Off切り替え、映像ソース、音声です。  
転送方式にはOMTとNDIに対応しています。Decklinkなどのハードウェア出力は現在実装中です。  
映像ソースはInput、Scene、MU PRV、MU PGM、Multiviewから選択が可能です。  
音声はMaster, Headphone, 各Audio Aux、またはNone(音声なし)から選択可能です。  
Multiviewを映像ソースに選択した場合、音声の送出は出来ません。

OMTはエンコード方式を選択可能です。GPU encodeはフレームをGPUに載せたままVMXコーデックに変換し送信します。CPU encodeを選択した場合、UYVY形式で読み出し、CPU上の送出専用スレッドでVMXコーデックへの変換・送信を行います。  
NDIは常にCPU encodeです。

各出力毎に1スレッド割り当てられます。詳細は[NDI/OMT](/eiviz/ja/features/outputs/ndi-omt/)です。

Windowsは設定をOKしたときに適用され、macOSは行のApplyでも適用可能です。

## Multiview

<img src="/eiviz/images/ja/introduction/settings/multiview.jpg" alt="Multiview設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

マルチビューの設定と追加制御です。詳細は[Multiviews](/eiviz/ja/concepts/multiviews/)をご参照ください。

### 新規Multiviewの既定Mixing Unit

マルチビューを新規作成した際の規定Preview/Programの対象Mixing Unitです。

### プロジェクト既定のプレビュー更新間隔

プロジェクトデフォルトのフレーム更新間隔です。パフォーマンスへの影響を防ぐため、PCスペックが十分ではない環境では更新間隔を下げることを推奨します。    
毎フレームから8フレームおきまで設定可能です。既定は3フレームごとで、59.94ではおよそ20 fpsです。  

## 音声AUX

<img src="/eiviz/images/ja/introduction/settings/audio-aux.jpg" alt="音声AUX設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

内部ミックスは48 kHzステレオです。A〜HのAudio AUXを最大8本足せます。  
詳細は[Audio Auxs](/eiviz/ja/concepts/audio-auxs/)と[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)をご参照ください。

Enabledを選択時は、出力デバイスを設定せずに内部でのミックス処理のみ動かします。

「HeadphoneはMasterをコピー」を入れると、HeadphoneバスはMasterと同じ中身になります。キュー用に別内容を流したいときは外します。

## 詳細

### 映像出力先ウィンドウの上限

Preview/Program/Multiviewをリアルタイムに表示するのに使います。  
たとえばSwitcher UIはPreviewとProgramを出すので、2スロット使います。本体のPreview/Program、開いているMultiview、Scene Editor、Overlay窓も同様に数えます。シーンタイルと入力プレビューのサムネは数えません。

自動は6からです。設定から上げられますが、不安定になる可能性があります。  
上限に達すると新しい窓は開きません。どれかを閉じると枠が空きます。技術的な背景は[システムアーキテクチャ](/eiviz/ja/introduction/architecture/)のホストです。

## 環境設定

<img src="/eiviz/images/ja/introduction/settings/preferences.jpg" alt="環境設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

グローバルなeivizの設定です。　　

### 言語

英語/日本語に対応しています。

### テーマ

ダーク、ライト、OS設定を選択可能です。
