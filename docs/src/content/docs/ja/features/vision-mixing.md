---
title: Vision Mixing
description: Mixing Unitを使った多段M/E映像スイッチング
---

eivizでは、他スイッチャーではMix EffectやM/Eと呼ばれる映像合成ユニットの単位を**Mixing Unit**と呼称します。
これは実質的にvMixでのMix InputやPanasonic Kairosでのシーン、Viz/NewTek TriCasterやBlackmagic DesignのM/Eに該当します。
最も大きな変更点として、セッションごとのMixing Unitには上限がなくPC性能が許す限りユーザーの責務で無限に追加することが可能です。  

## Buses

それぞれのMixing Unitには、**Preview**バスx1、**Program**バスx1、**Multiview**バスx16の合計18バスが存在します。
Previewバスはスイッチャー運用時にシーンの映像出力を確認する用途です。 Programバスは実際に出力される映像出力本体です。 内部的にPreview/Programバスには実装の際はありません。  
Multiview Busは複数のInput、SceneやPreview/Programをアサインすることにより、一つの映像中で複数の映像フィードの監視と確認が可能です。 Multiviewは16バスまで追加可能で、それぞれ違うタイルレウアウト、Inputのアサイン・設定が可能です。

## Input/Output re-route

PreviewとProgramにはSceneとInputを載せられます。OutputのソースはInput、Scene、MU Preview、MU Program、Multiviewです。流れは[Inputs](/concepts/inputs/)です。
