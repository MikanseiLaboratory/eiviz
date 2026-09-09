---
title: 音声、ASIOなど
description: 音声出力とAudio Input
---

内部ミックスは48 kHzステレオです。出力バスの割当は[設定](/eiviz/ja/introduction/settings/)の音声AUXです。

## Audio Input

マイクや出力デバイスのループバックは、既存のInput一覧に`Audio`として追加します。別コレクションは作りません。Audio Inputは常時バスへ送られ、SceneのAudio Follow対象にはなりません。Sceneレイヤーや映像ピッカーにも出しません。

WindowsではWASAPI共有モードで取り込みます。デバイスを空にすると、その時点の既定デバイスを追従します。排他モードが失敗しても共有へ黙って落ちません。プロセスループバックとASIO入力、macOSのCore Audio入力は未実装で、選んだ場合は明示的に失敗します。

## OMT送出

ProgramなどのOMT出力は、映像エンコードとは別にPCMを送ります。遅いVMXエンコードで10 msの音声が詰まることはありません。
