---
title: リモート接続
description: Eiviz.Remoteから別のeivizを操作する
---

Windows版の`Eiviz.Remote`やmacOS版の`eiviz-remote`を使用することで、ネットワーク経由で別マシンのeivizを遠隔操作できます。操作は接続先の`ControlService`を通じて行われます。通信プロトコルについては[eiviz API](/eiviz/ja/developers/api/)を参照してください。

## 接続手順

1. 接続先マシンでAPI受付を有効にします（GUIの環境設定、または[headless](/eiviz/ja/features/headless/)の`eiviz-headless run`）。
2. 接続用の認証tokenを設定します。
3. 操作側マシンで`Eiviz.Remote`（macOSは`eiviz-remote`）を起動します。
4. 画面左上のConnectボタンからIPアドレス、ポート番号、tokenを入力して接続します。

接続は信頼できるLANまたはVPN環境で行ってください。入力したtokenはOSの資格情報マネージャー（Windows Credential Manager/macOS Keychain）に安全に保存され、セッションファイルには含まれません。Connectボタン横のドロップダウンから最近接続したホストを再選択できます。

複数のクライアントから同時に接続できます。

## 映像の受信

プレビュー画面にはPreview/Programの2画面表示、またはMultiviewの1画面表示を選択できます。画面上部のヘッダーメニューから受信するNDIまたはOMTソースを選択してください。

OMT受信時のデコード方式（CPUまたはGPU）は、Remote側の環境設定（Preferences）で変更できます（既定: CPU推奨）。

## 設定の同期

設定ウィンドウでの操作（表示、パフォーマンス、出力、音声、Web API、Multiviewなど）は、接続先のセッションへ送信されて即座に反映されます。

言語やテーマ、OMT受信設定は各Remoteクライアント固有の設定です。

## セッションの保存

Saveボタンをクリックすると、接続先ホストで現在開いているセッションファイルに保存されます。

- headless環境では、起動時に指定したセッションファイル（未指定時は自動生成された日付付きファイル）へ保存されます。
- GUI環境で新規作成した未保存セッションの場合は、ホスト側で保存先ファイルが確定するまで保存できません（`UNAVAILABLE`が返されます）。

## メディアファイルの扱い

静止画や動画のInputを追加する場合、ファイルはクライアントから接続先ホストのメディア保存ディレクトリへ自動的に転送されます。

既定の保存先ディレクトリ:
- Windows: `%LOCALAPPDATA%\eiviz\media`
- macOS: `~/Library/Application Support/eiviz/media`
- Linux/headless: `$XDG_DATA_HOME/eiviz/media`（未設定時は`~/.local/share/eiviz/media`）
