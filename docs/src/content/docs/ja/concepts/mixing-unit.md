---
title: Mixing Unit
description: PreviewとProgramでSceneを切り替える単位
---

他スイッチャーのM/Eに該当する機能です。vMixのMix Input、TriCasterやATEMのM/E、Kairosのシーンに相当します。  
メインの映像スイッチングオペレーションを担う根幹機能です。 Previewに次の絵、Programに現在使用する映像が表示されます。

セッションあたりの数に上限はありません。マシンの性能が許す限り上限なしで追加可能です。  
解像度と出力フレームレートはユニットごとに持てます。  

[Overlay](/eiviz/ja/concepts/overlays/)はこのユニットのProgramに載ります。あるユニットのProgramを、別ユニットの入力やOutputのソースにもできます。

切替用の映像バスはPreviewとProgramです。  
Switcher UIはPreviewとProgramを表示するので、映像出力先ウィンドウを2スロット使います。閉じると枠が空きます。

Add Inputから、このユニットのPreviewまたはProgramをMix Inputとして追加できます。既存のFrameDelayリング（1–8フレーム）を参照し、swapchainは増やしません。別のMixing Unitに載せると入れ子のM/Eになります。同じユニットへの自己配線は拒否されます。音声は対象のAudio BusかNoneを指定します。
