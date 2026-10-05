---
title: Decklink
description: Decklinkへの出力
---

DeckLinkの入出力はProモジュールの機能です。Freeビルドでは`ERR_NOT_SUPPORTED_PLAN`を返します。ホストは`mixer_capabilities`がリンク済みで、かつプランが許可しているときだけ、この出力種別を表示します。

公式のProパッケージは署名済みモジュールと`eiviz-pro.required`を同梱します。公開ビルドにはそのモジュールは含まれません。
