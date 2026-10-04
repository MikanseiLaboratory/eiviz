---
title: Audio Auxs
description: Master、Headphone、AUXバス
---

eiviz内部のオーディオミキサーは48 kHzステレオで動作します。

## バス構成

- **Masterバス**: 配信や録音用のメイン音声バス（削除不可）
- **Headphoneバス**: オペレーターのモニター用バス（Masterのコピー、または個別キュー出力）
- **AUXバス（A〜H）**: 最大8系統まで追加可能な個別送出用バス。ミックスマイナスや同時通訳、個別送出（ISO）に利用できます。

## Mixing Unitとの連動

各Mixing Unitは音声バスへのセンド設定を持ちます。

- **Follow**: Preview/Programの切り替えやTバーのフェード動作に連動して音声が切り替わります（Audio Follow）。
- **Independent**: 映像の切り替え状態に関わらず、割り当てられた入力を常に一定のレベルでミックスします。

## 出力デバイスの割り当て

[設定](/eiviz/ja/introduction/settings/)ウィンドウの「音声AUX」から、各バスを実際のオーディオインターフェース（WASAPI共有またはASIO）へルーティングできます。物理デバイスに出力せず内部ミックスのみを有効化することも可能です。詳細は[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)を参照してください。
