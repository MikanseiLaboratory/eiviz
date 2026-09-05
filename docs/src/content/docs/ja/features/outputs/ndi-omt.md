---
title: NDI / OMT
description: NDIおよびOMTへの出力
---

選んだソースをネットワークへ出す出口です。追加とソースの選び方は[Outputs](/eiviz/ja/concepts/outputs/)と[設定](/eiviz/ja/introduction/settings/)の出力です。

## 転送とエンコード

転送方式はOMTまたはNDIです。

OMTはエンコード経路を選べます。GPU encodeはフレームをGPUに載せたままVMXへ変換します。CPU encodeを選択した場合、UYVY形式で読み出し、CPU上の送出専用スレッドでVMXコーデックへの変換・送信を行います。  
NDIは常にCPU経路です。

各出力毎に1スレッド割り当てられます。

OMT出力は、受信機が0個のときVMXエンコードを止めることができます。既定はオンです。設定のOutputsで「OMT受信機が0個の場合にエンコードをスキップする」をオフにすると、購読が無いときもエンコードし続けます。NDIはこの設定を使いません。

## 音声

音声はMaster, Headphone, 各Audio Aux、またはNone(音声なし)から選択可能です。  
Multiviewを映像ソースに選択した場合、音声の送出は出来ません。

デバイスへのミックスは[音声、ASIOなど](/eiviz/ja/features/outputs/audio/)です。ネットワーク送出のPCMは、その内部ミックスから取ります。
