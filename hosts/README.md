# Hosts

Hosts are the executable entry points. They own windows, interaction, process
lifetime, and platform-specific I/O. Compose, audio, I/O, and session state stay
in the mixer under `crates/`.

| Directory | Stack | Platform | Status |
| --- | --- | --- | --- |
| `win32/` | C# 14 / .NET 10 / WPF, Direct3D 12 | Windows x64 (Arm in progress) | Supported |
| `macos/` | Swift 6 / SwiftUI, Metal | macOS 14+ on Apple Silicon (Intel builds) | Supported |
| `linux/` | Rust + GTK 4 + Vulkan | Linux | GUI not implemented |
| `headless/` | Rust daemon + `eivizctl` | Windows, macOS, Linux | Supported |

`headless/` is the GUI-less host. It links the `eiviz_mixer` rlib directly and
serves the Protobuf WebSocket control API. The GUI hosts load the mixer `cdylib`
through the internal C ABI instead. Linux currently ships headless only because
its GUI host is not implemented yet.
