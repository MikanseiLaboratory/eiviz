---
title: NDI / OMT
description: NDIおよびOMTへの出力
---

選んだソースをネットワークへ出す出口です。追加とソースの選び方は[Outputs](/eiviz/ja/concepts/outputs/)と[設定](/eiviz/ja/introduction/settings/)の出力です。

## 転送とエンコード

転送方式はOMTまたはNDIです。

OMTはエンコード経路を選べます。GPU encodeはフレームをGPUに載せたままVMXへ変換します。CPU encodeはUYVYへパックして読み戻し、送出スレッドでVMXへ圧縮します。PCMは映像の直後に同じ送出スレッドへ載せ、次の映像まで持ち越しません。  
NDIは常にCPU経路です。UYVYをSDKへ渡し、送信スレッドはエンコード完了を待ちません。送信の進みは映像クロックに合わせ、PCMは別スレッドへ載せます。

各出力は自分の送出スレッドを持ちます。一方のエンコード待ちが、他方の接続待ちや音声を止めません。

## 音声

行の音声はMasterかNoneです。既定はMasterです。  
Multiviewをソースにした出力は無音です。モザイクのエンコードとPCMを同じ経路に乗せません。

デバイスへのミックスは[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)です。ネットワーク送出のPCMは、その内部ミックスから取ります。
