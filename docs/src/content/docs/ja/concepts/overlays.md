---
title: Overlays
description: Mixing UnitのProgramに載せるDSK
---

Overlayは、一般的なスイッチャーにおけるDSK（ダウンストリームキー）に相当し、Program映像の上にテロップやロゴ、ワイプ画面などを重ねて合成する機能です。

各[Mixing Unit](/eiviz/ja/concepts/mixing-unit/)ごとに最大8系統まで設定でき、ソースには[Scene](/eiviz/ja/concepts/scenes/)またはInputを指定できます。

## 操作と設定

- **配置とサイズ**: メインウィンドウのOverlay設定から、画面上の表示位置（X/Y座標）やサイズを指定します。
- **切り替え効果**: CutまたはFadeによるトランジションに対応しています。
- **オンエア制御**: スイッチャー画面のトグルボタンで即座にON/OFFが可能です。

※Overlayプレビューウィンドウを表示する場合、映像出力スロットを1つ消費します（ウィンドウを閉じると解放されます）。
