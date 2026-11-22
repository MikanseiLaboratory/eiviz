---
title: eiviz
description: クロスプラットフォームの無制限M/Eソフトウェアスイッチャー。公式ドキュメント。
---

**eiviz**（ˈeɪvɪz）は、[未完成成果物研究所](https://mikanseilaboratory.github.io/)および河村 柊吾/[FlowingSPDG](https://github.com/FlowingSPDG)が開発・保守するソフトウェアスイッチャーです。  
WindowsとmacOSに対応しています。Linux対応は実験的です。

このサイトでは、使い方、開発動機、現場での運用までをまとめています。

## はじめに

- [eivizについて](/ja/introduction/about/) — 開発動機と技術選定
- [未完成成果物研究所](/ja/introduction/mikansei-laboratory/) — 開発コミュニティ
- [システム要件](/ja/introduction/requirements/) — 対応環境とハードウェア
- [ハードウェア固有の制約](/ja/introduction/hardware-limits/) — エンコーダー同時数、デコード資源、帯域
- [プラン](/ja/introduction/plans/) — Community、Professional、Enterpriseの上限
- [設定](/ja/introduction/settings/) — 設定ウィンドウの項目
- [システムアーキテクチャ](/ja/introduction/architecture/) — MixerとUIの構成

## 概念

- [Inputs](/ja/concepts/inputs/) — 映像ソースと、Input・Scene・Mixing Unit・Outputの関係
- [Scenes](/ja/concepts/scenes/) — Inputを重ねてつくる合成
- [Mixing Unit](/ja/concepts/mixing-unit/) — PreviewとProgramで切り替える単位
- [MU Bus](/ja/concepts/audio-auxs/) — Mixing Unitごとの音声バスとヘッドホン
- [Outputs](/ja/concepts/outputs/) — 選んだソースの送出
- [Multiviews](/ja/concepts/multiviews/) — 監視用のモザイク
- [Overlays](/ja/concepts/overlays/) — Mixing UnitのProgramに載せるDSK

## 各種機能

- [UVC Capture](/ja/features/inputs/uvc/) — UVCデバイスからの映像入力
- [NDI/OMT Capture](/ja/features/inputs/ndi-omt/) — NDIおよびOMTからの映像入力
- [動画、静止画](/ja/features/inputs/media/) — 動画ファイルと静止画の入力
- [Colour](/ja/features/inputs/colour/) — カラー入力
- [映像の合成](/ja/features/compositing/) — Sceneの合成と複数レイヤー
- [NDI/OMT](/ja/features/outputs/ndi-omt/) — NDIおよびOMTへの出力
- [DeckLink](/ja/features/outputs/decklink/) — DeckLinkへの出力
- [音声、ASIOなど](/ja/features/outputs/audio/) — 音声出力とASIO
- [Vision Mixing](/ja/features/vision-mixing/) — Mixing Unitを使った多段M/E
- [リモート接続](/ja/features/remote/) — 別のeivizをGUIから操作する
- [headless](/ja/features/headless/) — GUIなしのMixerをeivizctlとRemoteから操作する

## 開発者向け情報

- [互換API（vMix HTTP & TCP/OBS WebSocket）](/ja/developers/compatibility/) — vMix HTTP・TCPおよびOBS WebSocket互換API
- [eiviz API](/ja/developers/api/) — eiviz固有のProtobuf制御API
- [Function Reference](/ja/developers/function-reference/) — eivizの関数リファレンス
