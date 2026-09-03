---
title: Scenes
description: Inputを重ねてつくる合成
---

<img src="/eiviz/images/ja/concepts/scenes.jpg" alt="Scenesの概念図" style="max-width: 100%; height: auto;" />


複数のInputをレイヤーとして重ねた合成です。

## 編集

Scenesから追加し、編集で使用するInputと位置、サイズ、不透明度、重なり順を決めます。  
レイヤーのAudio Followで、そのInputの音を絵に追従させられます。

## 使い先

Mixing Unitのバス、[Overlay](/eiviz/ja/concepts/overlays/)のソース、[Multiview](/eiviz/ja/concepts/multiviews/)のタイル、Outputのソースとして使います。

## タグとタイル

Sceneにも複数のタグを付けられます。一覧上部のタブでAllと各タグを切り替えます。  
名前の帯を右クリックするとプレビューを折り畳み、負荷を下げられます。折り畳み時はクリックでPreview、ダブルクリックで設定です。

シーン一覧とスイッチャーのサムネはGPUから読み戻した絵です。シーンを増やしても映像出力先ウィンドウは増えません。本線のPreview/Programだけがリアルタイム表示の枠を使います。
