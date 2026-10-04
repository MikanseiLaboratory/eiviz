---
title: Mixing Unit
description: PreviewとProgramでSceneを切り替える単位
---

Mixing Unitは、一般的なスイッチャーにおけるM/E（Mix/Effects）に相当する映像切り替えの基本単位です。vMixのMix Input、TriCasterやATEMのM/E、Kairosのシーンに該当します。

Preview（スタンバイ映像）とProgram（本線出力映像）の2系統のバスを持ち、CUTやAUTO、Tバー操作によって映像を切り替えます。

## 主な特徴

- **無制限の追加**: PCスペックの許す限り、1つのセッション内に複数のMixing Unitを自由に追加できます。
- **独立した解像度・フレームレート**: ユニットごとに個別の解像度や出力フレームレートを設定可能です。
- **Overlay（DSK）**: [Overlay](/eiviz/ja/concepts/overlays/)の定義はセッション共通です。On-Airリストはユニットごとに独立しており、件数の上限はありません。
- **M/Eの多段構成（入れ子）**: あるMixing Unitの出力を「Mix Input」として別のMixing Unitの入力レイヤーに配置することで、多段M/E構成を構築できます（同一ユニットへの循環参照は禁止されています）。

※Switcherウィンドウを開くと、PreviewとProgramの表示用に映像出力スロットを2つ消費します。ウィンドウを閉じるとスロットは解放されます。
