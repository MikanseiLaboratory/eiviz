# 開発用スクリプト

リポジトリの検証を手で並べると順序を間違えるので、その手順をここに置いている。
すべて実ソフトウェアを操作する（UI Automation と Web API）。

## 前提

- Windows、eiviz の作業ツリー。
- Pro モジュール（`eiviz-pro`）が同じ親ディレクトリにあること。
- 実機テストでは、`D:\replay\test1` に書き込めること。

## build-and-run.ps1

鍵つきのミキサを組み、ホストを組み、出力先に配置して起動する。

`dotnet build` はワークスペースの target からミキサをコピーするが、それは Pro モジュールの
公開鍵なしで組んだものなので、ホストが起動できなくなる。鍵つきで組み直してから出力先へ
上書きする必要があり、手で並べていると順序を間違える。この 1 本にまとめてある。

```powershell
.\scripts\build-and-run.ps1
```

## replay-test-preset.ps1

開いている eiviz を、リプレイのテストが始まる状態まで持っていく。

- 入力に OBS の OMT 出力を加える（`OBS PGM`）。
- リプレイ設定にドライブを 1 本、Scene 1 を撮るアングルを 1 つ、自動追加を有効にする。
- プログラムを Scene 1（SMPTE バー）にする。
- クリップの追加先を「末尾」にする。

何度実行してもよい（すでに設定済みの項目は触らない）。

```powershell
.\scripts\replay-test-preset.ps1
```

OBS 側は OMT 出力が有効で、`OBS Output` という名前で広告されている必要がある。

## replay-two-channel.ps1

Replay A と B の 2 チャンネルを用意し、それぞれのプレイリストにクリップを作り、
B で再生して独立していることを確かめる。

```powershell
.\scripts\replay-two-channel.ps1
```

## replay-export-mode.ps1

書き出しウィンドウを開いた状態で、指定のモードで 1 本書き出す。録画中の確認
ダイアログを確実に処理するので、古いモーダルが残っていてもキー入力を飲まれない。

```powershell
# 0 = 選択中のクリップ, 1 = プレイリスト全体（トランジション込み）, 2 = カットのみ,
# 3 = このアングル, 4 = 全アングル
.\scripts\replay-export-mode.ps1 -ModeIndex 1 -Path "D:\replay\test1\export.mp4"
```

## check-rustfmt.py / check-session-dto.py

CI と同じ検査を手元で回す。

```powershell
python scripts/check-rustfmt.py
python scripts/check-session-dto.py
```
