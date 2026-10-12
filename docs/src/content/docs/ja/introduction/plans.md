---
title: プラン
description: Community、Professional、Enterpriseで使える機能と上限
---

eivizのプランはCommunity、Professional、Enterpriseの3つです。合成、NDI/OMT、音声、リモート接続はプランで分かれません。分かれるのはMixing Unitの本数、DeckLink、RTMPの映像サイズとフレームレート、および録画/SRT/ハードウェアエンコードの能力フラグです。

## 上限

幅、高さ、フレームレートは別々に判定します。どれか1つでも上限を超えると拒否します。フレームレートは`fps_num/fps_den`の値で比較し、上限ちょうども許可します。

| | Community | Professional | Enterprise |
| --- | --- | --- | --- |
| Mixing Unit | 4 | 無制限 | 無制限 |
| DeckLink入力 | なし | 4 | 無制限 |
| DeckLink出力 | なし | 1 | 無制限 |
| RTMPの幅 | 1280 | 1920 | 無制限 |
| RTMPの高さ | 720 | 1080 | 無制限 |
| RTMPのフレームレート | 30fps | 59.94fps | 無制限 |
| 録画/SRT/ハードウェアエンコード | なし | 能力フラグのみ（未実装） | 能力フラグのみ（未実装） |

Professionalの59.94fpsは`60000/1001`です。`60/1`の60fpsは、解像度が1920×1080以下でも上限を超えます。Communityの30fpsは`30/1`と`30000/1001`を含みます。縦長の1080×1920は、Professionalでは高さが1080を超えるため拒否します。

同じIDのMixing Unitを置き換える操作は、本数の上限を新たに消費しません。

## プランの決まり方

有効なライセンスチケットに書かれたプランが使われます。チケットの導入と状態表示はホストが行い、判定はProモジュールが行います。

チケットが未登録、期限切れ、署名不正、別のマシン向けのときはCommunityとして扱います。期限の基準は、OSの現在時刻と保存された最終確認時刻のうち進んでいる方です。時計を戻しても期限は延びません。期限切れのあと、すでに動いている送出はすぐには止まりません。新しく上限を超える操作は拒否します。

モジュールを含まない公開ビルドは、Communityの上限だけをMixerに内蔵しています。DeckLinkとRTMPの実装はリンクされないため、プラン内の解像度でも`ERR_IO`になります。プラン外の呼び出しは、実装の有無より先に`ERR_NOT_SUPPORTED_PLAN`を返します。`eiviz-pro.required`がある公式パッケージは、モジュールの欠落や署名不正、ABI不一致では起動せず、Communityへは落ちません。読み込みの詳細は[システムアーキテクチャ](/eiviz/ja/introduction/architecture/)を参照してください。

開発用の`pro-dev`付きモジュールだけが、環境変数`EIVIZ_PLAN`をチケットより優先します。値は`community`、`professional`（`pro`も同じ）、`enterprise`です。このフィーチャーの無いReleaseビルドは環境変数を無視します。
