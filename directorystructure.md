# ディレクトリ構成

```
eiviz/
  apps/eiviz-desktop/     # egui。Command 発行と購読のみ
  crates/
    eiviz-time/           # 有理数時刻
    eiviz-core/           # ドメイン。SDK/GPU 禁止
    eiviz-command/        # sequencer / reducer
    eiviz-project/        # schema, ZIP, atomic save
    eiviz-media/          # frame/audio 契約
    eiviz-gpu/            # CPU compositor（wgpu feature 可）
    eiviz-runtime/        # virtual clock, audio matrix
    eiviz-engine/         # 合成ルート
    eiviz-control/        # HTTP / TCP
    eiviz-io-*            # ファイル / NDI / OMT / DeckLink / audio / stream
    eiviz-codec-*         # software + gpu-video 隔離
  docs/                   # 要件・ADR・認定
  technologystack.md
  directorystructure.md
```

依存は `desktop/control/io → engine → runtime/gpu → command/project → core/time/media` の一方向です。
