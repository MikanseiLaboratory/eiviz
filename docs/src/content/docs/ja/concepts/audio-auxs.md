---
title: Audio Auxs
description: Master、Headphone、AUXバス
---

内部ミックスは48 kHzステレオです。MasterとHeadphoneは最初からあり、消せません。  
AUXはA〜Hまで最大8本です。コンソールのAUX、ミックスマイナス、ISO送りに相当します。

## Mixing Unitからの送り

[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)は、どのバスへ送るかを持ちます。  
FollowはPreview/ProgramとTバーのmixに追従します。Independentは映像の切替を無視し、そのバスに割り当てたInputをそのまま混ぜます。

## デバイスへの割当

実機への割当は[設定](/eiviz/ja/introduction/settings/)の音声AUXです。  
Enabledを選択時は、出力デバイスを設定せずに内部でのミックス処理のみ動かします。  
「HeadphoneはMasterをコピー」を入れると、HeadphoneバスはMasterと同じ中身になります。

デバイス側の詳細は[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)をご参照ください。
