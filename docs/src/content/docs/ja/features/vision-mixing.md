---
title: Vision Mixing
description: Mixing Unitを使った多段M/E映像スイッチング
---

eivizのスイッチング操作は、M/E（Mix/Effects）に相当する**Mixing Unit**を中心に行われます。

## バス構成とスイッチング

各Mixing Unitは**Preview**（待機画面）と**Program**（本線出力）の2系統のバスを備えています。

- **Sceneの選択**: スイッチャー画面のボタンをクリックして、Previewに次のシーン（[Scene](/eiviz/ja/concepts/scenes/)）をスタンバイします。
- **トランジション実行**: CUT、AUTOボタン、またはTバーのスライダー操作によって、Previewの映像をProgramへ切り替えます。
- **トランジション効果**: WGSLシェーダーによる多彩なトランジション（Cut、Fade、Wipe等）が用意されており、継続時間（ミリ秒）やイージングを指定可能です。

## OverlayとMultiview

- **[Overlay](/eiviz/ja/concepts/overlays/)**: 各Mixing UnitのProgram出力に対し、最大8系統のDSK（テロップやPinP）を重ねて合成できます。
- **[Multiviews](/eiviz/ja/concepts/multiviews/)**: 複数の入力や各ユニットのPreview/Programを一覧表示するマルチビュー画面を構築し、外部ディスプレイや別ウィンドウで常時監視できます。

## 多段M/E（リentrant構成）

あるMixing Unitの出力を「Mix Input」として別のMixing Unitの入力レイヤーに配置することで、ATEMやKairosのような多段M/E構成を容易に実現できます。
