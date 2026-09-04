---
title: "互換API（vMix HTTP & TCP/OBS WebSocket）"
description: vMix HTTP・TCPおよびOBS WebSocket互換API
---

eivizでは、従来のソフトウェアスイッチャー向けに開発された資産を活用するため、一部ソフトウェアスイッチャーと互換のAPIを備えています。

:::caution
互換APIは、互換先のAPIとの動作を保証するものではありません。データの欠落や破損など、完全に互換ではないデータも存在します。
:::

## vMix API (HTTP)

Mixerがプロセス内でHTTPサーバーを公開します。既定はすべてのインターフェイスのポート`8088`です。SettingsのWeb APIから有効化、ポート、BasicAuthを変えられます。ユーザー名とパスワードを空にすると認証は使いません。

| 項目 | 値 |
| --- | --- |
| エンドポイント | `GET /api` または `GET /API` |
| 状態取得 | クエリなし。`application/xml`でvMix形のXMLを返す |
| Function | `?Function=Fade&Duration=500` のようにクエリで実行し、成功時は同じXMLを返す |

Sceneを先に、そのあとInputをフラットなInputsとして並べます。SceneはBlank Input+レイヤーです。`preview`/`active`は選択中のMixing Unitのフラット番号です。追加のMixing Unitは`<mix>`です。

対応Functionは[Function Reference](/eiviz/ja/developers/function-reference/)です。未知のFunctionは404、引数不正は400を返します。vMixのように「存在するFunctionなら失敗しても成功」にはしません。

HTTPのアクセスログはMixerログとは別に`eiviz-mixer-http.log`へ出します。ポーリング用の素の`GET /api`は書きません。

## vMix API (TCP)

未実装です。組み込み向けのTCPは[#107](https://github.com/MikanseiLaboratory/eiviz/issues/107)で検討しています。

## OBS Websocket API

未実装です。[#79](https://github.com/MikanseiLaboratory/eiviz/issues/79)を参照してください。
