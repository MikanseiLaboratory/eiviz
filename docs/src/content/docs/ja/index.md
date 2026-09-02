---
title: eiviz
description: eiviz公式ドキュメント
---

**eiviz**（ˈeɪvɪz）は、[未完成成果物研究所](https://mikanseilaboratory.github.io/)および河村 柊吾/[FlowingSPDG](https://github.com/FlowingSPDG)が開発・保守するソフトウェアスイッチャーです。  
WindowsとmacOSに対応しています。Linuxは実験的です。

このサイトでは、使い方、開発目的、現場での運用までをまとめています。

## はじめに

- [eivizについて](/eiviz/ja/introduction/about/) — 開発動機と技術選定
- [未完成成果物研究所](/eiviz/ja/introduction/mikansei-laboratory/) — 開発コミュニティ
- [システム要件](/eiviz/ja/introduction/requirements/) — 対応環境とハードウェア
- [設定](/eiviz/ja/introduction/settings/) — 設定ウィンドウの項目
- [システムアーキテクチャ](/eiviz/ja/introduction/architecture/) — MixerとUIの構成

## 概念

- [Inputs](/eiviz/ja/concepts/inputs/) — 映像ソースと、Input・Scene・Mixing Unit・Outputの関係
- [Scenes](/eiviz/ja/concepts/scenes/) — Inputを重ねてつくる合成
- [Mixing Unit](/eiviz/ja/concepts/mixing-unit/) — PreviewとProgramで切り替える単位
- [Audio Auxs](/eiviz/ja/concepts/audio-auxs/) — Master、Headphone、AUXバス
- [Outputs](/eiviz/ja/concepts/outputs/) — 選んだソースの送出
- [Multiviews](/eiviz/ja/concepts/multiviews/) — 監視用のモザイク
- [Overlays](/eiviz/ja/concepts/overlays/) — Mixing UnitのProgramに載せるDSK

## 各種機能

- [UVC Capture](/eiviz/ja/features/inputs/uvc/) — UVCデバイスからの映像入力
- [NDI/OMT Capture](/eiviz/ja/features/inputs/ndi-omt/) — NDIおよびOMTからの映像入力
- [動画、静止画](/eiviz/ja/features/inputs/media/) — 動画ファイルと静止画の入力
- [Colour](/eiviz/ja/features/inputs/colour/) — カラー入力
- [映像の合成](/eiviz/ja/features/compositing/) — Sceneの合成と複数レイヤー
- [NDI/OMT](/eiviz/ja/features/outputs/ndi-omt/) — NDIおよびOMTへの出力
- [Decklink](/eiviz/ja/features/outputs/decklink/) — Decklinkへの出力
- [音声、ASIOなど](/eiviz/ja/features/outputs/audio/) — 音声出力とASIO
- [Vision Mixing](/eiviz/ja/features/vision-mixing/) — Mixing Unitを使った多段M/E

## 開発者向け情報

- [互換API（vMix HTTP & TCP/OBS WebSocket）](/eiviz/ja/developers/compatibility/) — vMix HTTP・TCPおよびOBS WebSocket互換API
- [新規API](/eiviz/ja/developers/api/) — eiviz固有のAPI
- [Function Reference](/eiviz/ja/developers/function-reference/) — eivizの関数リファレンス
