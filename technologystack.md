# 技術スタック（固定版）

本ファイルが依存関係の source of truth です。版変更は ADR を先に更新します。

| コンポーネント | 版 | 備考 |
|---|---|---|
| Rust | 1.85.0 (MSRV) / CI 1.85 | edition 2024 |
| wgpu | 24.0.5 | GUI と CPU 合成の既定。gpu-video とは両立させない |
| egui / eframe | 0.32.3 | ADR-0005 |
| serde / serde_json | 1.0.219 / 1.0.143 | 永続化・制御面 |
| uuid | 1.18.1 | v7 IDs |
| image | 0.25.6 | PNG/JPEG 入力 |
| zip | 4.3.x | portable `.eiviz` |
| tiny_http | 0.12.0 | localhost HTTP |
| gpu-video | 0.4.0 | **optional feature only**, MIT, wgpu 29 |
| Smelter | 非同梱 | ADR-0002 |
| NDI / OMT / DeckLink / ASIO | feature + HIL | ADR-0007 |

認定プロファイル: 1920×1080p, `60000/1001` fps, SDR BT.709 8-bit, 48 kHz。
