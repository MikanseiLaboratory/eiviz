# 技術スタック（固定版）

本ファイルが依存関係の source of truth です。版変更は ADR を先に更新します。

| コンポーネント | 版 | 備考 |
|---|---|---|
| Rust | 1.85.0 MSRV / CI 1.98 + MSRV check | edition 2024。workspace全体を1.85で実測確認 |
| wgpu（eiviz-gpu直接依存） | 24.0.5 | 合成は `Project.compositor` で明示選択（ADR-0010） |
| egui / eframe | 0.32.3 | `egui-wgpu`はwgpu 25.0.2を推移依存。現在compositorとDevice共有不可（ADR-0011） |
| serde / serde_json | 1.0.219 / 1.0.143 | 永続化・制御面 |
| uuid | 1.18.1 | v7 IDs |
| image | 0.25.6 | PNG/JPEG 入力 |
| mp4io | 0.1.2 exact | H.264 MP4 sample index、Rust 1.85確認済み |
| zip | 4.3.x | portable `.eiviz` |
| tiny_http | 0.12.0 | localhost HTTP |
| gpu-video | 0.4.0 | **optional feature only**, MIT, wgpu 29 |
| Smelter | 非同梱 | ADR-0002 |
| OMT libomt C ABI | v1.0.0.16 header/binary profile | MIT、runtime load、HIL pending |
| NDI / DeckLink / ASIO | feature + HIL | ADR-0007 |

認定プロファイル: 1920×1080p, `60000/1001` fps, SDR BT.709 8-bit, 48 kHz。
