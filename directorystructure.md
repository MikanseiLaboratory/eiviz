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
    eiviz-gpu/            # 明示 CpuReference compositor。wgpu-backend は別 backend
    eiviz-runtime/        # virtual clock, audio matrix
    eiviz-engine/         # 合成ルート
    eiviz-control/        # HTTP / TCP Command API（Stream Deck 固有ロジックなし）
    eiviz-io-*            # ファイル / NDI / OMT / DeckLink / audio / stream
    eiviz-codec-software/ # 明示software/external encoder contract
  docs/                   # 要件・ADR・認定
  technologystack.md
  directorystructure.md
```

依存は `desktop/control/io → engine → runtime/gpu → command/project → core/time/media` の一方向です。
