---
title: Vision Mixing
description: Mixing Unitを使った多段M/E映像スイッチング
---

eivizでは、他スイッチャーでMix EffectやM/Eと呼ばれる映像合成の単位を**Mixing Unit**と呼びます。  
vMixのMix Input、Panasonic Kairosのシーン、Viz/NewTek TriCasterやBlackmagic DesignのM/Eに相当します。

セッションごとのMixing Unitに上限はありません。PC性能が許す限り追加できます。  
解像度と出力フレームレートはユニットごとに持てます。セッション全体の時計は[設定](/eiviz/ja/introduction/settings/)のマスターフレームレートです。

## バス

それぞれのMixing Unitには、切替用の映像バスが2本あります。**Preview**と**Program**です。

Previewは次の絵を確認する用途です。Programは実際に出力される本線です。  
スイッチャー画面からPreviewへ載せられるのは[Scene](/eiviz/ja/concepts/scenes/)です。CUT、AUTO、TバーでPreviewをProgramへ入れ替えます。

### Overlay

[Overlay](/eiviz/ja/concepts/overlays/)はMixing Unitごとに最大8本です。ソースはSceneまたはInputです。  
CUTやTバーで混ざったあとのProgramに載ります。Previewバスには載りません。

### Multiview

監視用のモザイクはMixing Unitのバスではなく、セッション単位の[Multiviews](/eiviz/ja/concepts/multiviews/)です。  
Mixing Unitと同じく本数に上限はなく、[設定](/eiviz/ja/introduction/settings/)からPC性能が許す限り追加できます。タイルにはInput、Scene、MU Preview、MU Programを置けます。

## Transition

映像の切り替え時にはTransitionを使用可能です。  
Transitionは複数種類選択可能であり、WGSLを使った合成処理をしています。  
使用するアニメーション、フレーム補完方法を選択し、実行することでProgramの映像が切り替わります。

## 出力との関係

Outputに使用可能な映像ソースはInput、Scene、Mixing UnitのPreview・Program、Multiviewです。既定はMixing UnitのProgramです。流れは[Inputs](/eiviz/ja/concepts/inputs/)をご参照ください。
