---
title: eiviz API
description: eiviz固有の制御APIとheadless運用
---

eivizの各種操作は、Mixer内の`ControlService`を通じて処理されます。vMix互換HTTP/TCP、Protobuf WebSocket、CLIツール（`eivizctl`）からの要求はすべて同一のディスパッチャに集約されます。vMix互換APIについては[互換API](/eiviz/ja/developers/compatibility/)を参照してください。本ページではProtobufベースの制御APIについて解説します。

## 基本仕様

- プロトコル: `eiviz.control.v1`（`crates/eiviz-api/proto/eiviz/control/v1/control.proto`）
- 通信方式: WebSocket（`ws://`）、サブプロトコル`eiviz.protobuf.v1`（バイナリフレーム1枚につきEnvelope 1件）
- 接続ポート: 既定はloopbackのポート9400。bindアドレスや認証tokenなどの接続設定はホスト固有で管理され、セッションファイルには保存されません。
- セキュリティ: 暗号化（WSS/TLS）には対応していません。信頼できるLANまたはVPN環境で運用してください。
- 映像や音声の実データ、GPUテクスチャ、ウィンドウハンドル（HWND/NSView）はAPIの対象外です。

## 認証と権限

loopback（127.0.0.1）以外のアドレスへbindする場合は、認証tokenの設定が必須です。tokenの照合には定数時間比較（constant-time）が使用されます。

### ロールと権限

操作権限は以下の4段階です。クライアント側の自己申告で権限を昇格することはできず、ホスト側で設定された最大ロール（maxRole）が上限となります。

- `read`: 状態取得、スナップショット取得、イベント購読
- `operate`: カットやオート、Tバーなどのスイッチング操作、メディア再生、音声設定
- `configure`: セッション変更（`MutateSession`）、現在ファイルへの保存（`SaveSession`）、メディアファイルのアップロード
- `admin`: 任意パスの保存/読み込み、シャットダウン

### ブラウザ接続（Origin制限）

Webブラウザからの接続時は、環境変数`EIVIZ_API_ALLOWED_ORIGINS`にカンマ区切りで許可するOriginを指定します（`*`で全許可）。未設定の場合、悪意のある外部サイトからの不正操作を防ぐため、`Origin`ヘッダーを持つ接続は403エラーで拒否されます。

## コマンド一覧

| コマンド | 必要ロール | 説明 |
| --- | --- | --- |
| `GetCapabilities` / `GetSnapshot` / `Subscribe` | read | 機能取得、状態取得、イベント購読 |
| `Preview` / `Cut` / `Auto` / `SetMix` / `OverlayAuto` | operate | スイッチングおよびトランジション操作。`OverlayAuto`はOverlayのIDを指定します |
| `VideoPlay` / `VideoLoop` / `VideoSeek` | operate | 動画Inputの再生制御 |
| `AudioSetInput` / `AudioSetBus` | operate | 入力の送り先Mixing Unit、またはMU Bus/ヘッドホンのゲインとミュート |
| `SnapshotCmd` / `Discover` | operate | スクリーンショット取得、ソース検出 |
| `ReplaceSession` | configure | セッション全体の差し替え（リビジョン検証あり） |
| `MutateSession` | configure | セッションの部分変更（リビジョン不整合時は拒否） |
| `SaveSession` | configure | 現在のセッションファイルへの上書き保存 |
| `BeginUpload` / `WriteChunk` / `CommitUpload` / `AbortUpload` | configure | メディアファイルのアップロード |
| `Shutdown` | admin | デーモン/ホストの終了 |

## エラーハンドリング

APIエラーコードは`INVALID_ARGUMENT`、`NOT_FOUND`、`AMBIGUOUS`、`CONFLICT`、`UNAVAILABLE`、`PERMISSION_DENIED`、`IO`、`INTERNAL`に分類されます。同名リソースが存在する場合は自動解決せず`AMBIGUOUS`を返します。

## CLIおよびデーモンの仕様

- `eivizctl`: コマンドラインからAPIを呼び出すツールです。`--repl`で対話モードに対応します。詳細は[headless](/eiviz/ja/features/headless/)を参照してください。
- `eiviz-headless`: GUIを持たないデーモン実行用バイナリです。Ctrl+C、SIGTERM、または`Shutdown`コマンドにより安全に終了処理を行います。
