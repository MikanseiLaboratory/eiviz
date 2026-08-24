# ADR-0010 No implicit fallback

- Status: Accepted
- Date: 2026-08-24

## Context

計画と初期実装は「GPU が無いときは CPU」「欠落入力は灰色の偽カメラ」「ASIO が無いときは WASAPI」「Stream Deck 用の別プロトコル」といった暗黙経路を残していた。本番スイッチャーでは、要求した経路と実際の経路が一致しないことが事故になる。

## Decision

1. **Backend は明示選択。切替は Command / Project フィールドだけ。**
   - 合成は `Project.compositor`（`CpuReference` または `Wgpu`）。Runtime の backend と不一致ならエラー。
   - `Wgpu` で feature 未有効、hardware adapter なし、CPU-type adapter、blit 未実装のいずれでも **CPU `composite` を呼ばない**。
   - wgpu `force_fallback_adapter` は使わない。
   - Wgpu device loss は Engine を明示 `Degraded` にする。GUI owner が作成した
     replacement compositor は active snapshot の resource prewarm 成功後、次の
     frame boundary だけで再注入する。framework に device 再生成 API が無ければ
     restart required とし、CpuReference へ切り替えない。
2. **欠落 asset / live device は `Project.missing_media` だけが決める。**
   - `Slate` / `LastGood` / `Fail`。偽のカメラフィードや赤キャンバスへの暗黙置換は禁止。
   - `LastGood` で事前フレームが無いときは `Fail` と同じくエラー。Slate へ落とさない。
   - Desktop は asset の解決 path、期待 SHA-256、実 SHA-256、適用 policy を表示する。
     path 不在または hash 不一致を別名・同名ファイルで置換しない。
3. **Codec / audio device / I/O も同じ。**
   - software/external encode、WASAPI、ASIO は別の明示 profile / feature。選択した側が使えなければエラーまたは capability 不足表示であり、他方へ黙って乗り換えない。互換実装のないgpu-videoはcapabilityとして表示しない（ADR-0015）。
   - DeviceBinding は列挙済み hardware ID だけで解決する。表示名検索は行わない。
     再割当は versioned `UpdateDeviceBinding` command が old/new ID を一境界で更新する。
4. **Stream Deck ロジックは本リポジトリに置かない。**
   - プラグインは out-of-tree。eiviz は HTTP `/v1/command` と TCP JSON-lines の Command API だけを提供する。

## Consequences

CI の default `cargo test` は **CpuReference profile** を明示実行する。これは fallback ではない。`Wgpu` project を CPU Runtime で開くことはできない。Phase 7 の「encode＋fallback」は「明示 software-encode profile」と読み替える。
