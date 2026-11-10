# eiviz ロゴ素材

eivizの公式ロゴ一式です。採用版として確定したデータをそのまま格納しています。
再発注・差し替えの際は、このディレクトリごと交換してください。

## クレジット

ロゴ: Guleruun (https://twitter.com/guleruun)

ロゴを公開・掲載する際は、この表記でクレジットを入れてください。
`README.md`、`README.ja.md` の Credits 節にも同じ表記を入れています。

## 構成

| パス | 内容 |
| --- | --- |
| `eiviz-final-master.ai` | Illustratorマスター（ベクター原本） |
| `eiviz-final-overview.png` | ロゴシステム全体のサムネイル |
| `png/` | 書き出し済みPNG 12枚 |

`png/` は「配色」×「フレーム」×「レイアウト」の組み合わせです。

| 項目 | 値 |
| --- | --- |
| 配色 | `07-I1`（ダーク向け） / `07-I3`（ライト向け） |
| フレーム | `framed`（背景にフレームあり） / `frameless`（なし） |
| レイアウト | `icon`（アイコン単体） / `text-bottom`（テキスト下） / `text-right`（テキスト右） |

ファイル名は `eiviz-07-I1-framed-text-right.png` の形式です。

## 用途ごとの使い分け

| 用途 | 使うファイル |
| --- | --- |
| README（ダーク背景のGitHub） | `png/eiviz-07-I1-frameless-text-right.png` |
| docs のナビゲーションロゴ | `docs/src/assets/logo-{dark,light}.png` |
| favicon | `docs/public/favicon{,-dark}.png` |
| OGP / Social Preview | `docs/public/og.png` |

- `logo-dark.png` / `favicon-dark.png` は `07-I1`（ダーク向け）、`logo-light.png` / `favicon.png` は `07-I3`（ライト向け）から作っています。
- `logo-*.png` はナビゲーションの高さに合わせて余白を詰め、`favicon*.png` は小さいサイズでも読めるよう余白をほぼ削っています。
- `og.png` は1200x630で、`07-I1` のフレームあり・テキスト右を暗い背景の中央に置いています。

これらはこのディレクトリのPNGから書き出した派生ファイルです。原本を差し替えた場合は派生ファイルも再生成してください。
