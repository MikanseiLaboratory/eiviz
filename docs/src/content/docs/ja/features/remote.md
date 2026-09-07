---
title: リモート接続
description: Eiviz.Remoteから別のeivizを操作する
---

Windowsは`Eiviz.Host.exe`（Mixer）と`Eiviz.Remote.exe`（操作クライアント）に分かれます。macOSは`eiviz-mac.app`と`eiviz-remote.app`です。ライブ操作とセッション編集は、接続先の`ControlService`が担当しています。プロトコルは[eiviz API](/eiviz/ja/developers/api/)をご確認ください。

## 接続

1. 接続先でAPI待ち受けを有効にする（`Eiviz.Host.exe`の環境設定、または`eiviz-headless run --bind`）
2. 待ち受けtokenを設定する
3. `Eiviz.Remote.exe`（macOSは`eiviz-remote.app`）を起動し、左上のConnectを押す
4. WebSocket URLとtokenを入れてOKする

接続は信頼できるLANまたはVPN上の認証付き`ws://`です。tokenはWindows Credential Manager/macOS Keychainに保存します。セッションJSONには入れません。ホスト側の待ち受け項目は[設定](/eiviz/ja/introduction/settings/)の環境設定をご確認ください。

Connectの▾から最近使ったURLを選べます。

複数クライアントが同時に接続でき、ライブ状態は購読で揃います。

## 映像

PreviewとProgramはHostと同じPRV/PGMの枠です。ヘッダーのメニューから受信するNDIまたはOMTソースを選びます。Multiviewは、接続先でそのレイアウト向けに有効なNDIまたはOMT出力が1本のときにライブ映像が出ます。Input PreviewはHost向けです。

シーン一覧とスイッチャーのシーンボタンは、折り畳んだ状態ですべてのSceneを出します。サムネイルはHost向けです。

Still/Videoの追加は、クライアントでファイルを選び、接続先のメディア保存先へ保存したうえでInputを足します。

## 設定

表示、パフォーマンス、出力、音声、Web APIは接続先の設定ウィンドウで確認します。Multiviewの追加・開く・タイル編集・削除はクライアントから送ります。言語とテーマは`Eiviz.Remote.exe`自身の環境設定です。

セッションの変更は`MutateSession`と`expected_revision`です。revisionが一致しない変更は拒否され、最新を読み直してやり直します。

## メディア保存先

アップロードしたStill/Videoの保存先はホスト側です。未指定時は次のディレクトリです。

- Windows: `%LOCALAPPDATA%\eiviz\media`
- macOS: `~/Library/Application Support/eiviz/media`
- Linux/headless: `$XDG_DATA_HOME/eiviz/media`、未設定なら`~/.local/share/eiviz/media`

ホストの環境設定または`--media-directory`/`EIVIZ_MEDIA_DIRECTORY`で変更できます。
