---
title: Decklink
description: Decklink output
---

DeckLink input and output ship in the Pro module. A Free build returns `ERR_NOT_SUPPORTED_PLAN`. The host shows DeckLink only when `mixer_capabilities` reports that the module is linked and the plan allows it.

Official builds replace `pro/eiviz_pro` with the private module. The public tree keeps a Free stand-in at that path, and `mixer/Cargo.toml` does not change.
