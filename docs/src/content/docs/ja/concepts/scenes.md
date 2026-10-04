---
title: Scenes
description: Inputを重ねてつくる合成
---

<img src="/eiviz/images/ja/concepts/scenes.jpg" alt="Scenesの概念図" style="max-width: 100%; height: auto;" />

Sceneは、複数のInput（カメラ、動画、画像、テロップ等）をレイヤーとして重ね合わせて1つの画面を構成する機能です。OBS Studioのシーンに相当します。

## 構成と編集

メイン画面のScenesから追加・編集します。各レイヤーについて以下の設定が可能です。

- 位置（X/Y座標）、サイズ、回転、不透明度、クロップ
- 重なり順（前後関係）
- **Audio Follow**: 映像が選択された際に、そのInputに紐づく音声も自動的に追従・出力する設定

## 主な用途

作成したSceneは、以下の各機能のソースとして幅広く利用できます。

- Mixing UnitのPreview/Programへの割り当て
- [Overlay](/eiviz/ja/concepts/overlays/)のテロップ・グラフィックソース
- [Multiview](/eiviz/ja/concepts/multiviews/)の監視タイル
- [Outputs](/eiviz/ja/concepts/outputs/)を通じた外部ネットワークへの直接送出

## タグ機能と整理

Sceneには任意のタグを複数付与でき、一覧上部のタブからタグ別に素早く絞り込めます。

- **タグの付与**: Scene Editor内のチェックボックスから選択または新規作成します。
- **タグの管理**: タブ領域を右クリックすることで、タグの追加・名称変更・削除が可能です。
- **タイルの折り畳み**: Sceneタイルのヘッダーを右クリックするとコンパクト表示に切り替えられます。折り畳み中はサムネイル更新が停止し、描画負荷を抑制できます。
