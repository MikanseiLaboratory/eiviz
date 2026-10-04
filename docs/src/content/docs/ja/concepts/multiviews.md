---
title: Multiviews
description: 複数ソースを一枚に並べた監視用モザイク
---

<img src="/eiviz/images/ja/concepts/multiview.jpg" alt="Multiviews例" style="max-width: 100%; height: auto;" />

Multiviewは、複数の入力や出力映像を1つの画面に並べて監視できる機能です。セッション内に無制限に追加でき、個別のウィンドウとして表示したり、ネットワーク（NDI/OMT）へ送出したりできます。

## レイアウトとタイル設定

- **レイアウトテンプレート**: Preview+Programと周辺グリッドの構成や、2×2、3×3、4×4などの定型グリッドを選択できます。
- **タイルへのソース割り当て**: 各タイルにはInput、Scene、Mixing UnitのPreview/Program出力を自由に配置できます。

## 設定とパフォーマンス管理

- **追加と編集**: [設定](/eiviz/ja/introduction/settings/)ウィンドウまたはメイン画面右下のMultiviewメニューから追加・カスタマイズできます。
- **更新レート調整**: タイルの更新頻度（毎フレーム〜数フレームおき）を調整可能です。マシンスペックが限られる環境では更新間隔を広げることでGPU負荷を軽減できます。
- **ウィンドウ枠の消費**: Multiviewウィンドウを開くたびに映像出力スロットを1つ消費します（ウィンドウを閉じると解放されます）。
