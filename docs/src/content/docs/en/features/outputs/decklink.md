---
title: Decklink
description: Decklink output
---

DeckLink I/O requires the Pro module. Under the Community plan, calls return `ERR_NOT_SUPPORTED_PLAN`. The host UI displays DeckLink options only when `mixer_capabilities` indicates the module is loaded and permitted by the active plan.

macOS is not a supported DeckLink host yet. The macOS Pro build omits the DeckLink shim because the current Blackmagic macOS headers do not provide the `IID_IUnknown` declaration expected by `decklink-sys`; use Windows for DeckLink I/O. The macOS package still includes the signed Pro module for RTMP, Replay, and licensing.

Official Pro packages include the signed Pro module and `eiviz-pro.required`. Public open-source builds do not include this module. Channel limits are detailed in [Plans](/eiviz/en/introduction/plans/).
