---
title: Overlays
description: Mixing UnitのProgramに載せるDSK
---

Overlayは、一般的なスイッチャーにおけるDSK（ダウンストリームキー）に相当し、Program映像の上にテロップやロゴ、ワイプ画面などを重ねて合成する機能です。

定義はセッション共通です。On-Airは[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)ごとに独立し、件数の上限はありません。ソースは[Scene](/eiviz/ja/concepts/scenes/)またはInputです。Transitionのプリセットもセッション共通で、長さは実行したMixing Unitのフレームレートでフレーム数に換算します。vMixの`Overlay1`から`Overlay8`と、vMix XMLのOverlay一覧は、そのMixing UnitのOn-Air先頭8件です。9件目以降はvMix互換の面に出ません。

## 操作と設定

- **配置とサイズ**: メインウィンドウのOverlay設定から、画面上の表示位置（X/Y座標）やサイズを指定します。
- **切り替え効果**: CutまたはFadeによるトランジションに対応しています。
- **オンエア制御**: スイッチャー画面のトグルボタンで即座にON/OFFが可能です。

※Overlayプレビューウィンドウを表示する場合、映像出力スロットを1つ消費します（ウィンドウを閉じると解放されます）。
