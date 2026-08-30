// swift-tools-version: 6.0
import PackageDescription

let mixerLib = Context.environment["EIVIZ_MIXER_LIBDIR"] ?? "../mixer/target/release"

let package = Package(
    name: "EivizMac",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "eiviz-mac", targets: ["EivizMac"]),
    ],
    targets: [
        .executableTarget(
            name: "EivizMac",
            dependencies: ["EivizMixer"],
            path: "Sources/EivizMac",
            linkerSettings: [
                .unsafeFlags([
                    "-L\(mixerLib)",
                    "-leiviz_mixer",
                    "-rpath", "@executable_path",
                    "-rpath", mixerLib,
                ])
            ]
        ),
        .systemLibrary(
            name: "EivizMixer",
            path: "Sources/EivizMixer"
        ),
    ]
)
