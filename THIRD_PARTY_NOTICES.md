# Third-party notices

eiviz original source is licensed under the PolyForm Shield License 1.0.0.
See `LICENSE` and `NOTICE`.

This file covers components that eiviz uses but does **not** relicense.
Those components stay under their own terms. Permissive licenses (MIT,
Apache-2.0, Zlib) allow use inside a more restrictive application as long
as copyright and permission notices are preserved.

## Direct Rust dependencies (`mixer/Cargo.toml`)

| Crate | License | Source |
| --- | --- | --- |
| bytemuck | MIT OR Apache-2.0 | https://crates.io/crates/bytemuck |
| image | MIT OR Apache-2.0 | https://crates.io/crates/image |
| openmediatransport | MIT | https://github.com/MikanseiLaboratory/openmediatransport-rs |
| pollster | Apache-2.0 OR MIT | https://crates.io/crates/pollster |
| raw-window-handle | MIT OR Apache-2.0 OR Zlib | https://crates.io/crates/raw-window-handle |
| thiserror | MIT OR Apache-2.0 | https://crates.io/crates/thiserror |
| vmx | MIT | https://github.com/MikanseiLaboratory/vmx-rs |
| wgpu | MIT OR Apache-2.0 | https://crates.io/crates/wgpu |
| windows | MIT OR Apache-2.0 | https://crates.io/crates/windows |

Transitive crates pulled by the above (for example naga, png, parking_lot)
are also MIT and/or Apache-2.0. Full versions are pinned in `mixer/Cargo.lock`.

Official Open Media Transport libraries that those git crates independently
reimplement (`libomtnet`, `libomt`, `libvmx`) are MIT as well. They are not
copied into this repository.

## Host (C# / WPF)

The host targets .NET on Windows and uses WPF. Those platform components
are distributed by Microsoft under their own terms (typically MIT for the
.NET SDK/runtime sources). This repository does not vendor the .NET runtime.

## ASIO trademark

eiviz talks to installed ASIO drivers through the public COM `IASIO`
interface. It does not redistribute Steinberg's ASIO SDK.

ASIO is a trademark of Steinberg Media Technologies GmbH. Using the ASIO
name or shipping an ASIO-enabled **commercial** product may require a
separate arrangement with Steinberg. That is independent of this
repository's PolyForm Shield terms.

## Binary releases

A binary build includes compiled third-party code. Keep this file, `LICENSE`,
and `NOTICE` next to the executable (or in the release archive) so MIT and
Apache-2.0 notice requirements stay satisfied.
