---
title: 設定
description: セッションに保存される設定ウィンドウの項目
---

メインウィンドウの「設定」ボタンから開きます。リモート接続中も同様のウィンドウで接続先のセッション設定を編集できます。

## 表示

画面表示や基本解像度、フレームレートを設定します。

<img src="/eiviz/images/ja/introduction/settings/setting_ui.jpg" alt="設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

- **色設定**: Preview色、Program色、非アクティブ色の枠線やボタン表示色を設定します（既定: 緑、赤、グレー）。
- **マスターフレームレート**: セッション共通の基本フレームレートです（既定: 59.94p）。変更後はセッションを保存して再起動するか、セッションを再度開き直してください。各Mixing Unitで個別に上書きも可能です。
- **Mixing Unitの規定サイズ**: 新規作成するMixing Unitの解像度です（既定: 1920x1080）。
- **フレームバッファ**: 映像処理の遅延を吸収し出力を安定させるためのバッファ枚数です（既定: 3フレーム）。音声も同一フレーム数分だけ遅延同期されます。
- **内部カラーフォーマット**: 内部処理のピクセルフォーマットです（既定: UYVY 4:2:2）。RGB変換の負荷を削減しています。

## パフォーマンス

ハードウェアアクセラレーションに関する設定です。

<img src="/eiviz/images/ja/introduction/settings/performance.jpg" alt="パフォーマンス設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

- **グラフィックスアダプター**: 使用中のGPU名を表示します。
- **Resizable BAR / Unified Memory**: 
  - Windows: ReBAR最適化を有効にすると、CPUからVRAMへの直接転送を行います。画面のちらつきが発生する場合は無効にしてください。
  - macOS: Apple SiliconのUnified Memory最適化を使用し、共有テクスチャ領域経由で効率的に転送します。
- **NDIを取り込みスレッドでアップロード**: オン（既定）の場合、NDI受信スレッドからGPUへ直接アップロードし、描画スレッドの負荷を軽減します。

## 出力

ネットワーク（OMT/NDI）への映像・音声送出を設定します。

<img src="/eiviz/images/ja/introduction/settings/outputs.jpg" alt="出力設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

- **映像ソース**: Input、Scene、MU PRV、MU PGM、Multiviewから選択できます。
- **音声ソース**: Mixing UnitのMU Bus、またはNone（音声なし）から選択します（Multiview選択時は音声なし固定）。
- **解像度・フレームレート**: 出力ごとに個別に指定するか、「セッション設定を使用」を選択します。
- **エンコード方式（OMT）**:
  - **CPU（推奨）**: 高品質な標準エンコーダー。本線配信やプログラム出力に使用します。
  - **GPU**: GPU負荷を活用する補助エンコーダー。マルチビューなどの監視用途に適しています。
  - ※NDIは常にCPUでエンコードされます。

## Multiview

マルチビュー機能の動作を設定します。詳細は[Multiviews](/eiviz/ja/concepts/multiviews/)を参照してください。

<img src="/eiviz/images/ja/introduction/settings/multiview.jpg" alt="Multiview設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

- **新規Multiviewの既定Mixing Unit**: マルチビュー作成時にPreview/Programとして割り当てる既定のユニットを指定します。
- **プレビュー更新間隔**: タイルの更新頻度を設定します（既定: 3フレームごと、約20 fps）。スペックが不足する場合は間隔を広げることで描画負荷を軽減できます。

## ヘッドホン

<img src="/eiviz/images/ja/introduction/settings/audio-aux.jpg" alt="ヘッドホン設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

内部ミックスは48 kHzステレオです。このページでは、選択中のMixing Unitをcueするヘッドホンの出力先を設定します。各Mixing Unitの出力デバイスは、Mixing Unitのダイアログで選びます。詳細は[MU Bus](/eiviz/ja/concepts/audio-auxs/)と[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)を参照してください。

- **None**: 内部ミックスのままです。実機へ出すときはWASAPI共有またはASIOを使います。
- **ヘッドホンはcue中のMU Busをコピー**: オンにすると、cue中のMU Busと同じミックスを出します。オフのときは、cue対象を別にミックスします。

## Web API

外部制御用のAPIサーバーを設定します。設定の詳細は[eiviz API](/eiviz/ja/developers/api/)および[互換API](/eiviz/ja/developers/compatibility/)を参照してください。

- **HTTP**: vMix互換HTTPサーバー（既定ポート: 8088）。Basic認証のユーザー名・パスワードを設定可能です。
- **TCP**: vMix互換TCPサーバー（既定ポート: 8099）。認証には対応していません。
- **WebSocket**: eivizネイティブのProtobuf制御API（既定ポート: 9400）。loopback以外へのbindには認証tokenが必須です。
- **ブラウザのOrigin制限**: WebブラウザからWebSocketに接続する場合は、環境変数`EIVIZ_API_ALLOWED_ORIGINS`に対象のOriginまたは`*`を指定する必要があります。

## 詳細設定

- **映像出力先ウィンドウの上限**: PreviewやProgram、Multiviewをリアルタイム表示するための描画スロット数の上限です（既定: 6）。上限に達すると新規プレビューウィンドウが開けなくなります。不要なウィンドウを閉じるとスロットが解放されます。

## 環境設定（Preferences）

PC全体で共有されるグローバル設定です。

<img src="/eiviz/images/ja/introduction/settings/preferences.jpg" alt="環境設定ウィンドウのスクリーンショット" style="max-width: 100%; height: auto;" />

- **言語**: 日本語/英語の切り替え
- **テーマ**: ダーク、ライト、OS連動
- **API待ち受け**: ホスト側のbindアドレス、認証token、メディア保存ディレクトリの設定
- **ヘルプ**: 公式ドキュメントの表示
