# Release packaging

Phase 9 provides a packaging and signing foundation. It does **not** certify the
application, the optional native adapters, or any artifact built without the
release keys.

## Supported package profiles

- `windows-x86_64`: first-class Windows x64 MSIX, built with
  `wgpu-backend,audio-cpal,midi`.
- `macos-aarch64`: macOS arm64 `.app` inside a `.pkg`, minimum macOS 13, built
  with `wgpu-backend,audio-cpal,midi`.
- `linux-x86_64`: Ubuntu 24.04 x64 Debian package, built with
  `wgpu-backend,audio-cpal,midi`. This is the explicit Linux package; AppImage
  is not currently supported.

`packaging/release.json` is the machine-readable source of truth. Ordinary CI
uses Rust 1.97 and emits artifacts whose names contain `unsigned`. An unsigned
MSIX is a package construction test and is not installable under normal Windows
trust policy.

The profiles intentionally exclude `ndi`, `decklink`, and `audio-asio`.
OpenH264 and FDK AAC remain explicit runtime paths. Packaging rejects filenames
that look like NDI, DeckLink, ASIO, OpenH264, FDK AAC, or FFmpeg payloads. No
workflow downloads or silently bundles those SDKs/codecs. A separately reviewed
distribution may add an SDK only after its license, manifest, notices, tests,
and profile are changed explicitly.

## Reproducible unsigned inputs

Package builds use the pinned Cargo lockfile and Rust 1.97. CI derives
`SOURCE_DATE_EPOCH` from the source commit, remaps the checkout path to `/src`,
normalizes staged file timestamps, records the commit and feature list, and
writes SHA-256 sidecars. The Debian and MSIX package tools consume normalized
inputs. Apple `pkgbuild` is used for the unsigned macOS construction test.

Run a local Linux package build with:

```bash
export SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)"
python3 scripts/generate-third-party-notices.py
python3 scripts/generate-sbom.py
cargo build --locked --release --target x86_64-unknown-linux-gnu \
  -p eiviz-desktop --features wgpu-backend,audio-cpal,midi
cargo build --locked --release --target x86_64-unknown-linux-gnu \
  -p eiviz-project --bin eiviz-project-migrate
python3 scripts/build-package.py --target linux-x86_64 \
  --binary target/x86_64-unknown-linux-gnu/release/eiviz-desktop \
  --project-migrator target/x86_64-unknown-linux-gnu/release/eiviz-project-migrate
python3 scripts/validate-package.py --target linux-x86_64 \
  --artifact target/packages/eiviz_0.1.0_amd64_unsigned.deb
```

Byte-for-byte reproducibility is asserted only after comparing artifacts from
independent builders; this foundation does not make that certification claim.
Keyed signatures and trusted timestamps are intentionally non-reproducible.

Every package contains:

- the MIT `LICENSE`, curated native `NOTICE`, and generated Cargo dependency
  inventory;
- SPDX 2.3 and CycloneDX 1.5 SBOMs generated from the locked Cargo graph;
- a payload manifest containing file hashes, target, features, source commit,
  source epoch, an empty optional-SDK list, and truthful `unsigned-build-input`
  state (the signed wrapper has a separate post-signing release manifest);
- `eiviz-project-migrate`, which reads a project through the Rust migration and
  validation path and writes only to a distinct output path;
- `eiviz-data-migration.py`, the explicit backup/commit/rollback utility.

## Install upgrades and rollback

MSIX performs transactional application-package upgrades. macOS and Debian
replace application files through their native package managers. User projects
and configuration are not installer-owned and installers do not rewrite them.
Project schema migration remains in memory until the operator explicitly saves.

Before an upgrade, snapshot the selected user data directory outside that
directory:

```bash
python3 eiviz-data-migration.py prepare \
  --data-dir /path/to/eiviz-user-data \
  --state-dir /path/to/eiviz-upgrade-backups \
  --from-version 0.1.0 --to-version 0.2.0
```

The command prints a transaction identifier. After verification:

```bash
python3 eiviz-data-migration.py commit \
  --state-dir /path/to/eiviz-upgrade-backups --transaction TRANSACTION
```

To restore, the explicit confirmation token is mandatory:

```bash
python3 eiviz-data-migration.py rollback \
  --state-dir /path/to/eiviz-upgrade-backups --transaction TRANSACTION \
  --confirm RESTORE
```

Rollback verifies every backup hash and preserves the current data as
`pre-rollback-data` before replacement. Symlinks and nested data/backup roots
are rejected. Application package rollback uses the OS package manager; the
data script is deliberately separate.

To migrate one project without modifying the input:

```bash
eiviz-project-migrate old-project.json --output migrated-project.json
```

Unknown future schemas and legacy outputs that require invented codec profiles
remain hard errors.

## Signing and verification

`.github/workflows/release-sign.yml` is manual, requires the
`acknowledge_keyed_signing` input, and runs in the protected `release-signing`
environment. It fails before packaging when the selected target's secrets are
absent. It uploads signed evidence but does not publish a GitHub Release.

Required release environment secrets are:

- Windows: `WINDOWS_PFX_BASE64`, `WINDOWS_PFX_PASSWORD`, and the exact
  certificate-subject string `WINDOWS_PUBLISHER`. The workflow timestamps and
  verifies the MSIX with `signtool`.
- macOS: `MACOS_CERTIFICATES_P12_BASE64`,
  `MACOS_CERTIFICATES_PASSWORD`, `MACOS_APPLICATION_IDENTITY`,
  `MACOS_INSTALLER_IDENTITY`, `APPLE_ID`, `APPLE_APP_PASSWORD`, and
  `APPLE_TEAM_ID`. The workflow signs the hardened app and installer, submits
  to Apple notarization, staples the ticket, and assesses it.
- Linux: `LINUX_GPG_PRIVATE_KEY`, `LINUX_GPG_KEY_ID`, and, when applicable,
  `LINUX_GPG_PASSPHRASE`. Debian artifacts use a detached armored OpenPGP
  signature; the `.deb` format has no platform-equivalent embedded signature.

Signed runs generate an external `*.release.json` after signing. Verify on the
artifact's native platform:

```bash
python3 scripts/verify-signature.py --target windows-x86_64 \
  --artifact eiviz.msix --manifest windows-x86_64.release.json

python3 scripts/verify-signature.py --target macos-aarch64 \
  --artifact eiviz.pkg --manifest macos-aarch64.release.json \
  --assess-notarization

python3 scripts/verify-signature.py --target linux-x86_64 \
  --artifact eiviz.deb --signature eiviz.deb.asc \
  --manifest linux-x86_64.release.json
```

Verification uses `signtool`, `pkgutil`/`spctl`, or `gpg`. A green unsigned CI
package job never means that a release is signed, notarized, or trusted.
