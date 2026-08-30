import SwiftUI

@main
struct EivizMacApp: App {
    @StateObject private var mixer = MixerSession()

    var body: some Scene {
        WindowGroup("eiviz") {
            ContentView()
                .environmentObject(mixer)
                .frame(minWidth: 1280, minHeight: 640)
                .onAppear { mixer.boot() }
                .onDisappear { mixer.shutdown() }
        }
        .windowStyle(.titleBar)
        .defaultSize(width: 1480, height: 720)
    }
}
