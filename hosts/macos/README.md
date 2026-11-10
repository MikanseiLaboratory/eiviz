# eiviz macOS host

Swift 6 / SwiftUI host for eiviz. The mixer is the Rust `eiviz_mixer` dylib; the
host talks to it through the internal C ABI in `Sources/EivizMixer/eiviz_mixer.h`.

Target: macOS 14 or later on Apple Silicon. Intel Macs build but are not a
supported or performance-tested target.

## Prerequisites

- Xcode with Swift 6 (the package uses `swift-tools-version: 6.0`)
- Rust 1.97 (`rustup toolchain install 1.97.0`)
- NDI 6 SDK, either installed at `/Library/NDI SDK for Apple` or pointed at with
  `NDI_SDK_DIR`
- `LIBCLANG_PATH` for `grafton-ndi`'s bindgen step. `build.sh` discovers the
  libclang that ships with Xcode, so an Xcode-only machine normally needs no
  extra setup.

## Build and run

```bash
cd hosts/macos
./build.sh
open .build/out/Products/Release/eiviz-mac.app
```

`build.sh` builds the mixer and remote dylibs, points their install names at
`@rpath`, builds the Swift package, and assembles `eiviz-mac.app` and
`eiviz-remote.app`. The built binaries are in `$(swift build -c release --show-bin-path)`.

For an explicit architecture:

```bash
EIVIZ_MAC_ARCH=aarch64-apple-darwin ./build.sh
EIVIZ_MAC_ARCH=x86_64-apple-darwin ./build.sh
```

## Tests

The mixer suite runs on the host:

```bash
cargo test -p eiviz_mixer --locked --release --features pro-test-key -- --test-threads=1
```

The Swift replay checks are an executable target from the replay work branch once
it is merged:

```bash
cd hosts/macos
swift run -c release replay-core-checks
```

`eiviz-headless` must start without the NDI setup action's `DYLD_LIBRARY_PATH`:

```bash
cargo build -p eiviz-headless --locked --release --bins
env -u DYLD_LIBRARY_PATH ./target/release/eiviz-headless validate \
  --session headless/tests/fixtures/bars.eivz
```

## Packaging

`package-app.sh` creates the app bundles. `packaging/macos/package-pkg.sh`
installs the apps into `/Applications` and `eivizctl` / `eiviz-headless` into
`/usr/local/bin`. The CLI needs `libndi.dylib` and, for Pro, `libeiviz_pro.dylib`
plus `eiviz-pro.required` beside it; the pkg script copies them when they are in
the staging directory.

The current release workflow signs ad-hoc. On a machine with a Developer ID,
replace `codesign --force --sign -` with the identity and add notarization.

## Troubleshooting

- `Library not loaded: @rpath/libndi.dylib`: build through `build.sh`, or add
  `-Wl,-rpath,@executable_path` and keep `libndi.dylib` next to the binary.
  `headless/build.rs` does this for `eiviz-headless`.
- Camera or microphone does not open: allow the app in System Settings →
  Privacy & Security. For a reset, use `tccutil reset Camera` and
  `tccutil reset Microphone`.
- NDI is not visible: allow Local Network access, and check Help → Logs for the
  NDI runtime error. The app bundle needs `libndi.dylib` in `Contents/MacOS`.
- Gatekeeper blocks an ad-hoc pkg: right-click the pkg and choose Open, or after
  installation run `xattr -dr com.apple.quarantine /Applications/eiviz.app`.
- Crash logs: `~/Library/Logs/DiagnosticReports` contains `.ips` files. Keep the
  dSYM for the app, mixer dylib, and Pro module to symbolicate them.
