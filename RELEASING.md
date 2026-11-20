# リリース手順

eiviz のリリースは、**非公開の Pro モジュールを先にリリースしてから**、公開リポジトリのタグで行います。順番を逆にすると、公開側の Release ワークフローが `release not found` で失敗します。

## 前提

- リポジトリは 2 つ。公開 `MikanseiLaboratory/eiviz` と非公開 `MikanseiLaboratory/eiviz-pro`。
- 公開リリースは、同じバージョンの非公開リリース `<公開タグ>-pro` から署名済みモジュールを取得します（`packaging/fetch-pro-module.sh`）。`v0.5.0-beta.1` は `v0.5.0-beta.1-pro` を読みます。
- 必要なシークレット:
  - 公開リポジトリ: `EIVIZ_PRO_MODULE_PUBLIC_KEY`（`module-keygen` の公開鍵、64 桁 hex）、`EIVIZ_PRO_REPO_TOKEN`（非公開リリースを読めるトークン）
  - 非公開リポジトリ: `EIVIZ_PRO_MODULE_SECRET`（モジュール署名の秘密鍵。チケット用の鍵とは別）
- タグの形式は `vX.Y.Z` または `vX.Y.Z-(alpha|beta|rc)[...]`。`alpha` / `beta` / `rc` はプレリリース扱いになり、`latest` は更新されません。
- プレリリースではコード側のバージョンを上げる必要はありません。公開タグのバージョンはワークフローが `-p:Version` / `-p:InformationalVersion` で差し込みます。基本バージョン（`0.5.0` など）を上げるときだけ、`hosts/win32/*.csproj` と `crates/*/Cargo.toml` を更新します（macOS の `Info*.plist` は `package-app.sh` が `EIVIZ_VERSION` で上書きするため更新不要です）。

## 全体の流れ

1. 公開 `main` のリリース対象コミットを決める（CI が green であること）
2. 非公開リポジトリの `EIVIZ_PUBLIC_SHA` をそのコミットへ更新して push
3. 非公開リポジトリで `Pro release` ワークフローを dispatch（`tag=<公開タグ>-pro`）
4. 非公開リリースの成果物（3 ターゲット分）を確認
5. 公開リポジトリに注釈付きタグを作成して push（`Release` ワークフローが起動）
6. 公開リリースの成果物と、インストーラに Pro モジュールが入っていることを確認

## 手順

### 1. リリース対象のコミットを決める

```bash
cd eiviz
git checkout main
git pull --ff-only
git rev-parse HEAD
gh run list --branch main --limit 3   # CI が success であることを確認
```

CI が失敗している場合は、まず原因を解消するか、失敗が環境要因（Windows の `asio-sys` など）であれば `gh run rerun <run-id> --failed` で再実行して green にしてから進みます。

### 2. 非公開リポジトリの固定コミットを更新

`EIVIZ_PUBLIC_SHA` の 2 行目を、手順 1 で決めた 40 文字のコミットに置き換えます。

```bash
cd ../eiviz-pro
git checkout main
git pull --ff-only
# EIVIZ_PUBLIC_SHA を編集（2 行目だけ）
git commit -am "v0.5.0-beta.1 の固定コミットを公開側 main へ進める"
git push origin main
```

### 3. Pro リリースを実行（公開リリースより先）

```bash
gh workflow run "Pro release" \
  --repo MikanseiLaboratory/eiviz-pro \
  -f public_sha=<40文字の公開コミット> \
  -f tag=<公開タグ>-pro
```

- `public_sha` は `EIVIZ_PUBLIC_SHA` の 2 行目と一致している必要があります。一致しないとジョブが即座に失敗します。
- ワークフローはモジュールをビルドし、`eiviz-pro-ticket sign-module` で署名し、マニフェストの SHA-256 とモジュールの実体が一致することを確認してから非公開リリースを作成します。

### 4. 非公開リリースを確認

```bash
gh release view <公開タグ>-pro --repo MikanseiLaboratory/eiviz-pro
```

次の成果物が 3 ターゲット分そろっていることを確認します（`<target>` は `x86_64-pc-windows-msvc` / `aarch64-apple-darwin` / `x86_64-apple-darwin`）。

- `<target>-eiviz_pro.dll`（macOS は `<target>-libeiviz_pro.dylib`）と `.manifest`
- `<target>-module-public-key.txt`
- `eiviz-pro.required.<target>`
- `release-manifest.<target>.json`

### 5. 公開リリース

変更点をタグメッセージに書きます（`git tag -l --format='%(contents)' <過去のタグ>` が例）。

```bash
cd ../eiviz
printf '%s\n' "eiviz v0.5.0-beta.1" "" "alpha.6 からの変更:" "- ..." > /tmp/tag.txt
git tag -a v0.5.0-beta.1 -F /tmp/tag.txt <40文字の公開コミット>
git push origin v0.5.0-beta.1
gh run list --workflow Release --limit 1
```

### 6. 公開リリースを確認

```bash
gh release view v0.5.0-beta.1 --json isPrerelease,assets
```

- `eiviz-v0.5.0-beta.1-win-x64-setup.exe` と `.sha256`
- `eiviz-v0.5.0-beta.1-macos-arm64.pkg` / `-macos-x64.pkg` と `.sha256`
- `isPrerelease` が期待どおり（`beta` は `true`、`latest` は更新されない）
- Discord 通知ジョブも同じワークフローの最後に走る

インストーラに Pro モジュールが入っていることも確認します。Windows のセットアップを展開（またはユーザー単位でインストール）して、次があること:

- `eiviz_pro.dll`
- `eiviz_pro.dll.manifest`
- `eiviz-pro.required`（中身が `eiviz_pro.dll`）

`eiviz-pro.required` があるのにモジュールが無い・署名が不正・ABI 不一致の場合、ホストは Community へフォールバックせず起動に失敗します。

## 失敗したときの後始末

```bash
# 公開タグとリリースを消す（出し直すとき）
gh release delete vX.Y.Z-beta.1 --yes --cleanup-tag   # リリースがある場合
git push origin :refs/tags/vX.Y.Z-beta.1              # リリースが無い場合
git tag -d vX.Y.Z-beta.1
```

- リリース実行中にタグを消しても、実行中のワークフローは失敗扱いになるだけです。`gh run cancel <run-id>` で止めてから消します。
- 非公開リリースを作り直すときは、`Pro release` をもう一度 dispatch します。softprops は既存リリースのアセットを上書きします。
- 公開リリースが `release not found` で失敗したら、ほぼ確実に `<公開タグ>-pro` が無いか、`public_sha` が古いです。手順 2〜4 からやり直します。

## チェックリスト

- [ ] 公開 `main` の CI が green（失敗時は再実行して確認）
- [ ] `EIVIZ_PUBLIC_SHA` をリリース対象コミットへ更新して push
- [ ] `Pro release` が成功し、`<公開タグ>-pro` に 3 ターゲット分の成果物
- [ ] 公開タグのメッセージに変更点を記載
- [ ] `Release` ワークフローが成功し、assets 3 種 + `.sha256` がある
- [ ] インストーラに `eiviz_pro.dll` / `.manifest` / `eiviz-pro.required` が入っている
- [ ] `isPrerelease` と `latest` の扱いが期待どおり
