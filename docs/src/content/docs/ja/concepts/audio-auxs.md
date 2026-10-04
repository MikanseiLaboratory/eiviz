---
title: MU Bus
description: Mixing Unitごとの音声バスとヘッドホン
---

内部ミックスは48 kHzステレオです。MasterとAUXはありません。各[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)が専用のMU Busを1本持ちます。バスのIDはMixing UnitのIDと同じです。

## MU Bus

MU Busは、そのMixing Unitのミックスです。

- **Follow**: Preview/Programの切り替えとTバーに連動します。
- **Independent**: Tバーを無視し、Programの音声を常にミックスします。

出力デバイスはMixing Unitの設定で選びます。Noneは内部ミックスのままです。入力は、送る先のMixing Unitを複数選べます。未設定の入力は無音です。1つの入力を複数のデバイスへ同時に出すことはできません。

## ヘッドホン

ヘッドホンは、選択中のMixing Unitをcueする特別なバスです。[設定](/eiviz/ja/introduction/settings/)の「ヘッドホン」で出力デバイスを選びます。「ヘッドホンはcue中のMU Busをコピー」をオンにすると、そのミックスをそのまま出します。オフのときは、cue対象をヘッドホン用に別にミックスします。ローカルのモニター出力は、cue中のMU Busです。

## Mix Input

Mix Inputの音声は、参照先Mixing UnitのMU Busに固定されます。セッションMultiviewを参照する場合は無音です。

詳細は[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)を参照してください。
