---
title: Overlays
description: Mixing UnitのProgramに載せるDSK
---

Overlayは、一般的なスイッチャーにおけるDSK（ダウンストリームキー）に相当し、Program映像の上にテロップやロゴ、ワイプ画面などを重ねて合成する機能です。

Overlayの定義はセッション共通です。On-Airリストは[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)ごとに独立しており、件数の上限はありません。ソースは[Scene](/eiviz/ja/concepts/scenes/)またはInputです。トランジションプリセットもセッション共通で、効果時間は実行したMixing Unitのフレームレートに基づいてフレーム数に換算されます。vMixの`Overlay1`〜`Overlay8`およびvMix XMLのOverlay一覧には、そのMixing UnitでOn-Airとなっている先頭8件が割り当てられます。9件目以降はvMix互換インターフェースには反映されません。

## 操作と設定

- **配置とサイズ**: メインウィンドウのOverlay設定から、画面上の表示位置（X/Y座標）やサイズを指定します。
- **切り替え効果**: CutまたはFadeによるトランジションに対応しています。
- **オンエア制御**: スイッチャー画面のトグルボタンで即座にON/OFFが可能です。

※OverlayウィンドウはProgramをリアルタイム表示するため、映像出力スロットを1つ消費します（ウィンドウを閉じると解放されます）。
