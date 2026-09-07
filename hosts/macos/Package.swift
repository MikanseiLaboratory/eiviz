// swift-tools-version: 6.0
import PackageDescription

let mixerLib = Context.environment["EIVIZ_MIXER_LIBDIR"] ?? "../../target/release"

func exeLinker(_ plist: String) -> [LinkerSetting] {
    [
        .unsafeFlags([
            "-L\(mixerLib)",
            "-leiviz_mixer",
            "-leiviz_remote",
            "-Xlinker", "-rpath", "-Xlinker", "@executable_path",
            "-Xlinker", "-rpath", "-Xlinker", mixerLib,
            "-Xlinker", "-sectcreate",
            "-Xlinker", "__TEXT",
            "-Xlinker", "__info_plist",
            "-Xlinker", plist,
        ])
    ]
}

let package = Package(
    name: "EivizMac",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "eiviz-mac", targets: ["eiviz-mac"]),
        .executable(name: "eiviz-remote", targets: ["eiviz-remote"]),
    ],
    targets: [
        .target(
            name: "EivizMac",
            dependencies: ["EivizMixer", "EivizRemote"],
            path: "Sources/EivizMac"
        ),
        .executableTarget(
            name: "eiviz-mac",
            dependencies: ["EivizMac"],
            path: "Sources/eiviz-mac",
            linkerSettings: exeLinker("\(Context.packageDirectory)/Sources/EivizMac/Info.plist")
        ),
        .executableTarget(
            name: "eiviz-remote",
            dependencies: ["EivizMac"],
            path: "Sources/eiviz-remote",
            linkerSettings: exeLinker("\(Context.packageDirectory)/Sources/EivizMac/Info-Remote.plist")
        ),
        .systemLibrary(
            name: "EivizMixer",
            path: "Sources/EivizMixer"
        ),
        .systemLibrary(
            name: "EivizRemote",
            path: "Sources/EivizRemote"
        ),
    ]
)
