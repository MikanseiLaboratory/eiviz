---
title: Decklink
description: Decklinkへの出力
---

DeckLink入出力はProモジュール専用の機能です。Communityプランでは`ERR_NOT_SUPPORTED_PLAN`が返されます。ホストUIは`mixer_capabilities`でモジュールが読み込まれており、かつ有効なプランで許可されている場合のみDeckLink出力を選択肢に表示します。

macOSはDeckLinkの対応ホストではありません。BlackmagicのmacOSヘッダーが`decklink-sys`の期待する`IID_IUnknown`を宣言していないため、macOS版ProビルドはDeckLink shimを含めていません。DeckLink入出力にはWindowsを使用してください。macOSパッケージにもRTMP・Replay・ライセンス用の署名済みProモジュールは同梱されます。

公式のProパッケージには署名済みProモジュールと`eiviz-pro.required`が同梱されています。OSS公開ビルドには同モジュールは含まれません。利用可能な入出力数は[プラン](/eiviz/ja/introduction/plans/)を参照してください。
