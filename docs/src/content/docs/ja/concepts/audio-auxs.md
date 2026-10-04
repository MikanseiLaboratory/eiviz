---
title: MU Bus
description: Mixing Unitごとの音声バスとヘッドホン
---

内部ミックスは48 kHzステレオです。MasterやAUXといった独立バスは存在せず、各[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)が専用のMU Busを1本持ちます。バスIDはMixing UnitのIDと共通です。

## MU Bus

MU Busは、そのMixing Unitの音声ミックスです。

- **Follow**: Preview/Programの切り替えとTバーに連動します。
- **Independent**: Tバーを無視し、Programの音声を常にミックスします。

出力デバイスは各Mixing Unitの設定で選択します。Noneを指定した場合は内部ミックスのまま保持されます。Inputは送り先のMixing Unitを複数選択できます。送り先が設定されていないInputは無音となります。なお、1つのInputを複数のハードウェアデバイスへ同時に出力することはできません。

## ヘッドホン

メーターのヘッドホンアイコンをクリックすると、そのMixing UnitまたはInputの音声をモニターできます。もう一度クリックすると停止します。出力デバイスは[設定](/eiviz/ja/introduction/settings/)の「ヘッドホン」で選択します。

## Mix Input

Mix Inputの音声は、参照先Mixing UnitのMU Busに追従します。セッションのMultiviewを参照している場合は無音になります。

詳細は[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)を参照してください。
