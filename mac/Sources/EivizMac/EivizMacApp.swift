import SwiftUI

@main
struct EivizMacApp: App {
    @StateObject private var mixer = MixerController()

    init() {
        HostLog.install()
        EivizTheme.applyAppAppearance()
    }

    var body: some Scene {
        WindowGroup("eiviz") {
            ContentView()
                .environmentObject(mixer)
                .environment(\.mixerSurfaceEpoch, mixer.surfaceEpoch)
                .frame(minWidth: 1280, minHeight: 720)
                .preferredColorScheme(EivizTheme.colorScheme)
                .onAppear {
                    EivizTheme.applyAppAppearance()
                    mixer.boot()
                }
                .onDisappear { mixer.shutdown() }
        }
        .windowStyle(.titleBar)
        .defaultSize(width: 1680, height: 980)
    }
}
