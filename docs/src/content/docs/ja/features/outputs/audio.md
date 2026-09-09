---
title: 音声、ASIOなど
description: 音声出力とAudio Input
---

内部ミックスは48 kHzステレオです。出力バスの割当は[設定](/eiviz/ja/introduction/settings/)の音声AUXです。

## モニター

下部の音声バーでは、各InputのメーターはPost（フェーダーとミュートの後）です。ストリップのダブルクリックか歯車で、そのInputの音声ウィンドウを開きます。ウィンドウにはPreとPostがあり、フェーダーはそこにあります。

## Audio Input

マイクや出力デバイスのループバックは、既存のInput一覧に`Audio`として追加します。別コレクションは作りません。Audio Inputは常時バスへ送られ、SceneのAudio Follow対象にはなりません。Sceneレイヤーや映像ピッカーにも出しません。

Windowsのマイク、出力ループバック、WASAPI再生はcpalのWASAPIホスト（共有モード）です。Application Audioはrsacのプロセスループバック（f32/48 kHz/ステレオ、PCM自動変換なし）です。デバイスを空にすると、その時点の既定デバイスを追従します。出力はWASAPI共有かASIOです。Application Audioは表示中のウィンドウがあるプロセスだけを列挙します。プロセス取り込みが失敗してもエンドポイントループバックへは落ちません。ASIO入力は出力バスと同じドライバインスタンスを共有し、LとRに任意の入力チャンネルを割り当てます。macOSのCore Audio再生とマイク／ループバック取り込みはcpalです。Core Audioのプロセスループバックは未実装で、選んだ場合は明示的に失敗します。

## OMT送出

ProgramなどのOMT出力は、映像エンコードとは別にPCMを送ります。遅いVMXエンコードで10 msの音声が詰まることはありません。
