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

Web APIが有効なとき、Mixerは[vMix TCP API](https://www.vmix.com/help29/TCPAPI.html)互換のテキストサーバーをポート`8099`で開きます。Iryxなど[vmix-rs](https://github.com/MikanseiLaboratory/vmix-rs)系のパネルはこちらを使います。HTTPのBasicAuthはTCPには掛かりません。TCPの待ち受けに失敗してもHTTPは継続し、警告だけ出します。

UTF-8、行末は`\r\n`です。応答は`<command> <status> ...\r\n`で、`XML`は続けて本文を長さちょうど返します。クライアントは応答を待ってから次のコマンドを送ります。`SUBSCRIBE`中のイベントはいつでも届きます。

| コマンド | 動作 |
| --- | --- |
| `TALLY` | `TALLY OK 0121...`。桁はフラットInput順。0=オフ、1=Program、2=Preview。両方ならProgram |
| `FUNCTION` | HTTPと同じShortcut。例:`FUNCTION Fade Duration=500`。成功は`FUNCTION OK Completed` |
| `ACTS` | `Input`/`InputPreview`/`InputMix2`以降/`Overlay1`–`8`。引数なしは現在の割当。未対応のアクティベータは0 |
| `XML` / `XMLTEXT` | HTTPと同じXML。XPathは`vmix/version`、`preview`、`active`、`vmix/inputs/input[N]/@title`などのサブセット |
| `SUBSCRIBE` / `UNSUBSCRIBE` | `TALLY`と`ACTS`の変化をpush。Iryxはこれを使います |
| `VERSION` / `QUIT` | 版の取得と切断 |

対応Functionは[Function Reference](/eiviz/ja/developers/function-reference/)です。

## OBS Websocket API

未実装です。[#79](https://github.com/MikanseiLaboratory/eiviz/issues/79)を参照してください。
