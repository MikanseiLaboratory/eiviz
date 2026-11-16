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
| `app-icon/` | アプリアイコン（`.ico` / `.icns` と生成スクリプト） |

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
| アプリアイコン（Windows / macOS） | `app-icon/windows/eiviz.ico` / `app-icon/macos/eiviz.icns` |

- `logo-dark.png` / `favicon-dark.png` は `07-I1`（ダーク向け）、`logo-light.png` / `favicon.png` は `07-I3`（ライト向け）から作っています。
- `logo-*.png` はナビゲーションの高さに合わせて余白を詰め、`favicon*.png` は小さいサイズでも読めるよう余白をほぼ削っています。
- `og.png` は1200x630で、`07-I1` のフレームあり・テキスト右を暗い背景の中央に置いています。

これらはこのディレクトリのPNGから書き出した派生ファイルです。原本を差し替えた場合は派生ファイルも再生成してください。

## アプリアイコン

`07-I3`（ライト向け）のフレームありアイコンを元にしています。ライト／ダークどちらのOSテーマでも読めるため、フレームありを使っています。

```powershell
pwsh -File docs/src/assets/branding/app-icon/Generate-AppIcons.ps1
```

`app-icon/windows/eiviz.ico`（16〜256pxの9サイズ）と `app-icon/macos/eiviz.iconset/` を生成します。
macOS上で実行すると `iconutil` も呼び、`app-icon/macos/eiviz.icns` まで生成します。

macOS以外で `.icns` を作り直す場合:

```powershell
pwsh -File docs/src/assets/branding/app-icon/New-Icns.ps1
```

`.iconset` のPNGを `icns` コンテナに詰め直します（`iconutil` の代替。生成物はApple製ツールのものと完全一致はしません）。

### 参照箇所

| プラットフォーム | 設定 |
| --- | --- |
| Windows（アプリ） | `hosts/win32/Eiviz.Host.csproj` / `Eiviz.Remote.csproj` の `ApplicationIcon` |
| Windows（インストーラー） | `packaging/windows/eiviz.iss` の `SetupIconFile` とショートカットの `IconFilename` |
| macOS | `hosts/macos/Sources/EivizMac/Info*.plist` の `CFBundleIconFile` と、`hosts/macos/package-app.sh` の `Contents/Resources/eiviz.icns` へのコピー |
