# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**活発に開発中/不安定。**  
マルチM/Eの映像スイッチャーです。eivizは映像(eizou)+visualです。

[ドキュメント](https://mikanseilaboratory.github.io/eiviz/ja/) · [English](README.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| プラットフォーム | グラフィックス | 状態 |
| --- | --- | --- |
| Windows x64 | Direct3D 12 | 対応 |
| macOS | Metal | Tier 2 |
| Linux | Vulkan | 開発中 |

## インストール

[Download](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [すべてのリリース](https://github.com/MikanseiLaboratory/eiviz/releases)

**Windows x64:** zipを展開して`Eiviz.Host.exe`を実行します。  
**macOS Apple Silicon**（`macos-arm64`）: zipを展開して`./eiviz-mac`を実行します。  
**macOS Intel**（`macos-x64`）: 同じ手順です。x86_64/amd64向けです。

## できること

ライブ映像のミックス、スイッチ、オーバーレイ、送出です。各Mixing UnitにPreviewとProgramがあり、CUT、AUTO、Tバーで切り替えます。

Windowsで保存したセッションはmacOSでも同じ内容で開き、その逆も同様です。

入力: カラー、バー、静止画、動画ファイル、UVC、OMT、NDI®。  
出力: OMT、NDI®。  
シーン、オーバーレイ、マルチビュー。音声はWindowsとmacOSに対応しています。

## ライセンス

eiviz本体のソースは[PolyForm Shield License 1.0.0](LICENSE)です。

営利組織を含む内部利用は許可されています。eivizとの競合は許可されていません。このソフトウェアの実用的な代替となる映像スイッチャー（有償・無償を問わない）や、河村柊吾/未完成成果物研究所がこれを使って提供する他製品の代替を出荷することです。それには河村柊吾/未完成成果物研究所からの別ライセンスが必要です。

第三者のクレートとライブラリは、元のMIT/Apache-2.0/Zlibのままです。[NOTICE](NOTICE)と[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)をご参照ください。
