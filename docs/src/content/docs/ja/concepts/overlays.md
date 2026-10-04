---
title: Overlays
description: Mixing UnitのProgramに載せるDSK
---

Overlayは、一般的なスイッチャーにおけるDSK（ダウンストリームキー）に相当し、Program映像の上にテロップやロゴ、ワイプ画面などを重ねて合成する機能です。

定義はセッション共通です。On-Airは[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)ごとに独立し、件数の上限はありません。ソースには[Scene](/eiviz/ja/concepts/scenes/)またはInputを指定できます。Transitionのプリセットもセッション共通です。フレーム数の長さは、実行したMixing Unitのフレームレートで換算します。vMixの`Overlay1`から`Overlay8`、およびvMix XMLのOverlay一覧は、そのMixing UnitでOn-Airの先頭8件に対応します。9件目以降はvMix互換の面には出ません。

## 操作と設定

- **配置とサイズ**: メインウィンドウのOverlay設定から、画面上の表示位置（X/Y座標）やサイズを指定します。
- **切り替え効果**: CutまたはFadeによるトランジションに対応しています。
- **オンエア制御**: スイッチャー画面のトグルボタンで即座にON/OFFが可能です。

※Overlayプレビューウィンドウを表示する場合、映像出力スロットを1つ消費します（ウィンドウを閉じると解放されます）。
