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
