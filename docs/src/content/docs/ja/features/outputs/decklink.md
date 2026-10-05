---
title: Decklink
description: Decklinkへの出力
---

DeckLinkの入出力はProモジュール限定の機能です。Free版では`ERR_NOT_SUPPORTED_PLAN`を返します。ホストUIは`mixer_capabilities`でモジュールがリンクされており、現在のプランで許可されている場合のみDeckLink出力を選択肢に表示します。

公式のPro版パッケージには署名済みProモジュールと`eiviz-pro.required`が同梱されています。OSS（公開）ビルドには同モジュールは含まれません。
