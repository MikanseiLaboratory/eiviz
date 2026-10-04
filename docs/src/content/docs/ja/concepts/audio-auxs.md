---
title: MU Bus
description: Mixing Unitごとの音声バスとヘッドホン
---

内部ミックスは48 kHzステレオです。MasterとAUXはありません。各[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)が専用のMU Busを1本持ちます。バスIDはMixing UnitのIDと同じです。

## MU Bus

MU Busは、そのMixing Unitの音声ミックスです。

- **Follow**: Preview/Programの切り替えとTバーに連動します。
- **Independent**: Tバーを無視し、Programの音声を常にミックスします。

出力デバイスはMixing Unitの設定で選びます。Noneは内部ミックスのままです。入力は送り先のMixing Unitを複数選べます。送り先のない入力は無音です。1つの入力を複数のデバイスへ同時には出せません。

## ヘッドホン

メーターのヘッドホンアイコンを押すと、そのMixing UnitまたはInputの音が出ます。もう一度押すと止まります。[設定](/eiviz/ja/introduction/settings/)の「ヘッドホン」で出力デバイスを選びます。

## Mix Input

Mix Inputの音声は、参照先Mixing UnitのMU Busに固定です。セッションMultiviewを参照する場合は無音です。

詳細は[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)を参照してください。
