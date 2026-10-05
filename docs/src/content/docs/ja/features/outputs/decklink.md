---
title: Decklink
description: Decklinkへの出力
---

DeckLinkの入出力はProモジュールの機能です。Freeビルドでは`ERR_NOT_SUPPORTED_PLAN`を返します。ホストは`mixer_capabilities`がリンク済みで、かつプランが許可しているときだけ、この出力種別を表示します。

公式ビルドは`pro/eiviz_pro`を非公開モジュールで置き換えます。公開ツリーの同じパスはFree固定のスタブで、`mixer/Cargo.toml`は変わりません。
