---
title: Plans
description: Features and limits for Community, Professional, and Enterprise
---

eiviz offers three plans: Community, Professional, and Enterprise. Video compositing, NDI/OMT, audio processing, and remote control are available across all plans. Plans differ by Mixing Unit concurrency, DeckLink I/O channels, RTMP streaming resolution and frame rate limits, and capability flags for recording, SRT, and hardware encoding.

## Limits

Width, height, and frame rate are evaluated independently. Exceeding any of these limits results in rejection. Frame rate is compared as an exact ratio (`fps_num/fps_den`), and rates up to and including the cap are allowed.

| | Community | Professional | Enterprise |
| --- | --- | --- | --- |
| Mixing Units | 4 | Unlimited | Unlimited |
| DeckLink inputs | None | 4 | Unlimited |
| DeckLink outputs | None | 1 | Unlimited |
| RTMP width | 1280 | 1920 | Unlimited |
| RTMP height | 720 | 1080 | Unlimited |
| RTMP frame rate | 30 fps | 59.94 fps | Unlimited |
| Recording / SRT / hardware encode | None | Capability flag only (not implemented) | Capability flag only (not implemented) |

Professional 59.94 fps is represented as `60000/1001`. Integer 60 fps (`60/1`) exceeds the limit even when resolution is 1920×1080 or lower. Community 30 fps includes both `30/1` and `30000/1001`. Portrait video (1080×1920) is rejected on Professional because its height exceeds 1080.

Replacing an existing Mixing Unit with the same ID does not consume an additional slot.

## Plan Enforcement

A valid license ticket determines the active plan. The host UI manages ticket installation and displays status, while the Pro module validates and enforces entitlements.

Missing, expired, invalidly signed, or mismatched-machine tickets default to the Community plan. Expiration uses the later of the system clock and a persisted timestamp, preventing clock rollback bypasses. Existing outputs continue running when a ticket expires, but subsequent operations exceeding Community limits are rejected.

Public builds without a Pro module compile in Community limits only. Because DeckLink and RTMP implementations are not linked, even configurations within the Community cap return `ERR_IO`. Operations exceeding the active plan return `ERR_NOT_SUPPORTED_PLAN` prior to link verification. Official packages containing `eiviz-pro.required` fail startup if the module is missing, corrupted, or built for an incompatible ABI; they do not fall back to Community. See [System architecture](/eiviz/en/introduction/architecture/) for loading details.

Only development modules built with the `pro-dev` feature allow the `EIVIZ_PLAN` environment variable to take precedence over installed tickets (accepted values: `community`, `professional`, `pro`, `enterprise`). Release builds ignore this variable.

## Beta License

During the v0.x beta, signing in with a Google account issues a complimentary Professional license. It is valid through 2027-03-31 23:59 JST. Each account can activate up to 3 machines (reissuing for the same machine does not count toward this limit).

Beta tickets carry a `beta` flag and cannot be transferred to production releases.

The activation portal is available at [eiviz beta license](https://eiviz-license.ruk1ium.workers.dev), or via the License section in Settings.

1. In eiviz Settings, navigate to License and click **Open license site** (machine fingerprints are transferred automatically).
2. Sign in with your Google account. The generated ticket is returned and installed into the application automatically.
3. If navigating to the portal manually in a browser, paste your machine fingerprints, click **Issue**, and paste the generated ticket into the License field.

Once applied, the License page displays the associated account email address and expiration date.
