---
title: Outputs
description: 選んだソースの送出
---

選んだ映像を外へ送る出口です。Mixing UnitのProgramは内部の本線で、Outputはそれを（または別のソースを）ネットワークへ出す側です。  
ATEMの出力ルーティング、vMixのNDI/SRT出力に相当します。

## 追加とソース

[設定](/eiviz/ja/introduction/settings/)の出力から追加します。  
転送方式はOMTまたはNDIです。ソースはInput、Scene、MU Preview、MU Program、Multiviewから選択が可能です。  
音声はMaster, Headphone, 各Audio Aux、またはNone(音声なし)から選択可能です。  
Multiviewを映像ソースに選択した場合、音声の送出は出来ません。  
新規セッションの既定はMixing UnitのProgramです。

各出力の解像度とフレームレートは、設定のOutputsで個別に選べます。「セッション設定を使用」はMixing Unit（無ければセッションのマスターフレームレートと既定サイズ）を使います。映像は、描画が終わった瞬間ではなく、その出力のフレームレートで送ります。

Masterの合成、Mixing Unitの合成、各Outputは、共通の起点を持つ1つの有理数メディアクロックを共有します。期限とPTSは壁時計の加算ではなく、`index × interval`で都度算出します。遅れたOutputは取りこぼした枠を飛ばし、追いつきの連送はしません。Outputのフレームレートがソースより高いときは最新の完成フレームを繰り返し、低いときは古い完成フレームを捨てて最新を送ります。音声も同じ内容時刻とフレームバッファ遅延を使います。

各出力毎に1スレッド割り当てられます。1つのOutputのエンコードや通信待ちは、合成、音声スケジューラ、他のOutputを止めません。  
OMTとNDIは設定したOutputごとに1回エンコードします。同じOutputへ受信機が増えても再エンコードしません。同じ絵を指すOutputを10個作ると、エンコードは10系統です。  
Decklinkなどのハードウェア出力は現在実装中です。  
送出は[NDI/OMT](/eiviz/ja/features/outputs/ndi-omt/)をご確認ください。

## クロックのsoak

自動試験は23.976〜120fpsの枠数、59.94→50と59.94→25の間引き、29.97→59.94の繰り返し、映像と音声の共通起点を見ます。実機ではflash+toneをGPU OMT、CPU OMT、NDIで最低30分流し、A/Vずれを音声1パケットまたは出力1フレーム以内に保ちます。遅延や停止したOutputを混ぜても、他のOutputの歩調が維持されることを確認します。
