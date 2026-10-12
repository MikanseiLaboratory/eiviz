---
title: Plans
description: Features and limits for Community, Professional, and Enterprise
---

eiviz has three plans: Community, Professional, and Enterprise. Compositing, NDI/OMT, audio, and remote control are the same on every plan. The plan changes the Mixing Unit count, DeckLink, the RTMP picture size and frame rate, and the capability flags for recording, SRT, and hardware encode.

## Limits

Width, height, and frame rate are checked separately. Crossing any one of them is refused. Frame rate is compared as `fps_num/fps_den`, and a rate equal to the cap is allowed.

| | Community | Professional | Enterprise |
| --- | --- | --- | --- |
| Mixing Units | 4 | Unlimited | Unlimited |
| DeckLink inputs | None | 4 | Unlimited |
| DeckLink outputs | None | 1 | Unlimited |
| RTMP width | 1280 | 1920 | Unlimited |
| RTMP height | 720 | 1080 | Unlimited |
| RTMP frame rate | 30 fps | 59.94 fps | Unlimited |
| Recording / SRT / hardware encode | None | Capability flag only (not implemented) | Capability flag only (not implemented) |

Professional 59.94 fps is `60000/1001`. Integer 60 fps (`60/1`) is over the cap even at 1920×1080 or below. Community 30 fps includes `30/1` and `30000/1001`. A portrait 1080×1920 is refused on Professional because the height is over 1080.

Replacing a Mixing Unit that already has the same id does not consume another slot.

## How the plan is chosen

A valid license ticket selects the plan. The host installs the ticket and shows its status. The Pro module decides the plan.

An unregistered, expired, badly signed, or wrong-machine ticket is treated as Community. Expiry uses the later of the OS clock and a stored timestamp, so setting the clock back does not extend it. Outputs that are already running are not stopped when a ticket expires. A new call that exceeds the Community limits is refused.

A public build with no module compiles in the Community limits only. DeckLink and RTMP are not linked, so a picture inside the Community cap still returns `ERR_IO`. A call outside the plan returns `ERR_NOT_SUPPORTED_PLAN` before the link check. An official package that contains `eiviz-pro.required` does not start when the module is missing, badly signed, or built for a different ABI, and it does not fall back to Community. Loading is described in [System architecture](/eiviz/en/introduction/architecture/).

Only a development module built with `pro-dev` prefers the `EIVIZ_PLAN` environment variable over the ticket. The values are `community`, `professional` (`pro` is the same), and `enterprise`. A Release build without that feature ignores the variable.
