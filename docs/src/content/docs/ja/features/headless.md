---
title: headless
description: GUIなしでMixerを動かし、eivizctlとRemoteから操作する
---

`eiviz-headless`は、GUIを持たずにMixerをバックグラウンドで実行するデーモンです。セッションファイル（`.eivz`または`.eivzx`）を読み込んで映像合成を行い、制御用のProtobuf WebSocketを開きます（既定: loopbackポート9400）。`run`実行時に`--session`を省略した場合は、OSの`eiviz/sessions`ディレクトリ配下に日付付きの既定ファイルが自動作成され、そのファイルが使用されます。

Linuxではheadlessのみサポートされています。WindowsやmacOSの配布パッケージにも同じバイナリが同梱されています。操作はローカルの`eivizctl`や、別マシンの`Eiviz.Remote`（macOSは`eiviz-remote`）から行います。通信プロトコルについては[eiviz API](/eiviz/ja/developers/api/)、Remoteの画面操作については[リモート接続](/eiviz/ja/features/remote/)を参照してください。

初めて利用する場合は、まず「起動」と「Remoteから接続する」の手順に沿って接続を確認してください。

## 主な仕様

映像合成、Input/Output処理、セッション機能はGUI版と同等です。ただし以下の点が異なります。

- 制御インターフェースはProtobuf WebSocketのみで、vMix互換HTTP/TCPは提供されません。
- GUIプレビュー画面は表示されません。映像を確認する場合は、NDIやOMT出力を有効にし、Remote側の画面から対象ソースを受信してください。

## 起動

配布バイナリを利用するか、ソースコードからビルドします。

```bash
cargo build -p eiviz-headless --locked --release --bins
```

バイナリは`target/release/eiviz-headless`および`target/release/eivizctl`に生成されます。

```bash
eiviz-headless validate --session show.eivz
eiviz-headless canonicalize --session show.eivz
eiviz-headless export --session show.eivz --output show-portable.eivzx
eiviz-headless history --session show.eivz
eiviz-headless restore --session show.eivz --index 0 --output old.eivz
eiviz-headless run --session show.eivz --bind 127.0.0.1:9400
eiviz-headless run --bind 127.0.0.1:9400 --renderer auto
```

- `validate`と`canonicalize`: GPUを初期化せず、セッションの検証や正規化を行います。
- `run`: セッションを読み込んでランタイムを初期化し、WebSocketの受付を開始します。準備が完了するとstderrに`ready ws=`が出力されます。Ctrl+C（UnixではSIGTERM）またはAPIの`shutdown`コマンドで安全に終了します。
- `--bind`: 指定がない場合は設定ファイル（prefs）の値、設定ファイルにも指定がない場合は`127.0.0.1:9400`を使用します。loopback以外へbindする場合はtokenの設定が必須です。暗号化（WSS/TLS）には対応していないため、信頼できるLANやVPN環境で運用してください。

## 設定（prefs）

ホスト固有の接続設定は、セッションファイルではなくローカルの設定ファイルで管理されます。

| キー | 説明 |
| --- | --- |
| `bind` | WebSocketの受付アドレス（例: `127.0.0.1:9400`、外部許可時は`0.0.0.0:9400`） |
| `token` | 接続認証用token（値の確認時は`(set)`と表示） |
| `mediaDirectory` | Remoteからアップロードされたメディアの保存先 |
| `maxRole` | 付与する最大権限（`read`/`operate`/`configure`/`admin`） |
| `renderer` | GPUバックエンド（`auto`/`dx12`/`vulkan`/`metal`。OS非対応の値はエラー） |

設定ファイルの保存先:
- Windows: `%LOCALAPPDATA%\eiviz\headless-prefs.json`
- macOS/Linux: `$XDG_CONFIG_HOME/eiviz/headless-prefs.json`（未設定時は`~/.config/eiviz/headless-prefs.json`）

設定の優先順位（左側の指定が優先されます）:
- bind: `--bind` → 設定ファイルの`bind` → `127.0.0.1:9400`
- token: 設定ファイルの`token`のみ
- renderer: `--renderer` → 設定ファイルの`renderer` → `auto`
- メディア保存先: `--media-directory`または`EIVIZ_MEDIA_DIRECTORY` → 設定ファイル → 各OSの既定ディレクトリ
- 最大role: 設定ファイルの`maxRole` → `admin`

既定のメディア保存先:
- Windows: `%LOCALAPPDATA%\eiviz\media`
- macOS: `~/Library/Application Support/eiviz/media`
- Linux: `$XDG_DATA_HOME/eiviz/media`（未設定時は`~/.local/share/eiviz/media`）

## CLIツール（eivizctl）

`eivizctl`はheadlessの操作や設定を行うCLIツールです。通常実行と、`--repl`オプションによる対話モードに対応しています。

```bash
# 通常実行
eivizctl --url ws://127.0.0.1:9400 --token YOUR_TOKEN cut --unit 1

# 対話モード（REPL）
eivizctl --repl --url ws://127.0.0.1:9400 --token YOUR_TOKEN
```

### 設定ファイルの変更（prefs）

デーモンの起動前でも設定変更が可能です。

```bash
eivizctl prefs
eivizctl prefs get bind
eivizctl prefs set bind 0.0.0.0:9400
eivizctl prefs set token YOUR_TOKEN
eivizctl prefs set renderer dx12
```

### ライブ操作

デーモン起動中は、CLIからスイッチングやセッション操作を実行できます。

```text
eiviz> status
eiviz> input list
eiviz> input add --name Cam --kind Uvc
eiviz> mix preview --unit 1 --scene 2
eiviz> mix cut --unit 1
eiviz> mix auto --unit 1 --duration-ms 1000
eiviz> session save
eiviz> shutdown
```

## Remoteから接続する

1. headless側の設定でtokenを指定し、外部から接続する場合は`bind`を`0.0.0.0:9400`等に変更します。
2. `eiviz-headless run`を起動し、待機状態にします。
3. クライアントPCで`Eiviz.Remote`（macOSは`eiviz-remote`）を起動します。
4. 画面左上のConnectボタンをクリックし、headlessのIPアドレス、ポート番号（既定: 9400）、tokenを入力して接続します。

複数のクライアントから同時に接続できます。また、`eivizctl`とRemoteの併用にも対応しています。

映像プレビューを確認する場合は、headless側でNDIまたはOMT出力を有効にし、Remote側のヘッダーメニューから該当出力を選択してください。詳細は[リモート接続](/eiviz/ja/features/remote/)を参照してください。

## 終了コード

| コード | 意味 |
| --- | --- |
| 2 | 引数またはファイル読み込みエラー |
| 3 | セッション検証エラー |
| 4 | GPU/ランタイム初期化エラー |
| 5 | bind失敗 |
| 6 | その他ランタイムエラー |
