---
title: NDI / OMT Capture
description: NDIおよびOMTからの映像入力
---

ネットワーク上のNDIまたはOMTソースをInputとして取り込みます。追加手順は[Inputs](/eiviz/ja/concepts/inputs/)と[設定](/eiviz/ja/introduction/settings/)です。Remote側の受信設定は[リモート接続](/eiviz/ja/features/remote/)です。

## 転送とデコード

NDIはCPUで受信し、合成のためにGPUへアップロードします。

OMTはデコード経路を選べます。既定はCPU(推奨)です。

CPU(推奨)はOMTデフォルトのデコーダーです。カメラ入力など、ミッションクリティカルな映像にはこちらを使用してください。CPU decodeはフレームをシステムメモリへコピーします。

GPUは補助的なOMTデコーダーです。GPUに負荷を逃がす役割がありますが、CPUよりロスが多い為マルチビューなど補助的な用途の映像に使用してください。GPU decodeはVMXフレームをGPU上に保ちます。

保存済みセッションの明示的なGPU設定は維持します。新規Inputの既定だけがCPUです。

## 品質とバッファ

フレームバッファ(1–8)はこのソースのジッタを吸収します。Qualityは送信側へ要求します。未使用ソースも接続したままPreview(1/8 decode)を要求します。帯域節約とMultiview時のフル品質維持は、WindowsのInput設定から選べます。
