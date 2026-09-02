---
title: "互換API（vMix HTTP & TCP/OBS WebSocket）"
description: vMix HTTP・TCPおよびOBS WebSocket互換API
---

eivizでは、従来のソフトウェアスイッチャー向けに開発された資産を活用するため、一部ソフトウェアスイッチャーと互換のAPIを備えています。

:::caution
現在実装中の機能です。
:::

## vMix API (HTTP/TCP)

### HTTP API

vMix互換のXMLデータを返すHTTP APIです。  
Shortcut Functionにも対応しており、vMixで扱われる主要なShortcutを使用可能です。  

XMLデータはvMixと同形式に変換するため、Input,Scenesを全てフラットなInputsとして扱っています。 ScenesはBlankインプット+レイヤー構成 としてXMLに混ぜ込まれます。  

### TCP API

vMix TCP API互換のAPIです。
FUNCTION, ACTSの送受信に対応しており、eiviz側のイベント更新やフック処理をvMix TCP APIと同形式で送信します。  
組み込み環境など、メモリが少ない環境での動作を想定しています。  

## OBS Websocket API

OBS-Studio Websocket API互換のAPIです。

OBS Websocketと同一の認証システムと、主要なイベントデータ、eivizの制御が可能です。  

:::caution
この互換APIは、互換先のAPIとの動作を保証するものではありません。データの欠落や破損など、完全に互換ではないデータも存在します。
:::