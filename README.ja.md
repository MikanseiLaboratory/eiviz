# eiviz

[![Test Build](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/ci.yml)
[![Publish Release](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml/badge.svg)](https://github.com/MikanseiLaboratory/eiviz/actions/workflows/release.yml)

**現在鋭意開発中です。安定していな部分や未実装の機能が数多く存在します。ご理解ください**

## eiviz

映像オペレーターが作った、次世代のプロダクション向けグラフィックオペレーションツール・映像スイッチャーソフトウェアです。  
PCの性能が許す限り無限にM/Eを追加可能で、既存のソフトウェアスイッチャーを凌駕する圧倒的な自由度を提供します。  

eivizは、最新のモダンな技術アーキテクチャを採用することにより、既存のツールやソフトウェアでは実現が難しかった機能やクロスプラットフォーム対応を実現し、高い機能性と拡張性を提供しています。  
また、既存のソフトウェアスイッチャーの扱いづらさや反省を基に、より扱いやすく、アマチュア配信からプロの現場まで幅広く活躍できるソフトウェアとしてデザインされています。

eivizは特定の企業ではなく、メインメンテナとコミュニティのサポートにより開発・運営されています。

[ドキュメント](https://mikanseilaboratory.github.io/eiviz/ja/) · [English](README.md)

<img width="1916" height="1030" alt="eiviz" src="https://github.com/user-attachments/assets/7b2f30c0-7870-49d7-9fdc-369da2e10ef4" />

| 環境 | 描画 | 状態 |
| --- | --- | --- |
| Windows x64 | Direct3D 12またはVulkan | 対応済み |
| macOS | Metal | 対応済み(未テスト) |
| Linux | Vulkan | headlessのみ |

## インストール

[最新版ダウンロード](https://github.com/MikanseiLaboratory/eiviz/releases/latest) · [過去のバージョン](https://github.com/MikanseiLaboratory/eiviz/releases)

### Windows x64

`eiviz-*-win-x64-setup.exe`を実行します。

### macOS Apple Silicon（`macos-arm64`）

`eiviz-*-macos-arm64.pkg`を実行します。

### macOS Intel（`macos-x64`）

手順は同じです。`macos-x64`のpkgを使います。


### Linux 

現在CLI(headless)モードのみ対応しています。ソースコードをビルドし、`eiviz-headless`を実行してください。手順は[headless](https://mikanseilaboratory.github.io/eiviz/ja/features/headless/)です。
パフォーマンスの観点から、Releaseビルドで実行することを強く推奨します。

## 開発のAI利用について

本ツールは開発にAI/LLMを使用しています。

Vibe codingアプリケーションではなく、開発の要所・必要なタスクの支援にツールとして活用しており、LLMが開発を主導したものではありません。

ソフトウェア本体のアーキテクチャや実装はコアメンテナが責任を持って行い、実装コードもメンテナがレビューを通過したもののみ採用しています。

## ライセンス

[PolyForm Shield License 1.0.0](LICENSE)です。営利利用・社内利用を含め制限無く利用が可能です。

有償・無償を問わず、eivizの競合となる映像スイッチャーを開発・販売を禁止します。河村柊吾/未完成成果物研究所がこのソースコードを利用して出している他の製品についても同じ条件が適用されます。競合製品を出したい場合は、別途ライセンス契約が必要です。詳細はお問い合わせください。

同梱の第三者ライブラリは、それぞれのMIT、Apache-2.0、Zlibのままです。[NOTICE](NOTICE)と[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)を見てください。
