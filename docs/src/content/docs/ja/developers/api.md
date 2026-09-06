---
title: eiviz API
description: eiviz固有の制御APIとheadless運用
---

eivizの制御面はMixer内の`ControlService`が正本です。vMix互換HTTP/TCP、ProtobufのWebSocket、`eivizctl`はいずれも同じディスパッチャへ変換されます。vMix互換の待ち受けは[互換API](/eiviz/ja/developers/compatibility/)です。このページはeiviz固有のProtobuf面です。

## 公開契約

- プロトコル: `eiviz.control.v1`（`crates/eiviz-api/proto/eiviz/control/v1/control.proto`）
- WebSocket: `ws://127.0.0.1:9400`、subprotocol `eiviz.protobuf.v1`、binary frame 1枚がEnvelope 1個
- 公開済みfield numberは変更・再利用しません。削除時は`reserved`へ入れます

映像/音声フレーム、GPU texture、HWND/NSViewなどの描画・データ面はネットワーク公開対象外です。

## 認証と権限

既定bindはloopbackです。tokenは`EIVIZ_API_TOKEN`または`EIVIZ_API_TOKEN_FILE`から読み、コマンドラインやセッションJSONには保存しません。比較はconstant-timeです。

権限は`read`/`operate`/`configure`/`admin`です。任意パスのload/save/shutdownはadmin限定です。セッション本体はpathではなくbytesで送受信します。リモートbindは認証必須です。

## Command

| Command | 権限 | 内容 |
| --- | --- | --- |
| `GetCapabilities`/`GetSnapshot`/`Subscribe` | read | 能力、Document+LiveState、イベント購読 |
| `Preview`/`Cut`/`Auto`/`SetMix` | operate | Mixing Unitのライブ操作 |
| `VideoPlay`/`VideoLoop`/`VideoSeek` | operate | ビデオInput |
| `AudioSetInput`/`AudioSetBus` | operate | 音声 |
| `SnapshotCmd`/`Discover` | operate | スクリーンショットと発見 |
| `ReplaceSession` | configure | 正本Documentの置換（`expected_revision`でlost updateを拒否） |
| `Shutdown` | admin | graceful停止 |

CutでInputを指定した場合はPreviewを変えません。未指定ならPreviewをtakeしてswapします。

購読が遅れた場合は`Lag`イベントが返り、`GetSnapshot`のあと`after_sequence`から再開してください。

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

`validate`と`canonicalize`はGPUを初期化しません。`run`はセッションをparse/validateし、そのFPSでruntimeを作り、replace/reconcileしたあとAPI readinessを出して待機します。Ctrl+C/SIGTERMでは受付停止→worker→Input/Output→renderの順に期限付きで停止します。

終了コードは引数/読込が2、セッション検証が3、GPU/runtimeが4、bindが5、その他runtime失敗が6です。

## 運用

- token rotation: `EIVIZ_API_TOKEN`を差し替えてdaemonを再起動します
- loopback以外へbindする場合は認証とTLS terminationを必須にします
- ログはstderrの構造化可能なテキストです
- バックアップはセッションJSONを`eiviz-headless canonicalize`で正規化して保管します
- GUIからheadlessを起動する場合は同梱の`eiviz-headless`を明示的にspawnしてください
