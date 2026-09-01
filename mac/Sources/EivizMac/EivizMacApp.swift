import SwiftUI

@main
struct EivizMacApp: App {
    @StateObject private var mixer = MixerController()

    init() {
        HostLog.install()
    }

    var body: some Scene {
        WindowGroup("eiviz") {
            ContentView()
                .environmentObject(mixer)
                .frame(minWidth: 1280, minHeight: 720)
                .onAppear { mixer.boot() }
                .onDisappear { mixer.shutdown() }
        }
        .windowStyle(.titleBar)
        .defaultSize(width: 1680, height: 980)
    }
}
