import SwiftUI

public enum EivizLaunch {
    public static func run(remote: Bool) {
        HostRole.isRemote = remote
        EivizMacApp.main()
    }
}

struct EivizMacApp: App {
    @StateObject private var mixer = MixerController()

    init() {
        HostLog.install()
        EivizTheme.applyAppAppearance()
    }

    var body: some Scene {
        WindowGroup(L10n.t(HostRole.isRemote ? "app.titleRemote" : "app.title")) {
            ContentView()
                .environmentObject(mixer)
                .environment(\.mixerSurfaceEpoch, mixer.surfaceEpoch)
                .frame(minWidth: 1280, minHeight: 720)
                .preferredColorScheme(EivizTheme.colorScheme)
                .onAppear {
                    EivizTheme.applyAppAppearance()
                    mixer.boot()
                    if let path = CommandLine.arguments.dropFirst().first(where: {
                        let lower = $0.lowercased()
                        return lower.hasSuffix(".eivz") || lower.hasSuffix(".eivzx")
                    }) {
                        mixer.openSessionFromSystem(path: path)
                    }
                }
                .onOpenURL { url in
                    mixer.openSessionFromSystem(path: url.path)
                }
                .onDisappear { mixer.shutdown() }
        }
        .windowStyle(.titleBar)
        .defaultSize(width: 1680, height: 980)
    }
}
