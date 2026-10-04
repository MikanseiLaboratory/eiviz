---
title: "互換API（vMix HTTP & TCP/OBS WebSocket）"
description: vMix HTTP・TCPおよびOBS WebSocket互換API
---

eivizでは、既存のコントローラーや外部ツールを活用できるよう、一部のソフトウェアスイッチャーと互換性のあるAPIを提供しています。

:::caution
互換APIは完全な互換性を保証するものではありません。未対応のパラメータや動作の差異が含まれる場合があります。
:::

## vMix HTTP API

Mixerプロセス内でHTTPサーバーが動作します（既定: ポート`8088`）。設定ウィンドウの「Web API」から有効化、ポート番号、Basic認証を設定できます。

| 項目 | 内容 |
| --- | --- |
| エンドポイント | `GET /api` または `GET /API` |
| 状態取得 | クエリなし。vMix互換のXML（`application/xml`）を返却 |
| Function実行 | `GET /api?Function=Fade&Duration=500` のようにクエリで指定し、実行後にXMLを返却 |

XML出力では、Sceneに続いてInputがフラットな一覧として展開されます。対応しているFunctionの一覧は[Function Reference](/eiviz/ja/developers/function-reference/)を参照してください。未定義のFunctionには404、不正な引数には400を返します。なお、本家vMixとは異なり、既知のFunctionであっても処理に失敗した場合は成功として扱われません。

アクセスログは`eiviz-mixer-http.log`に出力されます（定期ポーリングの`GET /api`は除外されます）。

## vMix TCP API

ポート`8099`で[vMix TCP API](https://www.vmix.com/help29/TCPAPI.html)互換のテキストサーバーを提供します。[vmix-rs](https://github.com/MikanseiLaboratory/vmix-rs)やハードウェアコントロールパネルからの操作に対応しています。なお、HTTP用のBasic認証はTCPには適用されません。

通信はUTF-8、改行コードは`\r\n`です。

| コマンド | 動作 |
| --- | --- |
| `TALLY` | タリー状態の取得（各桁は入力順。0=オフ、1=Program、2=Preview） |
| `FUNCTION` | HTTPと同様のショートカット実行（例: `FUNCTION Fade Duration=500`） |
| `ACTS` | アクティベータ状態の取得（`Input`、`Preview`、`Overlay`等）。`Overlay1`から`Overlay8`はOn-Airの先頭8件に対応し、9件目以降はこのインターフェースでは取得できません |
| `XML` / `XMLTEXT` | 状態XMLの取得 |
| `SUBSCRIBE` / `UNSUBSCRIBE` | タリーやアクティベータ状態の変更通知を購読 |
| `VERSION` / `QUIT` | バージョン取得および接続切断 |

対応しているFunctionの一覧は[Function Reference](/eiviz/ja/developers/function-reference/)を参照してください。

## OBS WebSocket API

現在未実装です（[#79](https://github.com/MikanseiLaboratory/eiviz/issues/79)で検討中）。
