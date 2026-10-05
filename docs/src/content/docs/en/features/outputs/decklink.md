---
title: Decklink
description: Decklink output
---

DeckLink input and output ship in the Pro module. A Free build returns `ERR_NOT_SUPPORTED_PLAN`. The host shows DeckLink only when `mixer_capabilities` reports that the module is linked and the plan allows it.

An official Pro package ships the signed module and `eiviz-pro.required`. Public builds do not include that module.
