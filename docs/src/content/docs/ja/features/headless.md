---
title: headless
description: GUIなしでMixerを動かし、eivizctlとRemoteから操作する
---

`eiviz-headless`はホストUIなしでMixerを動かすデーモンです。セッションファイル（`.eivz`または`.eivzx`）を読み、映像合成を行い、Protobuf WebSocketを開きます。既定はloopbackのポート9400です。`run`で`--session`を省略すると、OSの`eiviz/sessions`配下へ日付付きの既定`.eivz`を書き、そのファイルを使います。

Linuxは現時点でこの形だけです。WindowsとmacOSのリリースにも同じバイナリが入っています。操作は同じマシンの`eivizctl`、または別PCの`Eiviz.Remote.exe`/`eiviz-remote.app`から行います。プロトコルは[eiviz API](/eiviz/ja/developers/api/)、Remoteの画面操作は[リモート接続](/eiviz/ja/features/remote/)です。

初回は「起動」から「Remoteから接続する」まで通してください。待ち受けのキーやREPLのコマンドは、そのあと必要なときだけ引いてください。

## できること

Mixerの合成、Input/Output、セッションの適用はGUIホストと同じです。開く制御面はProtobuf WebSocketだけです。vMix互換HTTP（8088）とvMix互換TCP（8099）は開きません。

Preview/Programのホスト窓、Input Preview、シーンサムネイルはHost向けです。headlessの絵をRemoteで見るときは、接続先でNDIまたはOMT出力を有効にして、Remote側のヘッダーからそのソースを選びます。

## 起動

リリースに同梱の`eiviz-headless`と`eivizctl`を使うか、ソースからReleaseビルドします。

```bash
cargo build -p eiviz-headless --locked --release --bins
```

バイナリは`target/release/eiviz-headless`と`target/release/eivizctl`です。セッションの例は`headless/tests/fixtures/bars.eivz`です。

```bash
eiviz-headless validate --session show.eivz
eiviz-headless canonicalize --session show.eivz
eiviz-headless export --session show.eivz --output show-portable.eivzx
eiviz-headless history --session show.eivz
eiviz-headless restore --session show.eivz --index 0 --output old.eivz
eiviz-headless run --session show.eivz --bind 127.0.0.1:9400
eiviz-headless run --bind 127.0.0.1:9400
```

`validate`と`canonicalize`はGPUを初期化しません。`run`はセッションを検証し、そのFPSでruntimeを作り、WebSocketの受付を始めて待機します。`--session`を省略すると、OSの`eiviz/sessions`配下へ日付付きの既定ファイルを作ります。準備できるとstderrへ`eiviz-headless session=`と`eiviz-headless ready ws=`が出ます。Ctrl+C（UnixはSIGTERMも）で停止します。

`--bind`を省略すると`eivizctl prefs`のbind、それも無ければ`127.0.0.1:9400`です。loopback以外へbindするときはtokenが必須です。このリリースは信頼できるLANまたはVPN上の認証付き`ws://`のみで、TLSは含みません。

## 待ち受け

GUIの環境設定に相当する値は、ホスト固有です。セッションファイルには保存しません。

| キー | 内容 |
| --- | --- |
| `bind` | WebSocketの待ち受け。例: `127.0.0.1:9400`、LANなら`0.0.0.0:9400` |
| `token` | 接続token。表示は`(set)` |
| `mediaDirectory` | Remoteから上げたStill/Videoの保存先 |
| `maxRole` | 付与roleの上限。`read`/`operate`/`configure`/`admin` |

ファイルは`%LOCALAPPDATA%\eiviz\headless-prefs.json`（Windows）、または`$XDG_CONFIG_HOME/eiviz/headless-prefs.json`（未設定なら`~/.config/eiviz/headless-prefs.json`）です。反映は次の`eiviz-headless run`です。

優先順位は次のとおりです。左が勝ちます。

- bind: `--bind` → prefsの`bind` → `127.0.0.1:9400`
- token: `EIVIZ_API_TOKEN`または`EIVIZ_API_TOKEN_FILE` → prefsの`token`
- メディア保存先: `--media-directory`または`EIVIZ_MEDIA_DIRECTORY` → prefsの`mediaDirectory` → OSのローカルアプリデータ配下`eiviz/media`
- 最大role: prefsの`maxRole` → 環境変数`EIVIZ_API_ROLE` → tokenがあれば`admin`、無ければ`read`

未指定のメディア保存先はWindowsが`%LOCALAPPDATA%\eiviz\media`、macOSが`~/Library/Application Support/eiviz/media`、Linuxが`$XDG_DATA_HOME/eiviz/media`（未設定なら`~/.local/share/eiviz/media`）です。

## 対話型CLI

既定はサブコマンドのCLIです。`--repl`を付けたときだけ対話型になります。プロンプトは`eiviz>`です。`exit`または`quit`で抜けます。接続先は`--url`で、既定は`ws://127.0.0.1:9400`です。クライアントのtokenは`--token`または`--token-file`です。REPLはWebSocketを1本維持し、コマンドごとに切断しません。

```bash
eivizctl --url ws://127.0.0.1:9400 --token YOUR_TOKEN cut --unit 1
eivizctl --repl --url ws://127.0.0.1:9400 --token YOUR_TOKEN
```

待ち受けの編集と、動いているMixerへの操作は別物です。

`prefs`はローカルの待ち受けファイルです。daemonが止まっていても書けます。ライブ操作と`mutate`は、先に`eiviz-headless run`が`ready`になっている必要があります。

```text
eiviz> prefs
eiviz> prefs get bind
eiviz> prefs set bind 0.0.0.0:9400
eiviz> prefs set token YOUR_TOKEN
eiviz> prefs set mediaDirectory /var/lib/eiviz/media
eiviz> prefs set maxRole configure
```

空文字を渡すとそのキーを消します。tokenの中身は表示しません。

サブコマンドでも同じです。

```bash
eivizctl prefs
eivizctl prefs get bind
eivizctl prefs set bind 127.0.0.1:9400
```

ライブ操作は1行で打てます。権限の対応は[eiviz API](/eiviz/ja/developers/api/)です。

```text
eiviz> status
eiviz> snapshot
eiviz> preview --unit 1 --scene 2
eiviz> cut --unit 1
eiviz> auto --unit 1 --duration-ms 1000
eiviz> replace --session show.eivz
eiviz> save
eiviz> shutdown
```

セッションの部分更新は`mutate`です。JSONの`kind`はcamelCaseです。`expected_revision`が0のときは衝突検査をしません。REPLの`mutate`は常に0です。

```text
eiviz> mutate {"kind":"deleteInput","id":2}
```

設定ウィンドウ相当は`kind`が`setSettings`です。`settings`、`outputs`、`buses`をまとめて送ります。欠けると拒否されるので、先に`snapshot`で現状を取るか、Remoteの設定ウィンドウから送ってください。

## Remoteから接続する

`eiviz-headless`が開くWebSocketは、GUIホストと同じ`eiviz.control.v1`です。subprotocolは`eiviz.protobuf.v1`です。

1. 接続先でtokenを入れる。LANから触るなら`bind`を`0.0.0.0:9400`など到達できるアドレスにする
2. `eiviz-headless run --session show.eivz`（または`eiviz-headless run`）を起動し、`ready ws=`を確認する
3. 操作するPCで`Eiviz.Remote.exe`（macOSは`eiviz-remote.app`）を起動する
4. 左上のConnectに、headless側のIP、ポート（既定9400）、同じtokenを入れてOKする

tokenはWindows Credential Manager/macOS Keychainに保存します。セッションファイルには入れません。Connectの▾から最近使った接続先を選べます。

複数クライアントが同時に接続できます。`eivizctl`とRemoteを並べても構いません。ライブ状態は購読で揃います。

PreviewとProgramのライブ映像は、接続先のNDIまたはOMT出力をRemoteのヘッダーから選びます。Multiviewは、そのレイアウト向けに有効なNDIまたはOMT出力が1本のときに出ます。画面の詳細は[リモート接続](/eiviz/ja/features/remote/)です。

## 環境変数

`headless/eiviz-headless.example.env`がひな形です。tokenをコマンドラインやセッションファイルに書かないでください。

| 変数 | 用途 |
| --- | --- |
| `EIVIZ_API_TOKEN` | サーバーのtoken |
| `EIVIZ_API_TOKEN_FILE` | サーバーのtokenをファイルから読む |
| `EIVIZ_API_REQUIRE_AUTH` | `0`以外で認証必須。未設定時はtokenがあれば必須 |
| `EIVIZ_API_ROLE` | サーバーの最大role |
| `EIVIZ_MEDIA_DIRECTORY` | アップロード保存先 |

tokenを回すときは環境変数または`eivizctl prefs set token`を差し替え、`eiviz-headless`を再起動します。

## 終了コード

| コード | 意味 |
| --- | --- |
| 2 | 引数またはファイル読込 |
| 3 | セッション検証 |
| 4 | GPU/runtime |
| 5 | bind |
| 6 | その他のruntime失敗 |

ログはstderrです。調査用の正規化JSONは`eiviz-headless canonicalize`です。通常Saveは`.eivz`（履歴入り・メディアなし）です。`eiviz-headless export`は`.eivzx`を書き、Still/Videoを同梱し履歴は含めません。`run`の読み込み時はファイルの隣の`*.media`へ展開します。GUIで書き出しを開くときは、メディアの展開先と作業用`.eivz`を尋ねます。RemoteのSaveと`eivizctl save`は、いまの`run`セッションファイルへ書き込みます。`eivizctl replace`では保存先は変わりません。

`eiviz-headless history --session show.eivz`はファイル内履歴を一覧します（index、unix ms、revision。新しい順、最大20件）。`eiviz-headless restore --session show.eivz --index N --output old.eivz`はその履歴を履歴なしの単体`.eivz`として書き出します。GUIの読み込みでは、履歴があるファイルを選ぶと最新版（既定）か特定のrevisionを選べます。最近使ったファイルとダブルクリックは最新版を開きます。
