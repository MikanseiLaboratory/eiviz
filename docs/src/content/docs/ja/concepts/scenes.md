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

Sceneにも複数のタグを付けられます。タグはセッションのカタログに残り、どのSceneにも付いていないタグもタブとして出ます。

### 付け方

Scene Editorでタグをチェックして付けます。同じ画面から新しいタグを追加できます。  
1つのSceneに複数付けられます。

### 一覧の絞り込み

メインウィンドウのScenes一覧の上に、排他のタブがあります。

- **すべて** — 全Scene
- **各タグ** — そのタグが付いたSceneだけ

タブ切替は一覧の表示だけを絞ります。Overlay、Multiview、設定の選択対象は全件のままです。  
別ウィンドウのMixing Unitスイッチャーにも、同じタグタブがあります。

### タグの管理

タブ帯を右クリックして、タグの追加、名前変更、削除ができます。

- 改名すると、付いているSceneも新しい名前に追従します
- 削除すると、各Sceneから外れます。表示中のタブを消した場合は「すべて」に戻ります

### タイルの折り畳み

名前の帯を右クリックすると、高さはそのままで横幅だけ狭くなり、隣が左に詰まります。サムネの読み戻しは止まります。折り畳み時はクリックでPreview、ダブルクリックで設定です。保存したセッションを開き直しても折り畳みは残ります。

シーン一覧とスイッチャーのサムネはGPUから読み戻した絵です。シーンを増やしても映像出力先ウィンドウは増えません。本線のPreview/Programだけがリアルタイム表示の枠を使います。
