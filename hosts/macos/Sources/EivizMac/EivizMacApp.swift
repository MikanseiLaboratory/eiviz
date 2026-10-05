import SwiftUI

enum AppChrome {
    static func editionName() -> String? {
        var caps = EivizCapabilities()
        guard mixer_capabilities(&caps) == 0 else { return nil }
        switch caps.plan {
        case 0: return "Free"
        case 1: return "Professional"
        case 2: return "Enterprise"
        default: return nil
        }
    }

    static func applyTitle() {
        let key = HostRole.isRemote ? "app.titleRemote" : "app.title"
        let title: String
        if let edition = editionName() {
            title = L10n.format(HostRole.isRemote ? "app.titleRemoteEdition" : "app.titleEdition", edition)
        } else {
            title = L10n.t(key)
        }
        DispatchQueue.main.async {
            NSApp.mainWindow?.title = title
        }
    }
}

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
                    AppChrome.applyTitle()
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
