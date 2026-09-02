---
name: ja-docs-no-latin-spaces
description: >-
  Japanese docs must not put half-width spaces around Latin/ASCII words
  (和欧間スペース禁止). Use when writing or editing Japanese Markdown under
  docs/src/content/docs/ja, about.md, index.md, Starlight docs, 半角スペース,
  OS/Linux/Windows の前後スペース, or Japanese technical writing.
---

# 日本語ドキュメント: 和欧間スペース禁止

日本語文で、半角語（OS、Linux、UI、eiviz、wgpu など）の前後に半角スペースを入れない。気持ち悪さの原因になるため、例外なくガードする。

英語ドキュメント（`docs/src/content/docs/en/`）には適用しない。

## 必須手順

日本語 Markdown を書いた・直したら、コミット前に必ず次を実行する。違反が残っていたら直して再実行し、ゼロ件になるまで終えない。

```bash
python3 .cursor/skills/ja-docs-no-latin-spaces/scripts/check.py
```

特定ファイルだけ見る場合:

```bash
python3 .cursor/skills/ja-docs-no-latin-spaces/scripts/check.py docs/src/content/docs/ja/introduction/about.md
```

## 規則

和文と欧文（半角英数、インラインコード、Markdown の `**` / リンク閉じ）の境界に半角スペースを置かない。

欧文フレーズ内部の語間スペースは残す（`OBS Studio`、`Direct3D 11`、`GTK 4`、`C ABI`、`Apple Silicon`、`Windows on ARM`、`host-visible`）。

日本語文中の欧文列挙は `/` の前後スペースなし（`Windows/macOS`、`Swift 6/SwiftUI`）。

## 例

禁止:

```
Linux では Rust と GTK 4
各 OS の UI から
eiviz は有志
`cdylib` として
**非推奨** です
Windows / macOS より
```

必須:

```
LinuxではRustとGTK 4
各OSのUIから
eivizは有志
`cdylib`として
**非推奨**です
Windows/macOSより
```

## 対象外

- フェンスコード（```` ``` ````）
- Markdown 表の `|` セル区切り
- URL・リンク先
- 欧文だけの見出し（`### Windows: .NET 10 / C# 14 / WPF / D3D12`）
