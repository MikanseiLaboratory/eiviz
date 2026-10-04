---
title: Scenes
description: Inputを重ねてつくる合成
---

<img src="/eiviz/images/ja/concepts/scenes.jpg" alt="Scenesの概念図" style="max-width: 100%; height: auto;" />

Sceneは、複数のInput（カメラ、動画、画像、テロップなど）をレイヤーとして重ね合わせて1つの画面を構成する機能です。OBS Studioのシーンに相当します。

## 構成と編集

メイン画面のScenesから追加・編集します。各レイヤーについて以下の設定が可能です。

- 位置（X/Y座標）、サイズ、回転、不透明度、クロップ
- 重なり順（前後関係）
- **Audio Follow**: 映像の選択・切り替えに連動し、そのレイヤーのInputに紐づく音声も自動的に追従して出力する設定

## 主な用途

作成したSceneは、以下の各機能のソースとして幅広く利用できます。

- Mixing UnitのPreview/Programへの割り当て
- [Overlay](/eiviz/ja/concepts/overlays/)のテロップ・グラフィックソース
- [Multiview](/eiviz/ja/concepts/multiviews/)の監視タイル
- [Outputs](/eiviz/ja/concepts/outputs/)を通じた外部ネットワークへの直接送出

## タグ機能と整理

Sceneには任意のタグを複数付与でき、一覧上部のタブからタグ別に素早く絞り込めます。

- **タグの付与**: Scene Editor内のチェックボックスで指定します。同ダイアログから新規タグを追加することも可能です。1つのSceneに複数のタグを設定できます。
- **タグの管理**: タブ領域を右クリックすることで、タグの追加、名前変更、削除が可能です。
- **タイルの折り畳み**: Sceneタイルのヘッダー（名前バー）を右クリックすると横幅が縮小され、コンパクト表示に切り替えられます。折り畳み中はサムネイルの読み戻しが停止し、描画負荷を軽減できます。折り畳み時もクリックでPreview選択、ダブルクリックで設定画面を開けます。
