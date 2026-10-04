---
title: MU Bus
description: One audio bus per Mixing Unit, plus Headphone
---

The internal mix is 48 kHz stereo. Master and AUX are gone. Every [Mixing Unit](/eiviz/en/concepts/mixing-unit/) has one MU Bus. The bus id is the Mixing Unit id.

## MU Bus

An MU Bus is the mix for that Mixing Unit. Follow tracks Preview/Program and the T-bar. Independent ignores the T-bar and always mixes Program. The output device is chosen on the Mixing Unit. None keeps the mix internal.

An Input routes to a set of Mixing Units. An Input with no route is silent. One Input still cannot feed several hardware devices at once.

## Headphone

Click the headphone icon on a Mixing Unit or Input meter to listen to that source. Click it again to stop. The output device is set under Headphone in [Settings](/eiviz/en/introduction/settings/).

## Mix Input

Mix Input audio follows the referenced Mixing Unit's MU Bus. A session Multiview is silent.

Device detail is in [Audio, ASIO, and related](/eiviz/en/features/outputs/audio/).
