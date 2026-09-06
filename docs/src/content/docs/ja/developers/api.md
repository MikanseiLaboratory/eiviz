---
title: eiviz API
description: eiviz固有の制御APIとheadless運用
---

eivizの制御面はMixer内の`ControlService`が正本です。vMix互換HTTP/TCP、ProtobufのWebSocket、`eivizctl`はいずれも同じディスパッチャへ変換されます。vMix互換の待ち受けは[互換API](/eiviz/ja/developers/compatibility/)です。このページはeiviz固有のProtobuf面です。

## 公開契約

- プロトコル: `eiviz.control.v1`（`crates/eiviz-api/proto/eiviz/control/v1/control.proto`）
- WebSocket: `ws://`、subprotocol `eiviz.protobuf.v1`、binary frame 1枚がEnvelope 1個
- 既定の待ち受けはloopbackのポート9400です。bindアドレス、ポート、token、最大role、メディア保存先はホスト固有です（GUIの環境設定、または`eiviz-headless --bind`/`EIVIZ_API_TOKEN`/`EIVIZ_MEDIA_DIRECTORY`）。セッションJSONには保存しません
- このリリースは信頼できるLANまたはVPN上の認証付き`ws://`のみです。TLSは提供しません
- 公開済みfield numberは変更・再利用しません。削除時は`reserved`へ入れます

映像/音声フレーム、GPU texture、HWND/NSViewなどの描画・データ面はネットワーク公開対象外です。

## 認証と権限

既定bindはloopbackです。headlessのtokenは`EIVIZ_API_TOKEN`または`EIVIZ_API_TOKEN_FILE`から読みます。GUIの待ち受けとリモートクライアントのtokenはWindows Credential Manager/macOS Keychainに置きます。セッションJSONには保存しません。比較はconstant-timeです。

権限は`read`/`operate`/`configure`/`admin`です。サーバーが付与roleをホストの最大roleで打ち止めにし、クライアント自己申告では昇格できません。任意パスのload/save/shutdownはadmin限定です。セッション本体はpathではなくbytesで送受信します。loopback以外へのbindは認証必須です。

## Command

| Command | 権限 | 内容 |
| --- | --- | --- |
| `GetCapabilities`/`GetSnapshot`/`Subscribe` | read | 能力、Document+LiveState、イベント購読 |
| `Preview`/`Cut`/`Auto`/`SetMix`/`OverlayAuto` | operate | Mixing Unitのライブ操作 |
| `VideoPlay`/`VideoLoop`/`VideoSeek` | operate | ビデオInput |
| `AudioSetInput`/`AudioSetBus` | operate | 音声 |
| `SnapshotCmd`/`Discover` | operate | スクリーンショットと発見 |
| `ReplaceSession` | configure | 正本Documentの置換（`expected_revision`でlost updateを拒否） |
| `MutateSession` | configure | 型付きDocument変更（`expected_revision`でlost updateを拒否。自動マージしません） |
| `BeginUpload`/`WriteChunk`/`CommitUpload`/`AbortUpload` | configure | ホスト保存先へのメディアupload。commit時だけStill/Video Inputを原子的に追加 |
| `Shutdown` | admin | graceful停止 |

CutでInputを指定した場合はPreviewを変えません。未指定ならPreviewをtakeしてswapします。

購読が遅れた場合は`Lag`イベントが返り、snapshotを取り直したあと`after_sequence`から再開してください。sequence欠番とserver epoch変更もsnapshot再同期です。Subscribeは常駐で、購読直後はsnapshotとsequence barrierを一度に渡します。

## リモートGUI

WindowsとmacOSは環境設定からリモートクライアントとして接続できます。状態の正本はリモート側のセッションです。クライアントの設定は確認専用で、言語・テーマ・接続先などクライアント固有の環境設定だけ編集できます。Input Previewとシーンサムネは使いません。Preview/Program/Multiview映像は、すでに有効なNDIまたはOMT出力のうち`MuPreview`/`MuProgram`/`Multiview`だけを受信します。eivizは出力を追加作成しません。該当なし、または複数一致のときはUnavailableと出します。

Still/Videoの追加はクライアント側でファイルを選び、ホストのメディア保存先へuploadしたあとInputへ載せます。パストラバーサル、上書き、symlink/junction先は拒否します。

## エラー

`INVALID_ARGUMENT`/`NOT_FOUND`/`AMBIGUOUS`/`CONFLICT`/`UNAVAILABLE`/`PERMISSION_DENIED`/`IO`/`INTERNAL`です。名前の重複は先勝ちにせず`AMBIGUOUS`です。

## eivizctl

```bash
eivizctl --url ws://127.0.0.1:9400 status
eivizctl cut --unit 1
eivizctl snapshot
eivizctl shutdown
```

## headless daemon

```bash
eiviz-headless validate --session show.eiviz.json
eiviz-headless canonicalize --session show.eiviz.json
eiviz-headless run --session show.eiviz.json --bind 127.0.0.1:9400
```

`validate`と`canonicalize`はGPUを初期化しません。`run`はセッションをparse/validateし、そのFPSでruntimeを作り、replace/reconcileしたあとAPI readinessを出して待機します。Ctrl+C/SIGTERMでは受付停止→worker→Input/Output→renderの順に期限付きで停止します。GUIのMixerも、環境設定（bind/token/メディア保存先）または設定（有効/ポート）で待ち受けを有効にすると同じWebSocketを開きます。

終了コードは引数/読込が2、セッション検証が3、GPU/runtimeが4、bindが5、その他runtime失敗が6です。

## 運用

- token rotation: headlessは`EIVIZ_API_TOKEN`を差し替え、GUIは環境設定の待ち受けtokenを変えて再起動します
- loopback以外へbindする場合は認証必須です。このリリースは信頼できるLANまたはVPN上の認証付き`ws://`のみです。TLSが必要なら手前で終端してください
- `--media-directory`/`EIVIZ_MEDIA_DIRECTORY`がホストのupload保存先です
- ログはstderrの構造化可能なテキストです
- バックアップはセッションJSONを`eiviz-headless canonicalize`で正規化して保管します
