# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**開発中です。まだ安定していません。**

クロスプラットフォームの無制限M/Eソフトウェアスイッチャーです。名前は映像(eizou)とvisualから取っています。

[ドキュメント](https://mikanseilaboratory.github.io/eiviz/ja/) · [English](README.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| 環境 | 描画 | 状態 |
| --- | --- | --- |
| Windows x64 | Direct3D 12 | 使えます |
| macOS | Metal | 対応済み（未テスト） |
| Linux | Vulkan | 開発中 |

## インストール

[最新版](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [過去のリリース](https://github.com/MikanseiLaboratory/eiviz/releases)

### Windows x64

`eiviz-*-win-x64-setup.exe`を実行します。  
zipの場合は展開して`Eiviz.Host.exe`を実行します。

### macOS Apple Silicon（`macos-arm64`）

`eiviz-*-macos-arm64.pkg`を実行すると、`/Applications`に入ります。

zipの場合は展開してから次を実行します。

```bash
cd eiviz-*-macos-arm64
xattr -cr .
open eiviz-mac.app
```

NDI®の発見と送出には`.app`が必要です。macOSにブロックされたときは、システム設定→プライバシーとセキュリティ→このまま開く、です。

### macOS Intel（`macos-x64`）

手順は同じです。`macos-x64`のpkgかzipを使います。

## eivizとは

映像オペレーターが映像オペレーションのために作った、次世代のプロダクション向けグラフィックオペレーションツールです。

PreviewとProgramを持つMixing Unitで、CUT、AUTO、Tバーから映像を切り替えます。オーバーレイとマルチビューも使えます。

入力はカラー、バー、静止画、動画ファイル、UVC、OMT、NDI®。出力はOMTとNDI®です。音声はWindowsとmacOSで使えます。

Windowsで保存したセッションは、macOSでも同じ内容で開きます。

## ライセンス

本体は[PolyForm Shield License 1.0.0](LICENSE)です。社内で使う分には、営利組織でも構いません。

有償・無償を問わず、eivizの代わりになる映像スイッチャーを出荷してはいけません。河村柊吾/未完成成果物研究所がこのソフトを使って出している他の製品についても同じです。競合する製品を出したい場合は、別途ライセンスが必要です。

同梱の第三者ライブラリは、それぞれのMIT、Apache-2.0、Zlibのままです。[NOTICE](NOTICE)と[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)を見てください。
