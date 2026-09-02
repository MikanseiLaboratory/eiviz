---
title: Inputs
description: 映像ソースと、Input・Scene・Outputの関係
---

他ソフトのInput、Source、カメラ入力に相当します。eivizでは**Input → Scene → Mixing Unit → Output**の順で扱います。切替そのものはMixing Unitです。

```mermaid
flowchart LR
  In["Input"] --> Sc["Scene"]
  Sc --> MU["Mixing Unit"]
  MU --> Out["Output"]
```

| | 役割 |
| --- | --- |
| Input | カメラ、ファイル、NDI/OMT、カラーなどの映像ソース |
| Scene | Inputをレイヤーとして重ねた合成 |
| Mixing Unit | SceneをPreview/Programに載せ、CUT/AUTO/Tバーで切替 |
| Output | 選んだソースをNDI/OMTへ送出 |

OutputのソースはInput、Scene、Mixing UnitのPreview/Program、Multiviewから選べます。既定はMixing UnitのProgramです。InputはMixing UnitとOutputにも直接載せられます。

## 種類

メインウィンドウのInputsから追加します。

- カラー/バー/ブラック
- 静止画
- 動画ファイル
- UVC（キャプチャデバイス）
- NDI/OMT

Sceneのレイヤー、Mixing Unitのバス、Multiviewのタイル、Outputのソースとして使います。

[Scenes](/eiviz/ja/concepts/scenes/) · [Mixing Unit](/eiviz/ja/concepts/mixing-unit/) · [Outputs](/eiviz/ja/concepts/outputs/)
