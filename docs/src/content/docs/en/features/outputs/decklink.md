---
title: Decklink
description: Decklink output
---

DeckLink I/O requires the Pro module. Under the Community plan, calls return `ERR_NOT_SUPPORTED_PLAN`. The host UI displays DeckLink options only when `mixer_capabilities` indicates the module is loaded and permitted by the active plan.

Official Pro packages include the signed Pro module and `eiviz-pro.required`. Public open-source builds do not include this module. Channel limits are detailed in [Plans](/eiviz/en/introduction/plans/).
